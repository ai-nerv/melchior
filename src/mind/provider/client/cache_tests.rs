use super::{Call, Client};
use crate::mind::lua::adapter::{LuaAdapter, engine_with_builtins};
use crate::mind::model::{Context, Message};
use crate::mind::provider::api::{Adapter, Delta, Options};
use crate::mind::provider::endpoint::Provider;
use crate::mind::provider::retry::RetryClass;
use crate::mind::provider::testing::{Server, reply};
use serde_json::json;
use std::sync::atomic::Ordering;
use std::time::Duration;

#[path = "cache_lifecycle.rs"]
mod lifecycle;

fn anthropic_answer(
    input: u64,
    output: u64,
    read: u64,
    write: u64,
) -> Vec<crate::mind::provider::testing::Step> {
    let events = [
        (
            "message_start",
            json!({"type":"message_start","message":{"id":"msg-fixture","type":"message","role":"assistant",
            "model":"fixture","content":[],"stop_reason":null,"stop_sequence":null,"usage":{
            "input_tokens":input,"output_tokens":0,"cache_read_input_tokens":read,"cache_creation_input_tokens":write}}}),
        ),
        (
            "content_block_start",
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
        ),
        (
            "content_block_delta",
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"synthetic answer"}}),
        ),
        (
            "content_block_stop",
            json!({"type":"content_block_stop","index":0}),
        ),
        (
            "message_delta",
            json!({"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":output}}),
        ),
        ("message_stop", json!({"type":"message_stop"})),
    ];
    let bytes: String = events
        .iter()
        .map(|(name, data)| format!("event: {name}\ndata: {data}\n\n"))
        .collect();
    reply(200, bytes.as_bytes())
}

#[tokio::test]
async fn consecutive_marked_http_requests_preserve_frames_and_supplied_cache_accounting() {
    let server = Server::start(vec![
        anthropic_answer(4, 2, 0, 30),
        anthropic_answer(7, 3, 30, 6),
    ])
    .await;
    let provider: Provider = serde_json::from_value(json!({"id":"fixture","name":"Fixture",
        "base_url":server.url,"api":"anthropic-messages","auth":{"kind":"none"},
        "compat":{"prompt_cache":"anthropic_ephemeral"},"models":[{"id":"fixture","name":"Fixture",
            "context_window":8000,"max_tokens":900,"cost":{"input":2,"output":8,"cache_read":0.25,"cache_write":2.5}}]})).expect("provider");
    let model = provider.models.first().expect("model");
    let adapter = LuaAdapter::new(
        engine_with_builtins().expect("builtins"),
        "anthropic-messages",
    )
    .expect("adapter");
    let mut context = Context {
        system: Some("unchanged rules\r\n".into()),
        messages: vec![Message::user("first")],
        tools: vec![crate::mind::model::Tool {
            name: "lookup".into(),
            description: "Lookup".into(),
            parameters: json!({"type":"object","properties":{},"required":[]}),
        }],
    };
    let options = Options::default();
    let client = Client::with_base_delay(Duration::ZERO);
    let mut expected = Vec::new();
    for (round, counts, price) in [(0, (4, 2, 0, 30), 99.0e-6), (1, (7, 3, 30, 6), 60.5e-6)] {
        expected.push(adapter.request(model, &context, &options));
        let mut deltas = Vec::new();
        let mut retries = Vec::new();
        let mut frames = Vec::new();
        let order = std::cell::RefCell::new(Vec::new());
        tokio::time::timeout(
            Duration::from_secs(3),
            client.stream_observing(
                &Call {
                    adapter: &adapter,
                    provider: &provider,
                    model,
                    context: &context,
                    options: &options,
                },
                |delta| {
                    order.borrow_mut().push("delta");
                    deltas.push(delta);
                },
                |retry| retries.push(retry),
                |frame| {
                    order.borrow_mut().push("frame");
                    frames.push(frame);
                },
            ),
        )
        .await
        .expect("bounded local stream")
        .expect("synthetic success");
        assert!(retries.is_empty());
        assert_eq!(order.borrow().first(), Some(&"frame"));
        assert_eq!(
            frames,
            [adapter
                .request_observed(model, &context, &options)
                .1
                .expect("frame")]
        );
        assert!(deltas.contains(&Delta::Text("synthetic answer".into())));
        assert!(deltas.contains(&Delta::Stop(crate::mind::model::StopReason::EndTurn)));
        let usage = deltas
            .iter()
            .rev()
            .find_map(|delta| match delta {
                Delta::Usage(usage) => Some(*usage),
                _ => None,
            })
            .expect("usage");
        assert_eq!(
            (
                usage.input,
                usage.output,
                usage.cache_read,
                usage.cache_write
            ),
            counts
        );
        assert!(
            (usage.price(model.cost) - price).abs() < 1e-12,
            "supplied-counter estimate round{round}"
        );
        context.messages.extend([
            Message::assistant("synthetic answer"),
            Message::user("next"),
        ]);
    }
    assert_eq!(server.requests.load(Ordering::SeqCst), 2);
    let captured: Vec<serde_json::Value> = server
        .bodies
        .lock()
        .expect("capture")
        .iter()
        .map(|body| serde_json::from_slice(body).expect("HTTP body JSON"))
        .collect();
    assert_eq!(captured, expected);
    for field in ["system", "tools"] {
        assert_eq!(captured[0][field], captured[1][field]);
    }
    assert_eq!(
        captured[0]["messages"][0]["content"][0]["cache_control"],
        json!({"type":"ephemeral"})
    );
    assert!(
        captured[1]["messages"][0]["content"][0]
            .get("cache_control")
            .is_none()
    );
    assert_eq!(
        captured[1]["messages"][1]["content"][0]["cache_control"],
        json!({"type":"ephemeral"})
    );
    assert_eq!(
        captured[1]["messages"][2]["content"][0]["cache_control"],
        json!({"type":"ephemeral"})
    );
}

