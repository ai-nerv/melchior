//! Streaming a turn from a provider: the only part of this crate that does I/O. What it knows of
//! a protocol comes from an [`Adapter`], and what it knows of a vendor from the catalog.

use crate::mind::provider::api::{Adapter, Delta, Options};
use crate::mind::provider::endpoint::{Auth, Provider};
use crate::mind::provider::model::Model;
use crate::mind::provider::retry::RetryClass;

/// How many times a request is made before the failure is the answer.
const MAX_ATTEMPTS: u32 = 4;

/// Everything one request is made of.
pub struct Call<'a> {
    pub adapter: &'a dyn Adapter,
    pub provider: &'a Provider,
    pub model: &'a Model,
    pub context: &'a Context,
    pub options: &'a Options,
}

/// A failure that is being waited out rather than reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Retrying {
    /// Which try just failed, counting from one.
    pub attempt: u32,
    pub max_attempts: u32,
    pub delay: std::time::Duration,
    pub why: String,
}
use crate::mind::model::Context;
use crate::mind::provider::sse;
use futures_util::StreamExt;

/// Why a turn could not be streamed.
#[derive(Debug, thiserror::Error)]
pub struct ProviderError {
    pub class: RetryClass,
    pub message: String,
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl ProviderError {
    fn new(class: RetryClass, message: impl Into<String>) -> Self {
        Self {
            class,
            message: message.into(),
        }
    }
}

/// Streams turns from providers.
pub struct Client {
    http: reqwest::Client,
    /// The first backoff delay; each later one grows from it.
    base_delay: std::time::Duration,
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

impl Client {
    /// The same, waiting `base_delay` before the first retry.
    #[must_use]
    pub fn with_base_delay(base_delay: std::time::Duration) -> Self {
        Self {
            base_delay,
            ..Self::new()
        }
    }

    /// A client with magi's defaults.
    #[must_use]
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::builder()
                // No overall timeout on purpose: a long turn is a long turn.
                .connect_timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap_or_default(),
            base_delay: crate::mind::provider::retry::BASE,
        }
    }

    /// Stream one turn, handing each delta to `on_delta`.
    pub async fn stream(
        &self,
        call: &Call<'_>,
        on_delta: impl FnMut(Delta),
    ) -> Result<(), ProviderError> {
        self.stream_reporting(call, on_delta, |_| {}).await
    }

    /// The same, saying when it is about to wait and try again.
    pub async fn stream_reporting(
        &self,
        call: &Call<'_>,
        mut on_delta: impl FnMut(Delta),
        mut on_retry: impl FnMut(Retrying),
    ) -> Result<(), ProviderError> {
        let mut attempt = 1;
        loop {
            // Deltas reach the caller as they arrive, so `on_retry` is the caller's signal to
            // retract whatever it has already published before the next attempt starts.
            let outcome = self.attempt(call, &mut on_delta).await;
            match outcome {
                Ok(()) => return Ok(()),
                Err(why) if why.class.is_retryable() && attempt < MAX_ATTEMPTS => {
                    let wait = crate::mind::provider::retry::backoff_from(
                        self.base_delay,
                        attempt,
                        crate::mind::provider::retry::seed(
                            &format!("{}/{}", call.provider.id, call.model.id),
                            attempt,
                        ),
                    );
                    on_retry(Retrying {
                        attempt,
                        max_attempts: MAX_ATTEMPTS,
                        delay: wait,
                        why: why.message.clone(),
                    });
                    tokio::time::sleep(wait).await;
                    attempt += 1;
                }
                Err(why) => return Err(why),
            }
        }
    }

