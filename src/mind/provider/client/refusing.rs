//! How a provider's refusal reaches the caller: with what it said, after as long as it asked, and
//! a request that cannot be built is not asked again.

use super::*;
use crate::mind::lua::adapter::{LuaAdapter, engine_with_builtins};
use crate::mind::provider::testing::{Server, Step, reply};
use serde_json::json;
use std::sync::atomic::Ordering;
use std::time::Duration;

fn model() -> Model {
    serde_json::from_value(
        json!({"id":"test","name":"Test","context_window":8000,"max_tokens":100}),
    )
    .expect("model")
}

fn provider(url: &str) -> Provider {
    Provider {
        id: "local".into(),
        name: "Local".into(),
        base_url: Some(url.into()),
        api: crate::mind::provider::model::Api::OpenAiCompletions,
        auth: Auth::None,
        compat: None,
        models: Vec::new(),
        discover: false,
        details: None,
        avoid: Vec::new(),
    }
}

fn lua() -> LuaAdapter {
    LuaAdapter::new(
        engine_with_builtins().expect("protocols"),
        "openai-completions",
    )
    .expect("adapter")
}

/// The shipped protocol, sending one header no request can carry.
struct Broken(LuaAdapter);

impl Adapter for Broken {
    fn endpoint(&self, base_url: &str, model: &Model) -> String {
        self.0.endpoint(base_url, model)
    }
    fn headers(&self, _key: Option<&str>) -> Vec<(String, String)> {
        vec![(
            "authorization".to_owned(),
            "Bearer sk-with\na-line-break".to_owned(),
        )]
    }
    fn request(&self, model: &Model, context: &Context, options: &Options) -> serde_json::Value {
        self.0.request(model, context, options)
    }
    fn on_event(
        &self,
        state: &mut crate::mind::provider::api::StreamState,
        event: &sse::Event,
    ) -> Vec<Delta> {
        self.0.on_event(state, event)
    }
}

/// One turn through `adapter` at `url`, with the waits it was told about.
async fn asked(adapter: &dyn Adapter, url: &str) -> (Result<(), ProviderError>, Vec<Retrying>) {
    let (provider, model, context, options) = (
        provider(url),
        model(),
        Context::default(),
        Options::default(),
    );
    let mut waits = Vec::new();
    let client = Client::with_base_delay(Duration::ZERO);
    let outcome = tokio::time::timeout(
        Duration::from_secs(10),
        client.stream_reporting(
            &Call {
                adapter,
                provider: &provider,
                model: &model,
                context: &context,
                options: &options,
            },
            |_| {},
            |retry| waits.push(retry),
        ),
    )
    .await
    .expect("local deadline");
    (outcome, waits)
}

#[tokio::test]
async fn a_refusal_says_what_the_provider_said() {
    let body = br#"{"error":{"message":"This request requires more credits.","code":402}}"#;
    let server = Server::start(vec![reply(402, body)]).await;
    let (outcome, waits) = asked(&lua(), &server.url).await;
    let why = outcome.expect_err("refused");
    assert!(why.message.contains("402"), "{}", why.message);
    assert!(
        why.message.contains("requires more credits"),
        "{}",
        why.message
    );
    assert!(waits.is_empty(), "a payment refusal was asked again");
}

#[tokio::test]
async fn a_provider_that_says_when_to_come_back_is_left_that_long() {
    let throttled = "HTTP/1.1 429 Slow\r\nRetry-After-Ms: 300\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}";
    let fine = format!(
        "data: {}\n\ndata: {}\n\n",
        json!({"choices":[{"delta":{"content":"ok"}}]}),
        json!({"choices":[{"delta":{},"finish_reason":"stop"}]})
    );
    let server = Server::start(vec![
        vec![Step::Bytes(throttled.as_bytes().to_vec())],
        reply(200, fine.as_bytes()),
    ])
    .await;
    let (outcome, waits) = asked(&lua(), &server.url).await;
    outcome.expect("answered on the second try");
    assert_eq!(waits.len(), 1, "{waits:?}");
    assert!(waits[0].delay >= Duration::from_millis(300), "{waits:?}");
    assert!(waits[0].delay <= Duration::from_millis(330), "{waits:?}");
}

#[tokio::test]
async fn a_header_that_cannot_be_built_is_refused_once_and_never_sent() {
    let server = Server::start(vec![reply(200, b"{}")]).await;
    let (outcome, waits) = asked(&Broken(lua()), &server.url).await;
    let why = outcome.expect_err("no request");
    assert_eq!(why.class, RetryClass::Invalid, "{}", why.message);
    assert!(
        !why.message.contains("sk-with"),
        "the key was echoed: {}",
        why.message
    );
    assert!(waits.is_empty(), "asked again {} times", waits.len());
    assert_eq!(server.requests.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_provider_off_this_machine_over_plain_http_is_never_asked() {
    let (outcome, waits) = asked(&lua(), "http://models.invalid/v1").await;
    let why = outcome.expect_err("refused");
    assert_eq!(why.class, RetryClass::Invalid, "{}", why.message);
    assert!(
        why.message.contains("http://models.invalid "),
        "{}",
        why.message
    );
    assert!(waits.is_empty(), "asked again {} times", waits.len());
}

#[test]
fn a_wait_asked_for_is_read_in_milliseconds_or_seconds_and_capped() {
    let mut headers = reqwest::header::HeaderMap::new();
    assert_eq!(crate::mind::provider::retry::after(&headers), None);
    headers.insert("retry-after", "7".parse().expect("a value"));
    assert_eq!(
        crate::mind::provider::retry::after(&headers),
        Some(Duration::from_secs(7))
    );
    headers.insert("retry-after-ms", "250".parse().expect("a value"));
    assert_eq!(
        crate::mind::provider::retry::after(&headers),
        Some(Duration::from_millis(250))
    );
    headers.remove("retry-after-ms");
    headers.insert("retry-after", "999999".parse().expect("a value"));
    assert!(crate::mind::provider::retry::after(&headers) <= Some(Duration::from_secs(600)));
    headers.insert(
        "retry-after",
        "Wed, 21 Oct 2015 07:28:00 GMT".parse().expect("a date"),
    );
    assert_eq!(crate::mind::provider::retry::after(&headers), None);
}

#[test]
fn a_refusal_that_echoes_a_key_says_the_reason_without_it() {
    let echoed =
        r#"{"error":{"message":"Incorrect API key provided: sk-proj-a1b2c3d4e5f6g7h8i9j0k1l2."}}"#;
    let said = summary(echoed);
    assert!(said.contains("Incorrect API key provided"), "{said}");
    assert!(!said.contains("a1b2c3d4"), "{said}");
    assert_eq!(
        summary("raw text with SECRET-bearer-123"),
        "it gave no reason a person can read"
    );
}
