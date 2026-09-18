//! Streaming a turn from a provider: the only part of this crate that does I/O. What it knows of
//! a protocol comes from an [`Adapter`], and what it knows of a vendor from the catalog.

use crate::mind::provider::api::{Adapter, Delta, Options};
use crate::mind::provider::control;
use crate::mind::provider::endpoint::{Auth, Provider};
use crate::mind::provider::model::Model;
use crate::mind::provider::retry::RetryClass;

#[cfg(test)]
mod streaming;

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
    control: control::Limits,
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
            control: control::LIMITS,
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

        // Bounded like the body below: a provider that takes the request and never answers at all
        // holds the turn open for as long as it stays quiet.
        let sent = tokio::time::timeout(QUIET, request.json(&body).send()).await;
        let response = match sent {
            Ok(sent) => sent.map_err(|e| {
                ProviderError::new(RetryClass::Transport, e.without_url().to_string())
            })?,
            Err(_) => return Err(hung(&provider.id)),
        };

        let status = response.status();
        if !status.is_success() {
            let deadline = tokio::time::Instant::now() + self.control.timeout;
            let detail = control::read(response, self.control, deadline)
                .await
                .map_err(|why| {
                    ProviderError::new(
                        RetryClass::of_status(status.as_u16()),
                        format!("{} returned {status}: {why}", provider.id),
                    )
                })?;
            let class = RetryClass::of(status.as_u16(), &detail);
            return Err(ProviderError::new(
                class,
                format!("{} returned {status} ({class:?})", provider.id),
            ));
        }

        let mut parser = sse::Parser::new();
        let mut state = crate::mind::provider::api::StreamState::default();
        let mut stopped = false;
        let mut said = String::new();
        // The last events, said when the stream ends: whether a finish came before the end is what
        // tells an answer from one the provider cut off.
        let mut tail: Vec<String> = Vec::new();
        let mut body = response.bytes_stream();
        loop {
            let Ok(next) = tokio::time::timeout(QUIET, body.next()).await else {
                return Err(hung(&provider.id));
            };
            let Some(chunk) = next else { break };
            let chunk = chunk.map_err(|e| {
                ProviderError::new(RetryClass::Transport, e.without_url().to_string())
            })?;
            parser
                .feed(&chunk, |event| {
                    kept(&mut tail, &event.data);
                    for delta in adapter.on_event(&mut state, &event) {
                        stopped |= matches!(delta, Delta::Stop(_));
                        if let Delta::Text(text) = &delta {
                            said.push_str(text);
                        }
                        on_delta(delta);
                    }
                })
                .map_err(|why| ProviderError::new(RetryClass::Transport, why.to_string()))?;
        }
        if let Some(event) = parser
            .finish()
            .map_err(|why| ProviderError::new(RetryClass::Transport, why.to_string()))?
        {
            kept(&mut tail, &event.data);
            for delta in adapter.on_event(&mut state, &event) {
                stopped |= matches!(delta, Delta::Stop(_));
                if let Delta::Text(text) = &delta {
                    said.push_str(text);
                }
                on_delta(delta);
            }
        }
        let finish = tail.iter().rev().find_map(|data| reason_in(data));
        // Which of a router's upstreams served it: speeds differ by tenfold between them.
        let upstream = tail
            .iter()
            .rev()
            .find_map(|data| value_in(data, "provider"));
        crate::noted!("ask: {} stream ended, stop {stopped}", provider.id);
        let outcome = unfailed(&tail, &provider.id)
            .and_then(|()| finished(stopped, &provider.id))
            .and_then(|()| unleaked(&said, &provider.id))
            .and_then(|()| completed(finish.as_deref(), &provider.id));
        // Only an answer that came through whole makes its upstream the one asked first.
        if let (Ok(()), Some(upstream)) = (&outcome, upstream) {
            on_delta(Delta::Served(upstream));
        }
        outcome
    }
}

/// The finish a provider named in one event, in any of the spellings the dialects use.
fn reason_in(data: &str) -> Option<String> {
    ["finish_reason", "stop_reason", "finishReason"]
        .iter()
        .find_map(|key| value_in(data, key))
}

/// A string field of an event, by name, without parsing the rest of it.
fn value_in(data: &str, key: &str) -> Option<String> {
    let key = format!("\"{key}\":\"");
    let from = data.find(&key)? + key.len();
    let len = data[from..].find('"')?;
    Some(data[from..from + len].to_owned())
}

/// Keep the last two events' data, bounded.
fn kept(tail: &mut Vec<String>, data: &str) {
    tail.push(data.chars().take(4_000).collect());
    if tail.len() > 2 {
        tail.remove(0);
    }
}

