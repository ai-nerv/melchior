use super::*;
use crate::mind::lua::adapter::{LuaAdapter, engine_with_builtins};
use crate::mind::model::StopReason;
use crate::mind::provider::testing::{Server, Step, chunk, headers, reply};
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::sync::{Arc, atomic::Ordering};
use std::time::Duration;

fn fixture(url: &str) -> (Provider, LuaAdapter) {
    let provider: Provider = serde_json::from_value(json!({"id":"fixture","name":"Fixture",
        "api":"openai-completions","base_url":url,"auth":{"kind":"none"},
        "models":[{"id":"model","name":"Model","context_window":8000,"max_tokens":128}]}))
    .expect("provider");
    (
        provider,
        LuaAdapter::new(
            engine_with_builtins().expect("engine"),
            "openai-completions",
        )
        .expect("adapter"),
    )
}

fn event(value: Value) -> String {
    format!("data: {value}\r\n\r\n")
}

fn calls(values: Value) -> Value {
    json!({"choices":[{"delta":{"tool_calls":values}}]})
}

fn start(index: usize, id: &str, args: &str) -> Value {
    json!({"index":index,"id":id,"function":{"name":"echo","arguments":args}})
}

fn partial() -> String {
    event(calls(json!([
        start(0, "abandoned", "{"),
        start(1, "buffered", "[2]")
    ])))
}

fn completed() -> String {
    [event(calls(json!([start(0,"fresh_a","{\"city\":"),start(1,"fresh_b","[null,")]))),
        event(calls(json!([{"index":1,"function":{"arguments":"[],{}]"}},
            {"index":0,"function":{"arguments":"\"Zürich 🦀\"}"}}]))),
        event(json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]})),
        event(json!({"choices":[],"usage":{"prompt_tokens":9,"completion_tokens":3,"prompt_tokens_details":{"cached_tokens":2}}})),
        "data: [DONE]\r\n\r\n".into()].concat()
}

fn correct(events: &[Delta]) {
    let mut calls = Vec::new();
    for event in events {
        match event {
            Delta::ToolCallStart { id, name } => {
                assert_eq!(name, "echo");
                calls.push((id.clone(), String::new()));
            }
            Delta::ToolCallArgs(args) => calls.last_mut().expect("active call").1.push_str(args),
            _ => {}
        }
    }
    assert_eq!(
        calls,
        vec![
            ("fresh_a".into(), r#"{"city":"Zürich 🦀"}"#.into()),
            ("fresh_b".into(), "[null,[],{}]".into())
        ]
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Delta::Stop(_)))
            .count(),
        1
    );
    assert!(events.contains(&Delta::Stop(StopReason::ToolUse)));
    assert!(
        events
            .iter()
            .any(|e| matches!(e,Delta::Usage(u) if u.input==7 && u.cache_read==2 && u.output==3))
    );
    assert!(!format!("{events:?}").contains("abandoned"));
}

