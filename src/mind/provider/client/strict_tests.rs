use super::*;
use crate::mind::provider::testing::{Server, reply};
use crate::mind::{
    lua::adapter::{LuaAdapter, engine_with_builtins},
    model::{Message, Tool, ToolStrictness},
};
use serde_json::{Value, json};
use std::sync::atomic::Ordering;
use std::time::Duration;

const LIMIT: &str = "Too many strict tools (21)";

fn provider(url: &str) -> Provider {
    serde_json::from_value(
        json!({"id":"fixture","name":"Fixture","api":"anthropic-messages",
        "base_url":url,"auth":{"kind":"none"},"compat":{"supports_strict_tools":true},
        "models":[{"id":"m","name":"M","context_window":8192,"max_tokens":1024}]}),
    )
    .expect("provider")
}

fn context(policy: ToolStrictness) -> Context {
    Context {
        messages: vec![Message::user("lookup")],
        tools: vec![Tool {
            name: "lookup".into(),
            description: "Lookup".into(),
            strict: Some(policy),
            parameters: json!({"type":"object","properties":{"term":{"type":"string"}}}),
        }],
        ..Default::default()
    }
}

fn error(kind: &str, message: &str) -> Vec<crate::mind::provider::testing::Step> {
    reply(
        400,
        json!({"type":"error","error":{"type":kind,"message":message}})
            .to_string()
            .as_bytes(),
    )
}

fn success() -> Vec<crate::mind::provider::testing::Step> {
    reply(200, b"event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n")
}

#[tokio::test]
async fn only_preferred_tools_are_downgraded_after_a_classified_pre_stream_rejection() {
    let server = Server::start(vec![error("invalid_request_error", LIMIT), success()]).await;
    let provider = provider(&server.url);
    let adapter = LuaAdapter::new(
        engine_with_builtins().expect("builtins"),
        "anthropic-messages",
    )
    .expect("adapter");
    let mut context = context(ToolStrictness::Prefer);
    let mut required = context.tools[0].clone();
    required.name = "required".into();
    required.strict = Some(ToolStrictness::Require);
    context.tools.push(required);
    let mut retries = Vec::new();
    tokio::time::timeout(
        Duration::from_secs(3),
        Client::with_base_delay(Duration::ZERO).stream_reporting(
            &Call {
                adapter: &adapter,
                provider: &provider,
                model: &provider.models[0],
                context: &context,
                options: &Options::default(),
            },
            |_| {},
            |retry| retries.push(retry),
        ),
    )
    .await
    .expect("deadline")
    .expect("fallback succeeds");
    assert_eq!(server.requests.load(Ordering::SeqCst), 2);
    assert_eq!(retries.len(), 1);
    assert_eq!(retries[0].delay, Duration::ZERO);
    let bodies = server.bodies.lock().expect("bodies");
    let first: Value = serde_json::from_slice(&bodies[0]).expect("first");
    let second: Value = serde_json::from_slice(&bodies[1]).expect("second");
    assert_eq!(first["tools"][0]["strict"], true);
    assert!(second["tools"][0].get("strict").is_none());
    assert_eq!(
        second["tools"][0]["input_schema"],
        context.tools[0].parameters
    );
    assert_eq!(first["tools"][1], second["tools"][1]);
    assert_eq!(second["tools"][1]["strict"], true);
    assert_eq!(context.tools[0].strict, Some(ToolStrictness::Prefer));
}

#[tokio::test]
async fn required_tools_unrelated_errors_and_a_second_limit_error_are_not_downgraded() {
    for (policy, kind, message, expected) in [
        (ToolStrictness::Require, "invalid_request_error", LIMIT, 1),
        (ToolStrictness::Prefer, "authentication_error", LIMIT, 1),
        (
            ToolStrictness::Prefer,
            "invalid_request_error",
            "unsupported field",
            1,
        ),
        (ToolStrictness::Prefer, "invalid_request_error", LIMIT, 2),
    ] {
        let server = Server::start(vec![error(kind, message), error(kind, message)]).await;
        let provider = provider(&server.url);
        let adapter = LuaAdapter::new(
            engine_with_builtins().expect("builtins"),
            "anthropic-messages",
        )
        .expect("adapter");
        let mut retries = 0;
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            Client::with_base_delay(Duration::ZERO).stream_reporting(
                &Call {
                    adapter: &adapter,
                    provider: &provider,
                    model: &provider.models[0],
                    context: &context(policy),
                    options: &Options::default(),
                },
                |_| {},
                |_| retries += 1,
            ),
        )
        .await
        .expect("deadline");
        assert_eq!(
            result.expect_err("terminal refusal").class,
            RetryClass::Invalid
        );
        assert_eq!(server.requests.load(Ordering::SeqCst), expected);
        assert_eq!(retries, expected - 1);
    }
}

#[test]
fn fallback_classification_is_protocol_status_type_and_message_specific() {
    use crate::mind::provider::{model::Api, strict::limit_error};
    let body = json!({"error":{"type":"invalid_request_error","message":LIMIT}}).to_string();
    assert!(limit_error(Api::AnthropicMessages, 400, &body));
    for (api, status) in [
        (Api::OpenAiCompletions, 400),
        (Api::AnthropicMessages, 401),
        (Api::AnthropicMessages, 500),
    ] {
        assert!(!limit_error(api, status, &body));
    }
    assert!(!limit_error(Api::AnthropicMessages, 400, LIMIT));
}

#[tokio::test]
async fn an_error_inside_a_started_stream_does_not_disable_preferred_strict_tools() {
    let bytes = format!(
        "event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":\"partial\"}}}}\n\nevent: error\ndata: {{\"type\":\"error\",\"error\":{{\"type\":\"invalid_request_error\",\"message\":\"{LIMIT}\"}}}}\n\n"
    );
    let server = Server::start((0..4).map(|_| reply(200, bytes.as_bytes())).collect()).await;
    let provider = provider(&server.url);
    let adapter = LuaAdapter::new(
        engine_with_builtins().expect("builtins"),
        "anthropic-messages",
    )
    .expect("adapter");
    let mut text = String::new();
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        Client::with_base_delay(Duration::ZERO).stream(
            &Call {
                adapter: &adapter,
                provider: &provider,
                model: &provider.models[0],
                context: &context(ToolStrictness::Prefer),
                options: &Options::default(),
            },
            |delta| {
                if let Delta::Text(chunk) = delta {
                    text.push_str(&chunk)
                }
            },
        ),
    )
    .await
    .expect("deadline");
    assert!(result.is_err());
    assert!(text.contains("partial"));
    let bodies = server.bodies.lock().expect("bodies");
    assert!(!bodies.is_empty());
    for bytes in bodies.iter() {
        let body: Value = serde_json::from_slice(bytes).expect("request");
        assert_eq!(body["tools"][0]["strict"], true);
    }
}