    /// One try, with no policy about what to do if it fails.
    async fn attempt(
        &self,
        call: &Call<'_>,
        mut on_delta: impl FnMut(Delta),
    ) -> Result<(), ProviderError> {
        let Call {
            adapter,
            provider,
            model,
            context,
            options,
        } = call;
        let Some(base_url) = provider.base_url.as_deref() else {
            return Err(ProviderError::new(
                RetryClass::Invalid,
                format!("{} has no endpoint configured", provider.id),
            ));
        };
        // Resolved per attempt rather than at catalog load: an OAuth token has a lifetime.
        let key = match credential(&self.http, provider).await {
            Ok(key) => key,
            Err(why) => return Err(ProviderError::new(RetryClass::Auth, why.to_string())),
        };
        if key.is_none() && !matches!(provider.auth, crate::mind::provider::endpoint::Auth::None) {
            return Err(ProviderError::new(
                RetryClass::Auth,
                format!("{}: {}", provider.id, provider.auth.requirement()),
            ));
        }

        let mut request = self.http.post(adapter.endpoint(base_url, model));
        for (name, value) in adapter.headers(key.as_deref()) {
            request = request.header(name, value);
        }
        let body = adapter.request(model, context, options);

        let response = request
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::new(RetryClass::Transport, e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            // A context-window overflow arrives as an ordinary 400, and only the body tells it
            // apart from a malformed request, so the class is read from both.
            let detail = response.text().await.unwrap_or_default();
            let class = RetryClass::of(status.as_u16(), &detail);
            return Err(ProviderError::new(
                class,
                format!("{} returned {status}: {}", provider.id, first_line(&detail)),
            ));
        }

        let mut parser = sse::Parser::new();
        let mut state = crate::mind::provider::api::StreamState::default();
        let mut body = response.bytes_stream();
        while let Some(chunk) = body.next().await {
            let chunk =
                chunk.map_err(|e| ProviderError::new(RetryClass::Transport, e.to_string()))?;
            let text = String::from_utf8_lossy(&chunk);
            for event in parser.push(&text) {
                for delta in adapter.on_event(&mut state, &event) {
                    on_delta(delta);
                }
            }
        }
        if let Some(event) = parser.finish() {
            for delta in adapter.on_event(&mut state, &event) {
                on_delta(delta);
            }
        }
        Ok(())
    }
}

/// The first line of an error body, bounded.
fn first_line(text: &str) -> String {
    text.lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .chars()
        .take(300)
        .collect()
}

/// The credential to send, renewed if it was about to expire. The exchange is not retried: if the
/// provider will not renew, signing in again is the only thing that helps.
async fn credential(
    http: &reqwest::Client,
    provider: &Provider,
) -> Result<Option<String>, crate::mind::provider::oauth::Error> {
    let Auth::OAuth {
        token_url: Some(token_url),
        client_id: Some(client_id),
        ..
    } = &provider.auth
    else {
        return Ok(provider.auth.resolve());
    };

    let mut store = crate::mind::provider::oauth::Store::load()?;
    let tokens = store
        .get(&provider.id)
        .ok_or_else(|| crate::mind::provider::oauth::Error::NotSignedIn(provider.id.clone()))?;
    if !tokens.is_stale(crate::mind::provider::oauth::now()) {
        return Ok(Some(tokens.access.clone()));
    }
    let refresh = tokens
        .refresh
        .clone()
        .ok_or_else(|| crate::mind::provider::oauth::Error::Expired(provider.id.clone()))?;

    let renewed = crate::mind::provider::oauth::exchange(
        http,
        token_url,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh),
            ("client_id", client_id),
        ],
    )
    .await?;
    let access = renewed.access.clone();
    store.put(&provider.id, renewed);
    // Best effort: a token that works but could not be written costs a refresh next time.
    let _ = store.save();
    Ok(Some(access))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_error_body_is_reduced_to_a_sentence() {
        assert_eq!(first_line("\n\noverloaded\ndetail\n"), "overloaded");
        assert_eq!(first_line(&"x".repeat(1000)).len(), 300);
        assert_eq!(first_line(""), "");
    }
}

impl Client {
    /// Ask for one value of a known shape, and parse it. Most protocols put the answer in the
    /// response text; Anthropic has no `response_format`, so its adapter asks with a single forced
    /// tool and the value arrives as that call's arguments, which wins over any text beside it.
    pub async fn value(&self, call: &Call<'_>) -> Result<serde_json::Value, ProviderError> {
        let text = std::cell::RefCell::new(String::new());
        let args = std::cell::RefCell::new(String::new());
        self.stream_reporting(
            call,
            |delta| match delta {
                Delta::Text(chunk) => text.borrow_mut().push_str(&chunk),
                Delta::ToolCallArgs(chunk) => args.borrow_mut().push_str(&chunk),
                _ => {}
            },
            // This one keeps deltas, so it must retract them before the next attempt.
            |_| {
                text.borrow_mut().clear();
                args.borrow_mut().clear();
            },
        )
        .await?;

        let text = text.into_inner();
        let args = args.into_inner();
        let raw = if args.trim().is_empty() { &text } else { &args };
        serde_json::from_str(raw.trim()).map_err(|why| {
            ProviderError::new(
                RetryClass::Invalid,
                format!("the answer was not the shape that was asked for: {why}"),
            )
        })
    }
}

#[cfg(test)]
mod value_tests {

    #[test]
    fn a_tool_call_answer_is_preferred_over_text() {
        let text = "Sure! Here is the JSON:";
        let args = r#"{"ok":true}"#;
        let raw = if args.trim().is_empty() { text } else { args };
        assert_eq!(raw, args);
    }

    #[test]
    fn text_is_used_when_there_was_no_tool_call() {
        let text = r#"{"ok":false}"#;
        let args = "";
        let raw = if args.trim().is_empty() { text } else { args };
        assert_eq!(raw, text);
    }
}