#[tokio::test]
async fn single_byte_http_chunks_preserve_interleaved_arguments_and_late_usage() {
    let mut steps = vec![headers(200)];
    steps.extend(
        completed()
            .as_bytes()
            .iter()
            .map(|byte| chunk(std::slice::from_ref(byte))),
    );
    steps.push(Step::Bytes(b"0\r\n\r\n".to_vec()));
    let server = Server::start(vec![steps]).await;
    let (provider, adapter) = fixture(&server.url);
    let mut events = Vec::new();
    tokio::time::timeout(
        Duration::from_secs(5),
        Client::new().stream(
            &Call {
                adapter: &adapter,
                provider: &provider,
                model: &provider.models[0],
                context: &Context::default(),
                options: &Options::default(),
            },
            |event| events.push(event),
        ),
    )
    .await
    .expect("deadline")
    .expect("stream");
    correct(&events);
    assert_eq!(server.requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn dropped_stream_retry_resets_buffered_calls_without_changing_request_bytes() {
    let delivered = Arc::new(tokio::sync::Notify::new());
    let server = Server::start(vec![
        vec![
            headers(200),
            chunk(partial().as_bytes()),
            Step::Wait(Arc::clone(&delivered)),
        ],
        reply(200, completed().as_bytes()),
    ])
    .await;
    let (provider, adapter) = fixture(&server.url);
    let events = RefCell::new(Vec::new());
    let retries = Cell::new(0);
    tokio::time::timeout(
        Duration::from_secs(5),
        Client::with_base_delay(Duration::ZERO).stream_reporting(
            &Call {
                adapter: &adapter,
                provider: &provider,
                model: &provider.models[0],
                context: &Context::default(),
                options: &Options::default(),
            },
            |event| {
                if matches!(&event,Delta::ToolCallArgs(args) if args=="{") {
                    delivered.notify_one();
                }
                events.borrow_mut().push(event);
            },
            |_| {
                assert!(
                    events
                        .borrow()
                        .iter()
                        .any(|e| matches!(e,Delta::ToolCallStart{id,..} if id=="abandoned"))
                );
                assert!(!events.borrow().iter().any(|e| matches!(e, Delta::Stop(_))));
                assert!(
                    !events
                        .borrow()
                        .iter()
                        .any(|e| matches!(e,Delta::ToolCallStart{id,..} if id=="buffered"))
                );
                events.borrow_mut().clear();
                retries.set(retries.get() + 1);
            },
        ),
    )
    .await
    .expect("deadline")
    .expect("retried");
    correct(&events.into_inner());
    assert_eq!(retries.get(), 1);
    assert_eq!(server.requests.load(Ordering::SeqCst), 2);
    let bodies = server.bodies.lock().expect("requests");
    assert_eq!(bodies[0], bodies[1]);
}

#[tokio::test]
async fn cancellation_closes_socket_and_next_stream_has_no_abandoned_or_buffered_calls() {
    let delivered = Arc::new(tokio::sync::Notify::new());
    let server = Server::start(vec![
        vec![headers(200), chunk(partial().as_bytes()), Step::Hold],
        reply(200, completed().as_bytes()),
    ])
    .await;
    let (provider, adapter) = fixture(&server.url);
    let (context, options, client) = (Context::default(), Options::default(), Client::new());
    let call = Call {
        adapter: &adapter,
        provider: &provider,
        model: &provider.models[0],
        context: &context,
        options: &options,
    };
    let mut events = Vec::new();
    let mut pending = Box::pin(client.stream(&call, |event| {
        if matches!(&event,Delta::ToolCallArgs(args) if args=="{") {
            delivered.notify_one();
        }
        events.push(event);
    }));
    tokio::time::timeout(Duration::from_secs(5),async {
        tokio::select! {()=delivered.notified()=>{}, result=&mut pending=>panic!("ended before cancellation: {result:?}")}
    }).await.expect("delivery");
    drop(pending);
    server.wait_for(&server.closed, 1).await;
    assert!(!events.iter().any(|e| matches!(e, Delta::Stop(_))));
    assert!(
        !events
            .iter()
            .any(|e| matches!(e,Delta::ToolCallStart{id,..} if id=="buffered"))
    );
    events.clear();
    tokio::time::timeout(
        Duration::from_secs(5),
        client.stream(&call, |e| events.push(e)),
    )
    .await
    .expect("deadline")
    .expect("fresh stream");
    correct(&events);
}

#[tokio::test]
async fn truncation_and_malformed_terminal_streams_retain_usage_without_executable_success() {
    for reason in ["repetition_truncation", "content_filter", "unexpected"] {
        let script = [
            event(calls(json!([start(0, "a", "{}"), start(1, "b", "[]")]))),
            event(json!({"choices":[{"finish_reason":reason}]})),
            event(json!({"usage":{"prompt_tokens":9,"completion_tokens":3}})),
            "data: [DONE]\n\n".into(),
        ]
        .concat();
        let server = Server::start(vec![reply(200, script.as_bytes())]).await;
        let (provider, adapter) = fixture(&server.url);
        let mut events = Vec::new();
        tokio::time::timeout(
            Duration::from_secs(5),
            Client::new().stream(
                &Call {
                    adapter: &adapter,
                    provider: &provider,
                    model: &provider.models[0],
                    context: &Context::default(),
                    options: &Options::default(),
                },
                |event| events.push(event),
            ),
        )
        .await
        .expect("deadline")
        .expect("protocol stop");
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, Delta::Stop(_)))
                .count(),
            1
        );
        assert!(
            events.contains(&Delta::Stop(if reason == "repetition_truncation" {
                StopReason::Length
            } else {
                StopReason::Error
            }))
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e,Delta::Usage(u) if u.input==9 && u.output==3))
        );
        assert_eq!(server.requests.load(Ordering::SeqCst), 1);
    }
}
