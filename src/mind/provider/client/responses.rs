use super::*;
use crate::mind::lua::adapter::{LuaAdapter, engine_with_builtins};
use crate::mind::model::StopReason;
use crate::mind::provider::testing::{Server, Step, chunk, headers, reply};
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::sync::{Arc, atomic::Ordering};
use std::time::Duration;

const APIS: [&str; 3] = [
    "openai-responses",
    "azure-openai-responses",
    "openai-codex-responses",
];

fn provider(api: &str, url: &str) -> Provider {
    serde_json::from_value(json!({"id":"fixture","name":"Fixture","api":api,
        "base_url":url,"auth":{"kind":"none"},"models":[{
            "id":"model","name":"Model","context_window":8000,"max_tokens":128,"reasoning":true}]}))
    .expect("provider")
}

fn adapter(api: &str) -> LuaAdapter {
    LuaAdapter::new(engine_with_builtins().expect("engine"), api).expect("adapter")
}

fn call(id: &str, arguments: &str) -> Value {
    json!({"type":"function_call","id":format!("fc_{id}"),"call_id":id,"name":"echo","arguments":arguments})
}

fn frame(value: Value) -> String {
    format!("data: {value}\r\n\r\n")
}

fn partial() -> String {
    [json!({"type":"response.output_item.added","output_index":0,"item":call("abandoned","")}),
        json!({"type":"response.function_call_arguments.delta","output_index":0,"item_id":"fc_abandoned","delta":"["}),
        json!({"type":"response.output_item.added","output_index":1,"item":call("buffered","")}),
        json!({"type":"response.function_call_arguments.delta","output_index":1,"item_id":"fc_buffered","delta":"{\"discard\":true}"})]
        .into_iter().map(frame).collect()
}

fn completed() -> String {
    let a = call("fresh_a", r#"{"city":"Zürich 🦀"}"#);
    let b = call("fresh_b", "[null,[],{}]");
    [json!({"type":"response.output_item.added","output_index":0,"item":call("fresh_a","")}),
        json!({"type":"response.output_item.added","output_index":1,"item":call("fresh_b","")}),
        json!({"type":"response.function_call_arguments.delta","output_index":1,"item_id":"fc_fresh_b","delta":"[null,[],{}]"}),
        json!({"type":"response.output_item.done","output_index":1,"item":b}),
        json!({"type":"response.function_call_arguments.delta","output_index":0,"item_id":"fc_fresh_a","delta":"{\"city\":"}),
        json!({"type":"response.output_item.done","output_index":0,"item":a}),
        json!({"type":"response.completed","response":{"output":[a,b],
            "usage":{"input_tokens":9,"output_tokens":3,"input_tokens_details":{"cached_tokens":2}}}})]
        .into_iter().map(frame).collect()
}

fn correct(events: &[Delta]) {
    let mut calls: Vec<(String, String)> = Vec::new();
    for event in events {
        match event {
            Delta::ToolCallStart { id, .. } => calls.push((id.clone(), String::new())),
            Delta::ToolCallArgs(arguments) => calls
                .last_mut()
                .expect("current call")
                .1
                .push_str(arguments),
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
            .filter(|event| matches!(event, Delta::Stop(_)))
            .count(),
        1
    );
    assert_eq!(events.last(), Some(&Delta::Stop(StopReason::ToolUse)));
    assert!(events.iter().any(|event|matches!(event,Delta::Usage(usage) if usage.input==7 && usage.cache_read==2 && usage.output==3)));
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Delta::Signature(_)))
    );
    assert!(!format!("{events:?}").contains("abandoned"));
    assert!(!format!("{events:?}").contains("buffered"));
}

#[tokio::test]
async fn one_byte_http_chunks_preserve_interleaved_calls_unicode_and_completion() {
    for api in APIS {
        let mut steps = vec![headers(200)];
        steps.extend(
            completed()
                .as_bytes()
                .iter()
                .map(|byte| chunk(std::slice::from_ref(byte))),
        );
        steps.push(Step::Bytes(b"0\r\n\r\n".to_vec()));
        let server = Server::start(vec![steps]).await;
        let (provider, adapter) = (provider(api, &server.url), adapter(api));
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
}

#[tokio::test]
async fn retry_discards_both_the_active_and_buffered_output_slots() {
    for api in APIS {
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
        let (provider, adapter) = (provider(api, &server.url), adapter(api));
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
                    if matches!(&event,Delta::ToolCallStart {id,..} if id=="abandoned") {
                        delivered.notify_one();
                    }
                    events.borrow_mut().push(event);
                },
                |_| {
                    assert!(events.borrow().iter().any(
                        |event| matches!(event,Delta::ToolCallStart {id,..} if id=="abandoned")
                    ));
                    assert!(!events.borrow().iter().any(
                        |event| matches!(event,Delta::ToolCallStart {id,..} if id=="buffered")
                    ));
                    events.borrow_mut().clear();
                    retries.set(retries.get() + 1);
                },
            ),
        )
        .await
        .expect("retry deadline")
        .expect("retried stream");
        correct(&events.into_inner());
        assert_eq!(retries.get(), 1);
        assert_eq!(server.requests.load(Ordering::SeqCst), 2);
        let bodies = server.bodies.lock().expect("requests");
        assert_eq!(bodies[0], bodies[1]);
    }
}

#[tokio::test]
async fn cancellation_closes_the_stream_and_the_next_request_has_fresh_slots() {
    for api in APIS {
        let delivered = Arc::new(tokio::sync::Notify::new());
        let server = Server::start(vec![
            vec![headers(200), chunk(partial().as_bytes()), Step::Hold],
            reply(200, completed().as_bytes()),
        ])
        .await;
        let (provider, adapter) = (provider(api, &server.url), adapter(api));
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
            if matches!(&event,Delta::ToolCallStart {id,..} if id=="abandoned") {
                delivered.notify_one();
            }
            events.push(event);
        }));
        tokio::time::timeout(Duration::from_secs(5), async {
            tokio::select! {
                () = delivered.notified() => {},
                result = &mut pending => panic!("stream ended before cancellation: {result:?}"),
            }
        })
        .await
        .expect("delivery deadline");
        drop(pending);
        server.wait_for(&server.closed, 1).await;
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, Delta::Signature(_) | Delta::Stop(_)))
        );
        assert!(
            !events
                .iter()
                .any(|event| matches!(event,Delta::ToolCallStart {id,..} if id=="buffered"))
        );
        events.clear();
        tokio::time::timeout(
            Duration::from_secs(5),
            client.stream(&call, |event| events.push(event)),
        )
        .await
        .expect("fresh deadline")
        .expect("fresh stream");
        correct(&events);
        assert_eq!(server.requests.load(Ordering::SeqCst), 2);
    }
}
