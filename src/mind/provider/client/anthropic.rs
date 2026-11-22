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
        "api":"anthropic-messages","base_url":url,"auth":{"kind":"none"},
        "compat":{"anthropic_replay":true,"allow_empty_signature":true},
        "models":[{"id":"model","name":"Model","context_window":8000,"max_tokens":128,"reasoning":true}]}))
        .expect("provider");
    let adapter = LuaAdapter::new(
        engine_with_builtins().expect("engine"),
        "anthropic-messages",
    )
    .expect("adapter");
    (provider, adapter)
}

fn frame(name: &str, value: Value) -> String {
    format!("event: {name}\r\ndata: {value}\r\n\r\n")
}

fn start() -> String {
    frame(
        "message_start",
        json!({"message":{"role":"assistant","content":[],
        "usage":{"input_tokens":7,"cache_read_input_tokens":2,"output_tokens":0}}}),
    )
}

fn partial() -> String {
    start()
        + &frame(
            "content_block_start",
            json!({"index":0,
        "content_block":{"type":"tool_use","id":"abandoned","name":"echo","input":{}}}),
        )
        + &frame(
            "content_block_delta",
            json!({"index":0,"delta":{"type":"input_json_delta","partial_json":"{"}}),
        )
}

fn completed() -> String {
    let mut out = start();
    for (index, block) in [
        json!({"type":"thinking","thinking":"Zürich 🦀"}),
        json!({"type":"text","text":"hello"}),
        json!({"type":"tool_use","id":"fresh","name":"echo","input":{"a":[null,[],{}],"n":18446744073709551615u64}}),
    ].into_iter().enumerate() {
        out.push_str(&frame("content_block_start",json!({"index":index,"content_block":block})));
        out.push_str(&frame("content_block_stop",json!({"index":index})));
    }
    out + &frame(
        "message_delta",
        json!({"delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":3}}),
    ) + &frame("message_stop", json!({}))
}

fn correct(events: &[Delta]) {
    assert_eq!(
        events
            .iter()
            .filter(|d| matches!(d, Delta::ToolCallStart { .. }))
            .count(),
        1
    );
    assert!(
        events
            .iter()
            .any(|d| matches!(d,Delta::ToolCallStart { id, name } if id=="fresh" && name=="echo"))
    );
    let args: String = events
        .iter()
        .filter_map(|d| {
            if let Delta::ToolCallArgs(v) = d {
                Some(v.as_str())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        serde_json::from_str::<Value>(&args).expect("arguments"),
        json!({"a":[null,[],{}],"n":18446744073709551615u64})
    );
    assert_eq!(
        events
            .iter()
            .filter(|d| matches!(d, Delta::Signature(_)))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|d| matches!(d, Delta::Stop(_)))
            .count(),
        1
    );
    assert_eq!(events.last(), Some(&Delta::Stop(StopReason::ToolUse)));
    assert!(
        events
            .iter()
            .any(|d| matches!(d,Delta::Usage(u) if u.input==7 && u.cache_read==2 && u.output==3))
    );
    assert!(
        events
            .iter()
            .any(|d| matches!(d,Delta::Thinking(v) if v=="Zürich 🦀"))
    );
    assert!(!format!("{events:?}").contains("abandoned"));
}

#[tokio::test]
async fn single_byte_chunks_preserve_json_and_unicode_in_complete_assistant_state() {
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
            |d| events.push(d),
        ),
    )
    .await
    .expect("deadline")
    .expect("stream");
    correct(&events);
    assert_eq!(server.requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn retry_discards_partial_arguments_and_prior_replay_state() {
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
            |d| {
                if matches!(&d,Delta::ToolCallStart {id,..} if id=="abandoned") {
                    delivered.notify_one();
                }
                events.borrow_mut().push(d);
            },
            |_| {
                assert!(
                    events
                        .borrow()
                        .iter()
                        .any(|d| matches!(d,Delta::ToolCallStart {id,..} if id=="abandoned"))
                );
                assert!(
                    !events
                        .borrow()
                        .iter()
                        .any(|d| matches!(d, Delta::Signature(_) | Delta::Stop(_)))
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
async fn cancellation_closes_the_socket_and_drops_incomplete_continuation() {
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
    let mut pending = Box::pin(client.stream(&call, |d| {
        if matches!(&d,Delta::ToolCallStart {id,..} if id=="abandoned") {
            delivered.notify_one();
        }
        events.push(d);
    }));
    tokio::time::timeout(Duration::from_secs(5), async {
        tokio::select! {
            ()=delivered.notified()=>{},
            result=&mut pending=>panic!("ended before cancellation: {result:?}"),
        }
    })
    .await
    .expect("delivery");
    drop(pending);
    server.wait_for(&server.closed, 1).await;
    assert!(
        !events
            .iter()
            .any(|d| matches!(d, Delta::Signature(_) | Delta::Stop(_)))
    );
    events.clear();
    tokio::time::timeout(
        Duration::from_secs(5),
        client.stream(&call, |d| events.push(d)),
    )
    .await
    .expect("deadline")
    .expect("fresh stream");
    correct(&events);
    assert_eq!(server.requests.load(Ordering::SeqCst), 2);
}
