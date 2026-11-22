//! Provider authentication and the interactive command-line entry points.

mod browser;
pub mod manage;

use crate::mind::provider::endpoint::{Auth, Provider};
use crate::mind::provider::oauth::{self, Store};
use anyhow::{Context, Result, bail};

pub async fn login(id: &str) -> Result<()> {
    let provider = find(id)?;
    if matches!(provider.auth, Auth::GitHubCopilot { .. }) {
        crate::mind::provider::copilot::login(&provider, |device| {
            println!(
                "Open {} and enter code {} (expires in {} seconds).",
                device.url, device.code, device.expires_in
            );
            Ok(())
        })
        .await?;
        println!("Signed in to {}.", provider.name);
        return Ok(());
    }
    browser::login(&provider, |url| {
        println!("Sign in to {} in your browser:\n\n  {url}\n", provider.name);
        let mut command = std::process::Command::new("xdg-open");
        crate::inherited::scrub(&mut command);
        if let Ok(mut child) = command
            .arg(url)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Ok(())
    })
    .await?;
    println!("Signed in to {}.", provider.name);
    Ok(())
}

pub fn logout(id: &str) -> Result<()> {
    remove(id)?;
    println!("Removed saved credentials for {id}. Environment keys are unchanged.");
    Ok(())
}

fn remove(id: &str) -> Result<()> {
    if id.is_empty() || id.len() > 256 {
        bail!("invalid provider identity");
    }
    let _held = oauth::hold(id)?;
    Store::amend(|store| {
        store.forget(id);
        store.api_keys.remove(id);
        store.attempts.remove(id);
        store.configurations.remove(id);
        store.copilot.remove(id);
    })?;
    Ok(())
}

fn save_key(provider: &Provider, key: String) -> Result<()> {
    if !matches!(&provider.auth, Auth::ApiKey { vars } if !vars.is_empty()) {
        bail!("this provider does not accept saved API keys");
    }
    if key.is_empty() || key.len() > 8192 || !key.bytes().all(|b| b.is_ascii_graphic()) {
        bail!("API keys must contain 1–8192 printable ASCII characters without spaces");
    }
    let _held = oauth::hold(&provider.id)?;
    Store::amend(|store| {
        store.api_keys.insert(
            provider.id.clone(),
            oauth::SavedKey {
                value: key,
                configuration: provider.credential_scope(),
            },
        );
    })?;
    Ok(())
}

pub fn status() -> Result<()> {
    for row in manage::list()? {
        println!("{:<24} {:<12} {}", row.id, row.method, row.status);
    }
    Ok(())
}

fn find(id: &str) -> Result<Provider> {
    let catalog = crate::mind::catalog::Catalog::load(&crate::mind::catalog::Catalog::dir())?;
    catalog
        .providers
        .into_iter()
        .find(|p| p.id == id)
        .context("unknown provider")
}