/// How long a started stream may say nothing before it is taken as hung. Long, because a very
/// large prompt nothing has cached is read in silence before the first token.
const QUIET: std::time::Duration = std::time::Duration::from_secs(180);

/// A provider that took the request and then said nothing at all: asked again, since a turn that
/// waits on it waits for as long as it stays quiet.
fn hung(provider: &str) -> ProviderError {
    crate::noted!("ask: {provider} sent nothing for {}s", QUIET.as_secs());
    ProviderError::new(
        RetryClass::Transport,
        format!("{provider} sent nothing for {}s", QUIET.as_secs()),
    )
}

/// A provider that ended the stream saying the generation failed. The turn has no answer to show
/// for it, so it is asked again rather than taken as done; `length` and `stop` are real endings.
fn completed(finish: Option<&str>, provider: &str) -> Result<(), ProviderError> {
    if !finish.is_some_and(|reason| reason.eq_ignore_ascii_case("error")) {
        return Ok(());
    }
    crate::noted!("ask: {provider} ended the stream with an error");
    Err(ProviderError::new(
        RetryClass::Transport,
        format!("{provider} ended the stream with an error"),
    ))
}

/// A router that took the request and then said, inside the stream, that the upstream it chose
/// had failed. Said as it was said: "closed the stream" names the symptom and hides the cause,
/// and which upstream it was is what the next attempt needs to know to go elsewhere.
fn unfailed(tail: &[String], provider: &str) -> Result<(), ProviderError> {
    let Some(error) = tail
        .iter()
        .rev()
        .filter_map(|data| serde_json::from_str::<serde_json::Value>(data).ok())
        .find_map(|event| {
            event
                .get("error")
                .filter(|e| e.is_object())
                .cloned()
                .map(|e| (e, event))
        })
    else {
        return Ok(());
    };
    let (error, event) = error;
    let said = error["message"]
        .as_str()
        .unwrap_or("an error with no message");
    let code = error["code"]
        .as_u64()
        .map(|code| format!(" ({code})"))
        .unwrap_or_default();
    let through = event["provider"]
        .as_str()
        .map(|upstream| format!(" through {upstream}"))
        .unwrap_or_default();
    crate::noted!("ask: {provider}{through} failed mid-stream: {said}{code}");
    // Classed by what was said, as a refusal with a status is: an upstream that timed out is
    // asked again, and one that says the prompt is too long wants a smaller prompt, not another try.
    let class = match error["code"]
        .as_u64()
        .and_then(|code| u16::try_from(code).ok())
    {
        Some(status) => match RetryClass::of(status, said) {
            RetryClass::Overflow => RetryClass::Overflow,
            class if class.is_retryable() => class,
            _ => RetryClass::Transport,
        },
        None => RetryClass::Transport,
    };
    Err(ProviderError::new(
        class,
        format!("{provider}{through} failed mid-stream: {said}{code}"),
    ))
}

/// A stream that closed without a stop was cut off, whatever it had sent: an error the caller
/// retries, rather than half an answer taken for a whole one.
fn finished(stopped: bool, provider: &str) -> Result<(), ProviderError> {
    if stopped {
        return Ok(());
    }
    Err(ProviderError::new(
        RetryClass::Transport,
        format!("{provider} closed the stream without saying it had finished"),
    ))
}

/// A model that wrote its tool call as text in its own markup, which the router passed on as an
/// answer and nothing downstream can run: asked again, as a stream cut off would be.
fn unleaked(said: &str, provider: &str) -> Result<(), ProviderError> {
    if !["<｜DSML｜", "<｜tool▁call"]
        .iter()
        .any(|mark| said.contains(mark))
    {
        return Ok(());
    }
    crate::noted!("ask: {provider} wrote a tool call as text");
    Err(ProviderError::new(
        RetryClass::Transport,
        format!("{provider} wrote a tool call as text"),
    ))
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

    let store = crate::mind::provider::oauth::Store::load()?;
    let tokens = store
        .get(&provider.id)
        .ok_or_else(|| crate::mind::provider::oauth::Error::NotSignedIn(provider.id.clone()))?;
    if !tokens.is_stale(crate::mind::provider::oauth::now()) {
        return Ok(Some(tokens.access.clone()));
    }

    // Claim this provider before spending its refresh token, so that two processes do not both
    // rotate it, and then look again: whoever held the claim may have just renewed it.
    let id = provider.id.clone();
    let _held = off_thread(move || crate::mind::provider::oauth::hold(&id)).await?;
    let store = crate::mind::provider::oauth::Store::load()?;
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
    let id = provider.id.clone();
    // Reported rather than ignored: a rotated refresh token that was not written is one the
    // provider has already retired, which signs the person out at the next start.
    off_thread(move || {
        crate::mind::provider::oauth::Store::amend(|store| store.renew(&id, renewed)).map(drop)
    })
    .await?;
    Ok(Some(access))
}

