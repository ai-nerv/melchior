//! Streaming a turn from a provider: the only part of this crate that does I/O. What it knows of
//! a protocol comes from an [`Adapter`], and what it knows of a vendor from the catalog.

use crate::mind::provider::Unanswered;
use crate::mind::provider::api::{Adapter, Delta, Options};
use crate::mind::provider::control;
use crate::mind::provider::endpoint::{Auth, Provider};
use crate::mind::provider::model::Model;
use crate::mind::provider::retry::RetryClass;

mod budget;
mod observing;
mod overflow;

#[cfg(test)]
mod cache_tests;
#[cfg(test)]
mod overflow_tests;
#[cfg(test)]
mod refusing;
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
    pub overflow: Option<crate::mind::model::overflow::Detail>,
    /// Whether asking again could help. The class's own answer, unless this failure is one the
    /// class usually recovers from and this one will not.
    pub again: bool,
    /// How long the provider said to wait before asking again, when it said.
    pub after: Option<std::time::Duration>,
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
            again: class.is_retryable(),
            after: None,
            overflow: None,
        }
    }

    /// The same failure, not worth asking again.
    fn finally(self) -> Self {
        Self {
            again: false,
            ..self
        }
    }
}

/// Streams turns from providers.
pub struct Client {
    http: crate::mind::provider::Http,
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
            http: crate::mind::provider::http()
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
        on_delta: impl FnMut(Delta),
        on_retry: impl FnMut(Retrying),
    ) -> Result<(), ProviderError> {
        self.stream_observing(call, on_delta, on_retry, |_| {})
            .await
    }

    /// Stream with serialization observations before each HTTP attempt.
    pub async fn stream_observing(
        &self,
        call: &Call<'_>,
        mut on_delta: impl FnMut(Delta),
        on_retry: impl FnMut(Retrying),
        on_frame: impl FnMut(crate::mind::model::RequestFrame),
    ) -> Result<(), ProviderError> {
        self.stream_traced(
            call,
            |delta, _| on_delta(delta),
            on_retry,
            on_frame,
            crate::mind::accounting::Trace::attempts(),
        )
        .await
    }

    pub(crate) async fn stream_traced(
        &self,
        call: &Call<'_>,
        mut on_delta: impl FnMut(Delta, crate::mind::model::MeasuredUsage),
        mut on_retry: impl FnMut(Retrying),
        mut on_frame: impl FnMut(crate::mind::model::RequestFrame),
        trace: crate::mind::accounting::Trace,
    ) -> Result<(), ProviderError> {
        let mut attempt = 1;
        let mut observed = observing::Attempt::new(trace.clone());
        let mut prepared = budget::Prepared::new(call, call.options);
        loop {
            // Deltas reach the caller as they arrive, so `on_retry` is the caller's signal to
            // retract whatever it has already published before the next attempt starts.
            let outcome = self
                .attempt(call, &prepared, &mut on_delta, &mut on_frame, &mut observed)
                .await;
            observed.finish(outcome.is_ok());
            match outcome {
                Ok(()) => return Ok(()),
                Err(why) if attempt < MAX_ATTEMPTS => {
                    let lowered = prepared.lowered(call, &why);
                    if lowered.is_none() && !why.again {
                        return Err(why);
                    }
                    // Seeded by this process too, so sessions sharing a model do not ask in step.
                    let seed = crate::mind::provider::retry::seed(
                        &format!(
                            "{}/{}/{}",
                            call.provider.id,
                            call.model.id,
                            std::process::id()
                        ),
                        attempt,
                    );
                    let wait =
                        crate::mind::provider::retry::backoff_from(self.base_delay, attempt, seed);
                    let wait = if let Some(next) = lowered {
                        prepared = next;
                        std::time::Duration::ZERO
                    } else {
                        crate::mind::provider::retry::at_least(wait, why.after, seed)
                    };
                    let waiting = observing::Waiting::new(trace.clone());
                    on_retry(Retrying {
                        attempt,
                        max_attempts: MAX_ATTEMPTS,
                        delay: wait,
                        why: why.message.clone(),
                    });
                    tokio::time::sleep(wait).await;
                    observed = waiting.dispatch();
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
        prepared: &budget::Prepared,
        mut on_delta: impl FnMut(Delta, crate::mind::model::MeasuredUsage),
        mut on_frame: impl FnMut(crate::mind::model::RequestFrame),
        observed: &mut observing::Attempt,
    ) -> Result<(), ProviderError> {
        let Call {
            adapter,
            provider,
            model,
            ..
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
            // A header that cannot be built is the request's fault, and asking again repeats it.
            let (Ok(named), Ok(valued)) = (
                reqwest::header::HeaderName::from_bytes(name.as_bytes()),
                reqwest::header::HeaderValue::from_str(&value),
            ) else {
                return Err(ProviderError::new(
                    RetryClass::Invalid,
                    format!(
                        "{}: the `{name}` header cannot be sent as it stands — a key with a space \
                         or a line break in it, perhaps",
                        provider.id
                    ),
                ));
            };
            request = request.header(named, valued);
        }
        if let Some(frame) = prepared.frame {
            on_frame(frame);
        }

        // Bounded like the body below: a provider that takes the request and never answers at all
        // holds the turn open for as long as it stays quiet.
        let sent = tokio::time::timeout(
            QUIET,
            request
                .json(&prepared.body)
                .send_observing(|| observed.dispatched()),
        )
        .await;
        let response = match sent {
            Ok(Err(refused @ Unanswered::Refused { .. })) => {
                return Err(ProviderError::new(
                    RetryClass::Invalid,
                    format!("{}: {refused}", provider.id),
                ));
            }
            Ok(Err(Unanswered::Http(e))) if e.is_builder() => {
                return Err(ProviderError::new(
                    RetryClass::Invalid,
                    format!(
                        "{}: the request could not be built: {}",
                        provider.id,
                        e.without_url()
                    ),
                ));
            }
            Ok(Err(Unanswered::Http(e))) => {
                return Err(unreached(&format!("could not reach {}", provider.id), e));
            }
            Ok(Ok(response)) => response,
            Err(_) => return Err(hung(&provider.id)),
        };

        let status = response.status();
        if !status.is_success() {
            let after = crate::mind::provider::retry::after(response.headers());
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
            let mut refused = ProviderError::new(
                class,
                format!(
                    "{} returned {status} ({class:?}): {}",
                    provider.id,
                    summary(&detail)
                ),
            );
            refused.after = after;
            refused.overflow = overflow::classified(model.api, status.as_u16(), class, &detail);
            return Err(refused);
        }

        let mut parser = sse::Parser::new();
        let mut state = crate::mind::provider::api::StreamState::default();
        if let Some(kept) = &prepared.scratch {
            state.scratch = kept.clone();
        }
        // A reply that is one JSON document and no stream yields no events; it is kept, within a
        // bound, to be handed over whole once it is known that none came.
        let mut whole: Vec<u8> = Vec::new();
        let mut events = 0usize;
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
                unreached(
                    &format!("lost {} partway through its answer", provider.id),
                    e,
                )
            })?;
            if events == 0 && whole.len() + chunk.len() <= WHOLE {
                whole.extend_from_slice(&chunk);
            }
            parser
                .feed(&chunk, |event| {
                    events += 1;
                    kept(&mut tail, &event.data);
                    for (delta, measured) in adapter.on_event_measured(&mut state, &event) {
                        stopped |= matches!(delta, Delta::Stop(_));
                        if let Delta::Text(text) = &delta {
                            said.push_str(text);
                        }
                        on_delta(delta, measured);
                    }
                })
                .map_err(|why| unparsed(why, &provider.id))?;
        }
        if let Some(event) = parser.finish().map_err(|why| unparsed(why, &provider.id))? {
            events += 1;
            kept(&mut tail, &event.data);
            for (delta, measured) in adapter.on_event_measured(&mut state, &event) {
                stopped |= matches!(delta, Delta::Stop(_));
                if let Delta::Text(text) = &delta {
                    said.push_str(text);
                }
                on_delta(delta, measured);
            }
        }
        // No event at all, and what came is one JSON document: a dialect that answers whole
        // rather than in a stream is handed it as one event, named for what it is.
        if events == 0 && serde_json::from_slice::<serde_json::Value>(&whole).is_ok() {
            let event = sse::Event {
                name: "body".to_owned(),
                data: String::from_utf8_lossy(&whole).into_owned(),
            };
            kept(&mut tail, &event.data);
            for (delta, measured) in adapter.on_event_measured(&mut state, &event) {
                stopped |= matches!(delta, Delta::Stop(_));
                if let Delta::Text(text) = &delta {
                    said.push_str(text);
                }
                on_delta(delta, measured);
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
            on_delta(Delta::Served(upstream), state.measured_usage);
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

/// A request as a dialect built it, apart from what the dialect kept for itself under
/// `__scratch`: what it needs in order to read the reply, which is no part of what is sent.
fn parted(mut body: serde_json::Value) -> (serde_json::Value, Option<serde_json::Value>) {
    if let Some(fields) = body.as_object_mut() {
        fields.remove("__budget");
    }
    let kept = body
        .as_object_mut()
        .and_then(|fields| fields.remove("__scratch"));
    (body, kept)
}

/// The most of a reply kept in case it turns out to be one document and no stream.
const WHOLE: usize = 1 << 20;

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

/// A request that never reached the provider, said as the provider and the reason underneath:
/// reqwest's own line is "error sending request", and "No route to host" is two sources below it.
/// The URL stays out, since some providers carry the key in its query string.
fn unreached(what: &str, error: reqwest::Error) -> ProviderError {
    let (deepest, routeless) = underneath(&error);
    let said = deepest.unwrap_or_else(|| error.without_url().to_string());
    let failed = ProviderError::new(RetryClass::Transport, format!("{what}: {said}"));
    // No route is the network saying the host is not there, and a minute of backoff does not put
    // it back. A refused connection is not this: that host is up, and a daemon restarting on it
    // answers seconds later.
    if routeless { failed.finally() } else { failed }
}

/// The innermost reason beneath `error`, and whether any reason on the way says there is no route.
fn underneath(error: &(dyn std::error::Error + 'static)) -> (Option<String>, bool) {
    let mut deepest = None;
    let mut routeless = false;
    let mut cause = error.source();
    while let Some(found) = cause {
        deepest = Some(found.to_string());
        routeless |= found.downcast_ref::<std::io::Error>().is_some_and(|io| {
            matches!(
                io.kind(),
                std::io::ErrorKind::HostUnreachable | std::io::ErrorKind::NetworkUnreachable
            )
        });
        cause = found.source();
    }
    (deepest, routeless)
}

/// A stream that could not be read: broken text is asked again, and one larger than this reads is
/// not, since it would be as large the next time.
fn unparsed(why: sse::Malformed, provider: &str) -> ProviderError {
    let failed = ProviderError::new(RetryClass::Transport, format!("{provider}: {why}"));
    if why == sse::Malformed::Utf8 {
        failed
    } else {
        failed.finally()
    }
}

/// What a provider said when it refused, as one short line: the `message` of the usual JSON error
/// and nothing else, with anything shaped like a credential taken out. A body that is not that
/// shape may be the request echoed back, so it is not repeated.
fn summary(detail: &str) -> String {
    let said = serde_json::from_str::<serde_json::Value>(detail)
        .ok()
        .and_then(|body| {
            body.pointer("/error/message")
                .or_else(|| body.get("message"))
                .or_else(|| body.get("error"))
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
        })
        .unwrap_or_default();
    let masked: Vec<&str> = said
        .split_whitespace()
        .map(|word| if secretive(word) { "<redacted>" } else { word })
        .collect();
    let said = crate::noted::short(&masked.join(" "));
    if said.is_empty() {
        "it gave no reason a person can read".to_owned()
    } else {
        said
    }
}

/// Whether a word looks like a key: long, unbroken, and mixing letters with digits.
fn secretive(word: &str) -> bool {
    let core = word.trim_matches(|c: char| !c.is_ascii_alphanumeric());
    core.len() >= 20
        && core
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && core.chars().any(|c| c.is_ascii_digit())
        && core.chars().any(|c| c.is_ascii_alphabetic())
}

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
    let said = error
        .get("message")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("an error with no message");
    let status = error.get("code").and_then(serde_json::Value::as_u64);
    let code = status.map(|code| format!(" ({code})")).unwrap_or_default();
    let through = event
        .get("provider")
        .and_then(serde_json::Value::as_str)
        .map(|upstream| format!(" through {upstream}"))
        .unwrap_or_default();
    crate::noted!("ask: {provider}{through} failed mid-stream: {said}{code}");
    // Classed by what was said, as a refusal with a status is: an upstream that timed out is
    // asked again, and one that says the prompt is too long wants a smaller prompt, not another try.
    let class = match status.and_then(|code| u16::try_from(code).ok()) {
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
    http: &crate::mind::provider::Http,
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
mod tests;

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
