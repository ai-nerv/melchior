//! GitHub device authorization and subscription-bound Copilot credentials.

mod device;
mod network;
mod runtime;

#[cfg(test)]
mod tests;

pub(crate) use device::login;
pub(crate) use runtime::{ResolveError, apply_catalog, resolve};

pub(crate) fn signed_in(store: &oauth::Store, provider: &Provider) -> bool {
    runtime::held(store, provider)
        .is_ok_and(|(tokens, _)| provider.auth.tokens_ready(&tokens, oauth::now()))
}

use super::{endpoint::Provider, oauth};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub device_url: String,
    pub oauth_url: String,
    pub token_url: String,
    pub verification_url: String,
    pub client_id: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            device_url: "https://github.com/login/device/code".into(),
            oauth_url: "https://github.com/login/oauth/access_token".into(),
            token_url: "https://api.github.com/copilot_internal/v2/token".into(),
            verification_url: "https://github.com/login/device".into(),
            client_id: "Iv1.b507a08c87ecfe98".into(),
        }
    }
}

impl Config {
    pub(crate) fn valid(&self) -> bool {
        !self.client_id.is_empty()
            && self.client_id.len() <= 256
            && [
                &self.device_url,
                &self.oauth_url,
                &self.token_url,
                &self.verification_url,
            ]
            .into_iter()
            .all(|url| safe_url(url))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub base_url: String,
    pub models: BTreeSet<String>,
}

pub(crate) struct Device {
    pub url: String,
    pub code: String,
    pub expires_in: u64,
}

pub(crate) fn config(provider: &Provider) -> Result<&Config, oauth::Error> {
    match &provider.auth {
        super::endpoint::Auth::GitHubCopilot { config } if config.valid() => Ok(config),
        _ => Err(refused("Invalid Copilot authentication configuration")),
    }
}

fn refused(message: &str) -> oauth::Error {
    oauth::Error::Refused(message.into())
}

fn safe_url(raw: &str) -> bool {
    raw.len() <= 8192
        && super::usable_url(raw)
        && reqwest::Url::parse(raw).is_ok_and(|url| {
            url.username().is_empty() && url.password().is_none() && url.fragment().is_none()
        })
}

pub(crate) fn endpoint(provider: &Provider, token: &str) -> Result<String, oauth::Error> {
    let proxy = token
        .split(';')
        .find_map(|part| part.strip_prefix("proxy-ep="));
    let raw = match proxy {
        Some(host) => format!(
            "https://{}",
            host.strip_prefix("proxy.")
                .map_or_else(|| host.to_owned(), |tail| format!("api.{tail}"))
        ),
        None => provider
            .base_url
            .clone()
            .ok_or_else(|| refused("Copilot endpoint missing"))?,
    };
    if destination(provider, &raw)?.path() != "/" {
        return Err(refused("Invalid Copilot endpoint path"));
    }
    Ok(raw.trim_end_matches('/').into())
}

pub(crate) fn destination(provider: &Provider, raw: &str) -> Result<reqwest::Url, oauth::Error> {
    let url = reqwest::Url::parse(raw).map_err(|_| refused("Invalid Copilot endpoint"))?;
    if !safe_url(raw) || url.query().is_some() {
        return Err(refused("Invalid Copilot endpoint"));
    }
    let trusted = url.scheme() == "https"
        && url.port().is_none()
        && url.host_str().is_some_and(|host| {
            matches!(
                host,
                "api.individual.githubcopilot.com"
                    | "api.business.githubcopilot.com"
                    | "api.enterprise.githubcopilot.com"
            )
        });
    if !trusted && super::routing::provider_destination(provider, raw).is_err() {
        return Err(refused("Untrusted Copilot endpoint"));
    }
    Ok(url)
}

pub(crate) fn headers(
    context: &crate::mind::model::Context,
    headers: &mut reqwest::header::HeaderMap,
    key: &str,
) -> Result<(), oauth::Error> {
    use reqwest::header::HeaderValue;
    headers.remove("x-api-key");
    let mut bearer = HeaderValue::from_str(&format!("Bearer {key}"))
        .map_err(|_| refused("Invalid Copilot credential"))?;
    bearer.set_sensitive(true);
    headers.insert("authorization", bearer);
    for (name, value) in network::HEADERS {
        headers.insert(name, HeaderValue::from_static(value));
    }
    headers.insert(
        "openai-intent",
        HeaderValue::from_static("conversation-edits"),
    );
    use crate::mind::model::{Content, Role};
    let agent = context
        .messages
        .last()
        .is_some_and(|message| message.role != Role::User);
    headers.insert(
        "x-initiator",
        HeaderValue::from_static(if agent { "agent" } else { "user" }),
    );
    let images = context.messages.iter().any(|message| {
        matches!(message.role, Role::User | Role::Tool)
            && message.content.iter().any(|content| match content {
                Content::Image { .. } => true,
                Content::ToolResult { images, .. } => !images.is_empty(),
                _ => false,
            })
    });
    if images {
        headers.insert("copilot-vision-request", HeaderValue::from_static("true"));
    }
    if headers.contains_key("anthropic-version") {
        headers.insert(
            "anthropic-dangerous-direct-browser-access",
            HeaderValue::from_static("true"),
        );
    }
    super::routing::check_headers(headers).map_err(refused)
}