#[tokio::test]
async fn an_explicit_cache_marker_rejection_is_sanitized_and_not_rewritten_or_retried() {
    for status in [400, 422] {
        let secret = "sk-fixture123456789012345678901234567890";
        let refusal = serde_json::to_vec(
            &json!({"error":{"message":format!("unsupported cache_control {secret}")}}),
        )
        .expect("refusal");
        let server = Server::start(vec![reply(status, &refusal)]).await;
        let provider: Provider = serde_json::from_value(json!({"id":"fixture","name":"Fixture",
            "base_url":server.url,"api":"anthropic-messages","auth":{"kind":"none"},
            "compat":{"prompt_cache":"anthropic_ephemeral"},
            "models":[{"id":"fixture","name":"Fixture","context_window":8000,"max_tokens":900}]}))
        .expect("provider");
        let model = provider.models.first().expect("model");
        let adapter = LuaAdapter::new(
            engine_with_builtins().expect("builtins"),
            "anthropic-messages",
        )
        .expect("adapter");
        let context = Context {
            system: Some("exact rules".into()),
            messages: vec![Message::user("exact user content")],
            ..Context::default()
        };
        let options = Options::default();
        let mut retries = Vec::new();
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            Client::with_base_delay(Duration::ZERO).stream_reporting(
                &Call {
                    adapter: &adapter,
                    provider: &provider,
                    model,
                    context: &context,
                    options: &options,
                },
                |_| {},
                |retry| retries.push(retry),
            ),
        )
        .await
        .expect("bounded rejection");
        let error = result.expect_err("unsupported markers");
        assert_eq!(error.class, RetryClass::Invalid);
        assert!(!error.again);
        assert!(error.message.contains("cache_control"));
        assert!(error.message.contains("<redacted>"));
        assert!(!error.message.contains(secret));
        assert!(retries.is_empty());
        assert_eq!(server.requests.load(Ordering::SeqCst), 1);
        let bodies = server.bodies.lock().expect("captured request");
        assert_eq!(bodies.len(), 1);
        let body: serde_json::Value =
            serde_json::from_slice(bodies.first().expect("one body")).expect("request JSON");
        assert_eq!(
            body["system"][0]["cache_control"],
            json!({"type":"ephemeral"})
        );
        assert_eq!(body["system"][0]["text"], "exact rules");
        assert_eq!(
            body["messages"][0]["content"][0]["text"],
            "exact user content"
        );
    }
}
