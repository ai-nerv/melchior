#![allow(clippy::expect_used, clippy::indexing_slicing)]

use futures_util::FutureExt;
use melchior::mind::{
    lua::adapter::{LuaAdapter, engine_with_builtins},
    model::{Context, Message},
    provider::{
        api::{Delta, Options},
        client::{Call, Client},
        endpoint::Provider,
    },
};
use melchior::{events::*, framing};
use serde_json::json;
use std::{collections::HashSet, sync::atomic::Ordering, time::Duration};

#[allow(dead_code)]
#[path = "../src/mind/provider/testing.rs"]
mod fixture;
use fixture::{Server, Step, chunk, headers, reply};

fn provider(url: &str) -> Provider {
    serde_json::from_value(json!({"id":"private-provider-canary", "name":"private-provider-canary",
        "base_url":url, "api":"openai-completions", "auth":{"kind":"none"},
        "models":[{"id":"private-model-canary", "name":"private-model-canary", "context_window":8000,"max_tokens":900}]})).expect("provider")
}

fn success() -> Vec<Step> {
    reply(200, b"data: {\"choices\":[{\"delta\":{\"content\":\"private-answer-canary\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n")
}

struct Native {
    client: Client,
    adapter: LuaAdapter,
    provider: Provider,
    context: Context,
    options: Options,
}

impl Native {
    fn new(url: &str) -> Self {
        Self {
            client: Client::new(),
            adapter: LuaAdapter::new(
                engine_with_builtins().expect("builtins"),
                "openai-completions",
            )
            .expect("adapter"),
            provider: provider(url),
            context: Context {
                messages: vec![Message::user("private-input-canary")],
                ..Default::default()
            },
            options: Options::default(),
        }
    }

    fn call(&self) -> Call<'_> {
        Call {
            adapter: &self.adapter,
            provider: &self.provider,
            model: &self.provider.models[0],
            context: &self.context,
            options: &self.options,
        }
    }

    async fn success(&self) {
        let mut text = String::new();
        tokio::time::timeout(
            Duration::from_secs(20),
            self.client.stream(&self.call(), |delta| {
                if let Delta::Text(part) = delta {
                    text.push_str(&part);
                }
            }),
        )
        .await
        .expect("bounded stream")
        .expect("successful stream");
        assert_eq!(text, "private-answer-canary");
    }
}

fn observed(receiver: &Receiver) -> Vec<Event> {
    let mut found = Vec::new();
    while let Ok(bytes) = receiver.try_recv() {
        let text = std::str::from_utf8(&bytes).expect("UTF8");
        for canary in [
            "private-provider-canary",
            "private-model-canary",
            "private-input-canary",
            "private-answer-canary",
            "private-error-canary",
            "127.0.0.1",
            "invalid",
        ] {
            assert!(!text.contains(canary), "event leaked {canary}");
        }
        let event = decode(&bytes).expect("schema");
        assert!(event.correlation.ask_id.is_some());
        assert!(event.correlation.attempt_id.is_some());
        assert_eq!(
            event.correlation,
            Correlation {
                ask_id: event.correlation.ask_id,
                attempt_id: event.correlation.attempt_id,
                ..Correlation::default()
            }
        );
        found.push(event);
    }
    for pair in found.windows(2) {
        assert!(pair[0].sequence < pair[1].sequence);
    }
    found
}

fn facts(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .map(|event| {
            let value = serde_json::to_value(event.fact).expect("fact");
            format!(
                "{}/{}",
                value["kind"].as_str().expect("kind"),
                value["outcome"].as_str().expect("outcome")
            )
        })
        .collect()
}

fn same_attempt(events: &[Event]) {
    assert!(!events.is_empty());
    assert!(
        events
            .iter()
            .all(|event| event.correlation == events[0].correlation)
    );
    for event in events {
        if let Fact::ProviderAttempt {
            outcome,
            duration_ms,
        } = event.fact
        {
            assert_eq!(duration_ms.is_some(), outcome != AttemptOutcome::Dispatched);
        }
    }
}

async fn successful_retry(receiver: &Receiver) {
    let server = Server::start(vec![reply(503, b"private-error-canary"), success()]).await;
    Native::new(&server.url).success().await;
    assert_eq!(server.requests.load(Ordering::SeqCst), 2);
    let events = observed(receiver);
    assert_eq!(
        facts(&events),
        [
            "provider_attempt/dispatched",
            "provider_attempt/failed",
            "retry/scheduled",
            "retry/dispatched",
            "provider_attempt/dispatched",
            "provider_attempt/succeeded"
        ]
    );
    same_attempt(&events[..2]);
    same_attempt(&events[2..]);
    assert_eq!(events[0].correlation.ask_id, events[2].correlation.ask_id);
    assert_ne!(
        events[0].correlation.attempt_id,
        events[2].correlation.attempt_id
    );
}

async fn refusals(receiver: &Receiver) {
    let mut native = Native::new("http://remote.invalid/private-path-canary");
    assert!(native.client.stream(&native.call(), |_| {}).await.is_err());
    let events = observed(receiver);
    assert_eq!(facts(&events), ["provider_attempt/refused"]);
    same_attempt(&events);
    native.provider.base_url = None;
    assert!(native.client.stream(&native.call(), |_| {}).await.is_err());
    let missing = observed(receiver);
    assert_eq!(facts(&missing), ["provider_attempt/refused"]);
    assert_ne!(events[0].correlation.ask_id, missing[0].correlation.ask_id);
}