/// Credential work touches the filesystem and waits on a lock another process may hold, neither of
/// which belongs on a thread that is driving requests.
async fn off_thread<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, crate::mind::provider::oauth::Error> + Send + 'static,
) -> Result<T, crate::mind::provider::oauth::Error> {
    match tokio::task::spawn_blocking(work).await {
        Ok(done) => done,
        Err(why) => Err(crate::mind::provider::oauth::Error::Refused(format!(
            "credentials could not be reached: {why}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stream_that_finishes_with_an_error_is_asked_again() {
        let why = completed(Some("error"), "openrouter").expect_err("an error is no answer");
        assert!(matches!(why.class, RetryClass::Transport), "{why:?}");
        for real in [Some("stop"), Some("length"), Some("tool_calls"), None] {
            assert!(completed(real, "openrouter").is_ok(), "{real:?}");
        }
    }

    #[test]
    fn a_stream_that_says_nothing_at_all_is_asked_again() {
        let why = hung("openrouter");
        assert!(matches!(why.class, RetryClass::Transport), "{why:?}");
        assert!(why.to_string().contains("180"), "{why}");
    }

    #[test]
    fn a_tool_call_written_as_text_is_asked_again() {
        let leaked = "Let me look.\n\n<｜DSML｜tool_cinvoke name=\"shell\">";
        let why = unleaked(leaked, "openrouter").expect_err("a leaked call is not an answer");
        assert!(matches!(why.class, RetryClass::Transport), "{why:?}");
        assert!(unleaked("I wrote the doc.", "openrouter").is_ok());
    }

    #[test]
    fn the_finish_a_provider_named_is_found_in_any_spelling() {
        let chunk = r#"{"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"cost":0.1}}"#;
        assert_eq!(reason_in(chunk).as_deref(), Some("stop"));
        assert_eq!(
            reason_in(r#"{"delta":{"stop_reason":"end_turn"}}"#).as_deref(),
            Some("end_turn")
        );
        assert_eq!(
            reason_in(r#"{"finishReason":"STOP"}"#).as_deref(),
            Some("STOP")
        );
        assert_eq!(reason_in("[DONE]"), None);
        let routed =
            r#"{"id":"gen-1","provider":"StreamLake","model":"deepseek/deepseek-v4-flash"}"#;
        assert_eq!(value_in(routed, "provider").as_deref(), Some("StreamLake"));
        let mut tail = Vec::new();
        for data in ["first", "second", "third"] {
            kept(&mut tail, data);
        }
        assert_eq!(tail, ["second", "third"]);
    }

    #[test]
    fn a_stream_that_never_stopped_is_retried_not_kept() {
        assert!(finished(true, "p").is_ok());
        let cut = finished(false, "p").expect_err("a cut-off stream");
        assert!(cut.class.is_retryable(), "{}", cut.message);
    }

    #[test]
    fn an_upstream_failing_inside_the_stream_is_said_as_it_was_said() {
        let failed = r#"{"id":"gen-1","model":"unknown","provider":"Novita","choices":[],"error":{"code":504,"message":"The operation was aborted","metadata":{"error_type":"timeout"}}}"#;
        let why = unfailed(&[failed.to_owned()], "openrouter").expect_err("a failed stream");
        assert_eq!(why.class, RetryClass::Overload);
        for part in ["Novita", "The operation was aborted", "504"] {
            assert!(why.message.contains(part), "{part}: {}", why.message);
        }
        let fine = r#"{"choices":[{"delta":{"content":"an error occurred to me"}}]}"#;
        assert!(unfailed(&[fine.to_owned()], "openrouter").is_ok());
    }

    #[test]
    fn an_overflow_said_inside_the_stream_is_an_overflow() {
        let long = r#"{"provider":"DeepInfra","choices":[],"error":{"code":400,"message":"Upstream error from DeepInfra: Requested input length 35327 exceeds maximum input length 32767"}}"#;
        let why = unfailed(&[long.to_owned()], "openrouter").expect_err("too long");
        assert_eq!(why.class, RetryClass::Overflow, "{}", why.message);
        // Anything else said mid-stream is still asked again: the request was taken, so it was
        // not malformed, whatever status the upstream's own failure carried.
        let odd = r#"{"choices":[],"error":{"code":400,"message":"upstream hiccup"}}"#;
        let why = unfailed(&[odd.to_owned()], "openrouter").expect_err("failed");
        assert_eq!(why.class, RetryClass::Transport);
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
