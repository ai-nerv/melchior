use super::{Device, config, network, refused};
use crate::mind::provider::{Http, endpoint::Provider, oauth};
use serde_json::Value;
use std::time::Duration;

pub(crate) async fn login(
    provider: &Provider,
    ready: impl FnOnce(Device) -> Result<(), oauth::Error>,
) -> Result<(), oauth::Error> {
    let conf = config(provider)?;
    let http = super::super::http()
        .build()
        .map_err(|_| refused("Could not start Copilot authentication"))?;
    let attempt = oauth::Pkce::generate()?.verifier;
    {
        let _held = oauth::hold(&provider.id)?;
        oauth::Store::amend(|store| {
            store.attempts.insert(provider.id.clone(), attempt.clone());
        })?;
    }
    let value = network::json(network::headers(http.post(&conf.device_url).form(&[
        ("client_id", conf.client_id.as_str()),
        ("scope", "read:user"),
    ])))
    .await?;
    let device = network::secret(&value, "device_code")?;
    let code = network::secret(&value, "user_code")?;
    if code.len() > 32 {
        return Err(refused("Invalid GitHub user code"));
    }
    let url = value
        .get("verification_uri")
        .and_then(Value::as_str)
        .filter(|url| *url == conf.verification_url)
        .ok_or_else(|| refused("Untrusted GitHub verification URL"))?
        .to_owned();
    let expires = value
        .get("expires_in")
        .and_then(Value::as_u64)
        .filter(|v| *v > 0)
        .ok_or_else(|| refused("Invalid GitHub device expiry"))?
        .min(900);
    let interval = value.get("interval").and_then(Value::as_u64).unwrap_or(5);
    if interval == 0 {
        return Err(refused("Invalid GitHub polling interval"));
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(expires);
    ready(Device {
        url,
        code,
        expires_in: expires,
    })?;
    let github = tokio::time::timeout_at(deadline, poll(&http, conf, &device, interval))
        .await
        .map_err(|_| refused("GitHub device code expired; start sign-in again"))??;
    let (mut tokens, account) = network::exchange(&http, provider, &github).await?;
    tokens.binding = Some(super::super::identity::digest(
        "copilot-login-v1",
        &[attempt.as_bytes()],
    ));
    let _held = oauth::hold(&provider.id)?;
    let mut saved = false;
    oauth::Store::amend(|store| {
        if store.attempts.get(&provider.id) == Some(&attempt) {
            store.put(&provider.id, tokens);
            store.copilot.insert(provider.id.clone(), account);
            store
                .configurations
                .insert(provider.id.clone(), provider.credential_scope());
            store.attempts.remove(&provider.id);
            saved = true;
        }
    })?;
    if !saved {
        return Err(refused("Copilot sign-in was cancelled or superseded"));
    }
    Ok(())
}

async fn poll(
    http: &Http,
    conf: &super::Config,
    device: &str,
    mut interval: u64,
) -> Result<String, oauth::Error> {
    loop {
        tokio::time::sleep(Duration::from_secs(interval.min(901))).await;
        let response = network::response(network::headers(http.post(&conf.oauth_url).form(&[
            ("client_id", conf.client_id.as_str()),
            ("device_code", device),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ])))
        .await?;
        if response.status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            interval = interval
                .saturating_add(5)
                .max(response.retry_after.unwrap_or(0));
            continue;
        }
        if response.status.is_success() && response.value.get("access_token").is_some() {
            if response.value.get("refresh_token").is_some() {
                return Err(refused(
                    "Expiring GitHub OAuth credentials are not supported by this client registration",
                ));
            }
            return network::secret(&response.value, "access_token");
        }
        match response.value.get("error").and_then(Value::as_str) {
            Some("authorization_pending") => {}
            Some("slow_down") => {
                interval = interval.saturating_add(5).max(
                    response
                        .value
                        .get("interval")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                )
            }
            Some("access_denied") => return Err(refused("GitHub sign-in was denied")),
            Some("expired_token") => return Err(refused("GitHub device code expired")),
            _ => return Err(refused("GitHub device authorization failed")),
        }
    }
}
