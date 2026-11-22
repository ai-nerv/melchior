use super::{Account, config, destination, endpoint, refused};
use crate::mind::provider::{Http, Request, control, endpoint::Provider, oauth};
use reqwest::header::{HeaderName, HeaderValue};
use serde_json::Value;
use std::time::Duration;

pub(super) const HEADERS: [(&str, &str); 4] = [
    ("user-agent", "GitHubCopilotChat/0.35.0"),
    ("editor-version", "vscode/1.107.0"),
    ("editor-plugin-version", "copilot-chat/0.35.0"),
    ("copilot-integration-id", "vscode-chat"),
];

pub(super) fn headers(mut request: Request) -> Request {
    for (name, value) in HEADERS {
        request = request.header(
            HeaderName::from_static(name),
            HeaderValue::from_static(value),
        );
    }
    request.header(
        reqwest::header::ACCEPT,
        HeaderValue::from_static("application/json"),
    )
}

pub(super) struct Response {
    pub status: reqwest::StatusCode,
    pub retry_after: Option<u64>,
    pub value: Value,
}

pub(super) async fn response(request: Request) -> Result<Response, oauth::Error> {
    let limits = control::Limits {
        bytes: 1024 * 1024,
        timeout: Duration::from_secs(15),
    };
    let deadline = tokio::time::Instant::now() + limits.timeout;
    tokio::time::timeout_at(deadline, async {
        let reply = request
            .send()
            .await
            .map_err(|_| refused("Copilot authentication request failed"))?;
        let status = reply.status();
        let retry_after = reply
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        let body = control::read(reply, limits, deadline)
            .await
            .map_err(|_| refused("Invalid or oversized Copilot response"))?;
        let value = if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            Value::Null
        } else {
            serde_json::from_str(&body).map_err(|_| refused("Invalid Copilot response"))?
        };
        Ok(Response {
            status,
            retry_after,
            value,
        })
    })
    .await
    .map_err(|_| refused("Copilot authentication request timed out"))?
}

pub(super) async fn json(request: Request) -> Result<Value, oauth::Error> {
    let reply = response(request).await?;
    if !reply.status.is_success() {
        return Err(refused(
            "Copilot refused authentication; check subscription and organization access",
        ));
    }
    Ok(reply.value)
}

pub(super) fn secret(value: &Value, field: &str) -> Result<String, oauth::Error> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty() && v.len() <= 16384 && v.bytes().all(|b| b.is_ascii_graphic()))
        .map(str::to_owned)
        .ok_or_else(|| refused("Invalid Copilot credential response"))
}

pub(super) async fn exchange(
    http: &Http,
    provider: &Provider,
    github: &str,
) -> Result<(oauth::Tokens, Account), oauth::Error> {
    let conf = config(provider)?;
    let value = json(headers(http.get(&conf.token_url).bearer_auth(github))).await?;
    let access = secret(&value, "token")?;
    let expires_at = value
        .get("expires_at")
        .and_then(Value::as_u64)
        .filter(|expiry| *expiry > oauth::now().saturating_add(60))
        .ok_or_else(|| refused("Copilot returned an expired token"))?;
    let base_url = endpoint(provider, &access)?;
    let url = destination(provider, &format!("{base_url}/models"))?;
    let models = json(headers(http.get(url).bearer_auth(&access)).header(
        HeaderName::from_static("x-github-api-version"),
        HeaderValue::from_static("2026-06-01"),
    ))
    .await?;
    let rows = models
        .get("data")
        .and_then(Value::as_array)
        .filter(|rows| rows.len() <= 4096)
        .ok_or_else(|| refused("Invalid Copilot model list"))?;
    let eligible =
        |row: &Value| row.pointer("/capabilities/supports/tool_calls") != Some(&Value::Bool(false));
    let selectable = |row: &Value| {
        row.get("model_picker_enabled") == Some(&Value::Bool(true))
            && matches!(
                row.pointer("/policy/state").and_then(Value::as_str),
                None | Some("enabled")
            )
    };
    let picker = rows.iter().any(|row| eligible(row) && selectable(row));
    let fallback = !picker && base_url == "https://api.individual.githubcopilot.com";
    let models = rows
        .iter()
        .filter(|row| {
            eligible(row)
                && (selectable(row)
                    || (fallback
                        && row.pointer("/policy/state").and_then(Value::as_str) == Some("enabled")))
        })
        .filter_map(|row| row.get("id").and_then(Value::as_str))
        .filter(|id| !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control))
        .map(str::to_owned)
        .collect();
    Ok((
        oauth::Tokens {
            access,
            refresh: Some(github.into()),
            expires_at,
            binding: None,
        },
        Account { base_url, models },
    ))
}
