//! Bounded authentication requests on stdin and status events on stdout.

use super::{Auth, Store};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Read, Write};

const LIMIT: usize = 32 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    version: u8,
    #[serde(rename = "event")]
    op: String,
    #[serde(default)]
    provider: String,
    #[serde(default)]
    key: String,
}

#[derive(Serialize)]
pub(super) struct Account {
    pub id: String,
    pub name: String,
    pub method: String,
    pub status: String,
    source: String,
    saved: bool,
    available: bool,
    variables: Vec<String>,
}

pub(super) fn list() -> anyhow::Result<Vec<Account>> {
    let catalog = crate::mind::catalog::Catalog::load(&crate::mind::catalog::Catalog::dir())?;
    let store = Store::load();
    let mut rows = Vec::new();
    for provider in &catalog.providers {
        let (method, available) = match &provider.auth {
            Auth::GitHubCopilot { config } => ("oauth-device", config.valid()),
            Auth::ApiKey { vars } => ("api-key", !vars.is_empty()),
            Auth::OAuth { .. } => ("oauth", super::browser::configured(provider)),
            Auth::None => continue,
            Auth::AwsSigV4 => ("aws-sig-v4", false),
            Auth::GoogleAdc => ("google-adc", false),
        };
        let saved = store
            .as_ref()
            .is_ok_and(|s| s.api_keys.contains_key(&provider.id) || s.get(&provider.id).is_some());
        let env = provider.auth.resolve().is_some();
        let (source, status) = if env {
            ("environment", "Environment key")
        } else {
            match &store {
                Err(_) => ("none", "Credential store unreadable"),
                Ok(s)
                    if matches!(provider.auth, Auth::ApiKey { .. })
                        && provider.saved_key(s).is_some() =>
                {
                    ("saved", "Saved API key")
                }
                Ok(s)
                    if matches!(provider.auth, Auth::OAuth { .. })
                        && provider.saved_tokens(s).is_some_and(|t| {
                            provider
                                .auth
                                .tokens_ready(t, crate::mind::provider::oauth::now())
                        }) =>
                {
                    ("saved", "Signed in")
                }
                Ok(s)
                    if matches!(provider.auth, Auth::GitHubCopilot { .. })
                        && crate::mind::provider::copilot::signed_in(s, provider) =>
                {
                    (
                        "saved",
                        if provider.models.is_empty() {
                            "Signed in; no enabled supported models"
                        } else {
                            "Signed in"
                        },
                    )
                }
                Ok(_) if saved => ("saved", "Expired or configuration changed"),
                Ok(_) if !available => ("none", "Authentication not configured"),
                Ok(_) => ("none", "Not signed in"),
            }
        };
        rows.push(Account {
            id: provider.id.clone(),
            name: provider.name.clone(),
            method: method.into(),
            status: status.into(),
            source: source.into(),
            saved,
            available,
            variables: provider.auth.vars().to_vec(),
        });
    }
    if let Ok(store) = &store {
        let ids: std::collections::BTreeSet<_> = store
            .providers
            .keys()
            .chain(store.api_keys.keys())
            .collect();
        for id in ids {
            if rows.iter().any(|row| &row.id == id) {
                continue;
            }
            rows.push(Account {
                id: id.clone(),
                name: id.clone(),
                method: if store.api_keys.contains_key(id) {
                    "api-key"
                } else {
                    "oauth"
                }
                .into(),
                status: "Provider no longer configured".into(),
                source: "saved".into(),
                saved: true,
                available: false,
                variables: Vec::new(),
            });
        }
    }
    rows.sort_by(|a, b| {
        (a.id != "mistral", &a.name, &a.id).cmp(&(b.id != "mistral", &b.name, &b.id))
    });
    Ok(rows)
}

fn emit(value: &serde_json::Value) -> std::io::Result<()> {
    let mut stdout = std::io::stdout().lock();
    serde_json::to_writer(&mut stdout, value)?;
    stdout.write_all(b"\n")?;
    stdout.flush()
}

pub fn stdio() -> std::io::Result<()> {
    let mut bytes = Vec::new();
    let read = std::io::stdin()
        .lock()
        .take((LIMIT + 1) as u64)
        .read_until(b'\n', &mut bytes);
    let parsed = if read.is_ok() && bytes.len() <= LIMIT {
        serde_json::from_slice::<Request>(&bytes).ok()
    } else {
        None
    };
    let Some(request) =
        parsed.filter(|r| r.version == 1 && r.provider.len() <= 256 && r.key.len() <= 8192)
    else {
        return emit(
            &serde_json::json!({"event":"finished","error":"Invalid authentication request","providers":[]}),
        );
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    if request.op == "verbs" {
        return emit(
            &serde_json::json!({"event":"verbs","version":1,"verbs":["list","set_key","login","remove","verbs"]}),
        );
    }
    let result: Result<(), &str> = runtime.block_on(async {
        match request.op.as_str() {
            "list" => Ok(()),
            "set_key" => {
                let provider = super::find(&request.provider).map_err(|_| "Unknown or invalid provider configuration")?;
                super::save_key(&provider, request.key).map_err(|_| "Could not save API key; check the key and private credential-store permissions")
            }
            "remove" => super::remove(&request.provider).map_err(|_| "Could not remove saved credentials"),
            "login" => {
                let provider = super::find(&request.provider).map_err(|_| "Unknown or invalid provider configuration")?;
                if matches!(provider.auth, Auth::GitHubCopilot { .. }) {
                    return crate::mind::provider::copilot::login(&provider, |device| {
                        emit(&serde_json::json!({"event":"device","url":device.url,"code":device.code,"expires_in":device.expires_in}))?;
                        Ok(())
                    }).await.map_err(|_| "Copilot sign-in failed, was denied or expired; check subscription and organization access, then retry");
                }
                super::browser::login(&provider, |url| {
                    emit(&serde_json::json!({"event":"browser","url":url}))?;
                    Ok(())
                }).await.map_err(|_| "Browser sign-in failed or expired; existing credentials were kept")
            }
            _ => Err("Unsupported authentication operation"),
        }
    });
    let rows = list();
    let error = result.err().or_else(|| {
        rows.is_err()
            .then_some("Could not list provider authentication")
    });
    emit(
        &serde_json::json!({"event":"finished","error":error,"providers":rows.unwrap_or_default()}),
    )
}
