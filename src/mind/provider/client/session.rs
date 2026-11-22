use super::Call;
use crate::mind::provider::{
    api::Options,
    compat::{self, SessionAffinityFormat},
    identity,
    model::{Api, Model},
    routing,
};
use reqwest::header::{HeaderMap, HeaderValue};

pub(super) fn validate(options: &Options) -> Result<(), &'static str> {
    if options
        .session_id
        .as_ref()
        .is_some_and(|id| id.is_empty() || id.len() > 256 || id.chars().any(char::is_control))
    {
        return Err("session_id must contain 1 to 256 UTF-8 bytes without control characters");
    }
    Ok(())
}

pub(super) fn bound(
    call: &Call<'_>,
    options: &Options,
    credential: &str,
    endpoint: &str,
) -> Option<String> {
    let compat = compat::resolve(call.model.compat);
    let enabled = match call.model.api {
        Api::OpenAiCompletions | Api::MistralConversations => {
            compat.send_session_affinity_headers || compat.supports_prompt_cache_key
        }
        Api::AnthropicMessages => compat.send_session_affinity_headers,
        _ => false,
    };
    if !enabled {
        return None;
    }
    let id = options.session_id.as_deref()?;
    Some(identity::digest(
        "provider-session-v1",
        &[
            id.as_bytes(),
            call.provider.id.as_bytes(),
            call.model.api.as_str().as_bytes(),
            call.model.id.as_bytes(),
            endpoint.as_bytes(),
            credential.as_bytes(),
        ],
    ))
}

pub(super) fn headers(
    model: &Model,
    session: Option<&str>,
    headers: &mut HeaderMap,
) -> Result<(), &'static str> {
    let compat = compat::resolve(model.compat);
    let Some(session) = session.filter(|_| {
        matches!(
            model.api,
            Api::OpenAiCompletions | Api::AnthropicMessages | Api::MistralConversations
        ) && compat.send_session_affinity_headers
    }) else {
        return Ok(());
    };
    let names: &[&'static str] = match (model.api, compat.session_affinity_format) {
        (Api::MistralConversations, _) => &["x-affinity"],
        (_, SessionAffinityFormat::OpenRouter) => &["x-session-id"],
        (Api::AnthropicMessages, _) => &["x-session-affinity"],
        (_, SessionAffinityFormat::Generic) => &["x-client-request-id", "x-session-affinity"],
        (_, SessionAffinityFormat::OpenAi) => {
            &["session_id", "x-client-request-id", "x-session-affinity"]
        }
    };
    let mut value = HeaderValue::from_str(session).map_err(|_| "invalid session identity")?;
    value.set_sensitive(true);
    for name in names {
        if model.api == Api::MistralConversations && headers.contains_key(*name) {
            continue;
        }
        headers.insert(*name, value.clone());
    }
    routing::check_headers(headers)
}

#[cfg(test)]
mod tests;