async fn cancel_request(receiver: &Receiver, in_body: bool) {
    let steps = if in_body {
        vec![
            headers(200),
            chunk(b"data: {\"choices\":[{\"delta\":{\"content\":\"private-answer-canary\"}}]}\n\n"),
            Step::Hold,
        ]
    } else {
        vec![Step::Hold]
    };
    let server = Server::start(vec![steps]).await;
    let native = Native::new(&server.url);
    let call = native.call();
    let ready = tokio::sync::Notify::new();
    let mut stream = Box::pin(native.client.stream(&call, |delta| {
        if matches!(delta, Delta::Text(_)) {
            ready.notify_one();
        }
    }));
    tokio::select! {
        result = &mut stream => panic!("unexpected completion: {result:?}"),
        () = async { if in_body { tokio::time::timeout(Duration::from_secs(3), ready.notified()).await.expect("body observed"); } else { server.wait_for(&server.requests, 1).await; } } => {}
    }
    drop(stream);
    server.wait_for(&server.closed, 1).await;
    let events = observed(receiver);
    assert_eq!(
        facts(&events),
        ["provider_attempt/dispatched", "provider_attempt/cancelled"]
    );
    same_attempt(&events);
    assert_eq!(server.requests.load(Ordering::SeqCst), 1);
}

async fn cancel_retry(receiver: &Receiver) {
    let server = Server::start(vec![reply(503, b"private-error-canary")]).await;
    let native = Native::new(&server.url);
    let call = native.call();
    let ready = tokio::sync::Notify::new();
    let mut stream = Box::pin(native.client.stream_reporting(
        &call,
        |_| {},
        |_| ready.notify_one(),
    ));
    tokio::select! {
        result = &mut stream => panic!("unexpected completion: {result:?}"),
        () = async { tokio::time::timeout(Duration::from_secs(3), ready.notified()).await.expect("retry scheduled"); } => {}
    }
    drop(stream);
    assert_eq!(server.requests.load(Ordering::SeqCst), 1);
    let events = observed(receiver);
    assert_eq!(
        facts(&events),
        [
            "provider_attempt/dispatched",
            "provider_attempt/failed",
            "retry/scheduled",
            "retry/cancelled"
        ]
    );
    same_attempt(&events[..2]);
    same_attempt(&events[2..]);
    assert_eq!(events[0].correlation.ask_id, events[2].correlation.ask_id);
    assert_ne!(
        events[0].correlation.attempt_id,
        events[2].correlation.attempt_id
    );
}

async fn callback_failures(receiver: &Receiver) {
    let server = Server::start(vec![success(), reply(503, b"private-error-canary")]).await;
    let native = Native::new(&server.url);
    let call = native.call();
    let failed = std::panic::AssertUnwindSafe(
        native
            .client
            .stream(&call, |_| panic!("private-callback-canary")),
    )
    .catch_unwind()
    .await;
    assert!(failed.is_err());
    let events = observed(receiver);
    assert_eq!(
        facts(&events),
        ["provider_attempt/dispatched", "provider_attempt/failed"]
    );
    same_attempt(&events);
    let failed = std::panic::AssertUnwindSafe(native.client.stream_reporting(
        &call,
        |_| {},
        |_| panic!("private-callback-canary"),
    ))
    .catch_unwind()
    .await;
    assert!(failed.is_err());
    let events = observed(receiver);
    assert_eq!(
        facts(&events),
        [
            "provider_attempt/dispatched",
            "provider_attempt/failed",
            "retry/scheduled",
            "retry/failed"
        ]
    );
    same_attempt(&events[..2]);
    same_attempt(&events[2..]);
    assert_eq!(server.requests.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn actual_attempts_retries_and_cancellation_are_content_free_and_nonblocking() {
    assert_eq!(emit(|| panic!("disabled preparation")), Emitted::Disabled);
    let server = Server::start(vec![success(), success(), success(), success()]).await;
    let native = Native::new(&server.url);
    native.success().await;
    assert_eq!(loss(), Loss::default());
    let receiver = enable_local().expect("explicit isolated activation");
    assert!(matches!(enable_local(), Err(EnableError::AlreadyEnabled)));
    drop(native.client.stream(&native.call(), |_| {}));
    assert!(
        observed(&receiver).is_empty(),
        "unpolled stream did not start"
    );
    native.success().await;
    let events = observed(&receiver);
    assert_eq!(
        facts(&events),
        ["provider_attempt/dispatched", "provider_attempt/succeeded"]
    );
    same_attempt(&events);
    successful_retry(&receiver).await;
    refusals(&receiver).await;
    cancel_request(&receiver, false).await;
    cancel_request(&receiver, true).await;
    cancel_retry(&receiver).await;
    callback_failures(&receiver).await;
    let ids: HashSet<_> = events
        .iter()
        .map(|event| String::from(event.correlation.attempt_id.expect("ID")))
        .collect();
    assert_eq!(ids.len(), 1);
    for _ in 0..QUEUED_EVENTS {
        assert_eq!(
            emit(|| (
                Correlation::default(),
                Fact::Retry {
                    outcome: RetryOutcome::Scheduled
                }
            )),
            Emitted::Queued
        );
    }
    native.success().await;
    assert_eq!(loss().full, 2);
    for _ in 0..QUEUED_EVENTS {
        receiver.try_recv().expect("queued filler");
    }
    assert!(receiver.try_recv().is_err());
    drop(receiver);
    native.success().await;
    assert_eq!(
        loss(),
        Loss {
            full: 2,
            closed: 2,
            ..Loss::default()
        }
    );
    assert_eq!(server.requests.load(Ordering::SeqCst), 4);
}
