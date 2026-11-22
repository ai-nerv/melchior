use super::*;
use crate::mind::{
    lua::adapter::{LuaAdapter, engine_with_builtins},
    model::{Content, Context, Image, Message, Role},
    provider::{
        api::{Adapter, Options},
        client::{Call, Client, budget::Prepared},
        endpoint::Provider,
        retry::RetryClass,
        testing::{Server, reply},
    },
};
use serde_json::json;
use std::sync::atomic::Ordering;
use std::time::Duration;

fn limits(value: Value) -> InputLimits {
    serde_json::from_value(value).expect("limits")
}

fn provider(api: Api, url: &str) -> Provider {
    serde_json::from_value(json!({"id":"fixture","name":"Fixture","api":api,
        "base_url":url,"auth":{"kind":"none"},"models":[{"id":"m","name":"M",
        "input":["text","image"],"context_window":8192,"max_tokens":1024}]}))
    .expect("provider")
}

fn image() -> Content {
    Content::Image {
        data: "YWJj".into(),
        media_type: "image/png".into(),
    }
}

fn conversation() -> Context {
    let mut user = Message::user("image");
    user.content.push(image());
    let mut call = Message::assistant("");
    call.content.push(Content::ToolCall {
        id: "one".into(),
        name: "read".into(),
        arguments: json!({"type":"image","source":{"data":"not-an-image"}}),
        thought_signature: None,
    });
    let tool = Message {
        role: Role::Tool,
        content: vec![Content::ToolResult {
            id: "one".into(),
            name: "read".into(),
            content: "tool image".into(),
            is_error: false,
            images: vec![Image {
                data: "YWJj".into(),
                media_type: "image/png".into(),
            }],
        }],
        stop_reason: None,
        usage: None,
        error: None,
    };
    Context {
        messages: vec![user, call, tool],
        ..Default::default()
    }
}

#[test]
fn final_json_byte_limit_counts_utf8_and_escaping_exactly() {
    let body = json!({"messages":[{"role":"user","content":"é🦀\n\"private-sentinel"}]});
    let size = serde_json::to_vec(&body).expect("JSON").len();
    for (maximum, accepted) in [(size - 1, false), (size, true), (size + 1, true)] {
        assert_eq!(
            check(
                Api::OpenAiCompletions,
                &limits(json!({"max_request_bytes":maximum})),
                &body
            )
            .is_ok(),
            accepted
        );
    }
    let mut writer = Remaining(1);
    assert!(writer.write_all(b"ab").is_err());
    assert_eq!(writer.0, 1);
}

#[test]
fn real_adapters_count_user_and_tool_images_without_mutating_history() {
    let context = conversation();
    let original = context.clone();
    for api in [
        Api::OpenAiCompletions,
        Api::MistralConversations,
        Api::OpenAiResponses,
        Api::AzureOpenAiResponses,
        Api::OpenAiCodexResponses,
        Api::AnthropicMessages,
        Api::GoogleGenerativeAi,
        Api::GoogleVertex,
        Api::PiMessages,
    ] {
        let provider = provider(api, "http://127.0.0.1:1");
        let adapter = LuaAdapter::new(engine_with_builtins().expect("engine"), api.as_str())
            .expect("adapter");
        let body = adapter.request(&provider.models[0], &context, &Options::default());
        let legal = limits(
            json!({"images":{"max_per_message":1,"max_per_request":2,"resize":{"max_bytes":4}}}),
        );
        assert_eq!(check(api, &legal, &body), Ok(()), "{api:?}: {body}");
        for rejected in [
            json!({"images":{"max_per_request":1}}),
            json!({"images":{"resize":{"max_bytes":3}}}),
        ] {
            assert!(check(api, &limits(rejected), &body).is_err(), "{api:?}");
        }
    }
    assert_eq!(context, original);
}

#[test]
fn anthropic_limits_apply_after_messages_merge_not_to_neutral_message_boundaries() {
    let mut provider = provider(Api::AnthropicMessages, "http://127.0.0.1:1");
    let adapter = LuaAdapter::new(
        engine_with_builtins().expect("engine"),
        "anthropic-messages",
    )
    .expect("adapter");
    let user = Message {
        content: vec![image()],
        ..Message::user("")
    };
    let context = Context {
        messages: vec![user.clone(), user],
        ..Default::default()
    };
    let body = adapter.request(&provider.models[0], &context, &Options::default());
    assert_eq!(body["messages"].as_array().expect("messages").len(), 1);
    provider.models[0].input_limits = Some(limits(
        json!({"images":{"max_per_message":1,"max_per_request":2}}),
    ));
    let options = Options::default();
    let call = Call {
        adapter: &adapter,
        provider: &provider,
        model: &provider.models[0],
        context: &context,
        options: &options,
    };
    let error = Prepared::new(&call, &options)
        .err()
        .expect("merged message refused");
    assert_eq!(error.class, RetryClass::Invalid);
    assert!(!error.again);
    assert_eq!(
        error.message,
        "provider message exceeds model image count limit"
    );
}

#[test]
fn tool_image_limits_follow_each_protocols_actual_message_grouping() {
    let mut context = conversation();
    let mut second = context.messages[2].clone();
    if let Content::ToolResult { id, .. } = &mut second.content[0] {
        *id = "two".into();
    }
    context.messages[1].content.push(Content::ToolCall {
        id: "two".into(),
        name: "read".into(),
        arguments: json!({}),
        thought_signature: None,
    });
    context.messages.push(second);
    for (api, per_message) in [
        (Api::OpenAiCompletions, 2),
        (Api::AnthropicMessages, 2),
        (Api::GoogleGenerativeAi, 2),
        (Api::OpenAiResponses, 1),
        (Api::PiMessages, 1),
    ] {
        let provider = provider(api, "http://127.0.0.1:1");
        let adapter = LuaAdapter::new(engine_with_builtins().expect("engine"), api.as_str())
            .expect("adapter");
        let body = adapter.request(&provider.models[0], &context, &Options::default());
        assert_eq!(
            check(
                api,
                &limits(json!({"images":{"max_per_message":per_message,"max_per_request":3}})),
                &body
            ),
            Ok(()),
            "{api:?}: {body}"
        );
        assert!(
            check(api, &limits(json!({"images":{"max_per_request":2}})), &body).is_err(),
            "{api:?}"
        );
        assert_eq!(
            check(api, &limits(json!({"images":{"max_per_message":1}})), &body).is_err(),
            per_message > 1,
            "{api:?}"
        );
    }
}

#[test]
fn payload_limits_do_not_fetch_remote_urls_or_traverse_tool_arguments() {
    for (api, body) in [
        (
            Api::OpenAiCompletions,
            json!({"messages":[{"content":[{"type":"image_url","image_url":{"url":"https://private-sentinel/image.png"}}]}]}),
        ),
        (
            Api::OpenAiResponses,
            json!({"input":[{"content":[{"type":"input_image","file_id":"private-sentinel"}]}]}),
        ),
        (
            Api::GoogleGenerativeAi,
            json!({"contents":[{"parts":[{"fileData":{"mimeType":"image/png","fileUri":"private-sentinel"}}]}]}),
        ),
    ] {
        assert!(check(api, &limits(json!({"images":{"max_per_request":1}})), &body).is_ok());
        let error = check(
            api,
            &limits(json!({"images":{"resize":{"max_bytes":4}}})),
            &body,
        )
        .expect_err("unknown size");
        assert!(!error.contains("private-sentinel"));
    }
    let body = json!({"messages":[{"role":"assistant","tool_calls":[{"arguments":{"type":"image_url","image_url":{"url":"private-sentinel"}}}]}]});
    assert!(
        check(
            Api::OpenAiCompletions,
            &limits(json!({"images":{"resize":{"max_bytes":1}}})),
            &body
        )
        .is_ok()
    );
    assert!(
        check(
            Api::BedrockConverseStream,
            &limits(json!({"images":{"max_per_request":1}})),
            &json!({})
        )
        .is_err()
    );
}

#[test]
fn byte_limit_excludes_internal_scratch_and_budget_and_text_only_omission_precedes_image_limits() {
    let mut provider = provider(Api::OpenAiCompletions, "http://127.0.0.1:1");
    let adapter = LuaAdapter::new(
        engine_with_builtins().expect("engine"),
        "openai-completions",
    )
    .expect("adapter");
    provider.models[0].input = vec![crate::mind::provider::model::Modality::Text];
    let context = conversation();
    let options = Options::default();
    let make = |provider: &Provider| {
        Prepared::new(
            &Call {
                adapter: &adapter,
                provider,
                model: &provider.models[0],
                context: &context,
                options: &options,
            },
            &options,
        )
        .expect("prepared")
    };
    let before = make(&provider);
    assert!(before.body.get("__budget").is_none());
    assert!(before.body.get("__scratch").is_none());
    let size = serde_json::to_vec(&before.body).expect("bytes").len();
    provider.models[0].input_limits = Some(limits(
        json!({"max_request_bytes":size,"images":{"max_per_request":1,"resize":{"max_bytes":1}}}),
    ));
    assert_eq!(make(&provider).body, before.body);
}

#[tokio::test]
async fn actual_http_body_matches_exact_preflight_byte_boundary() {
    let server = Server::start(vec![reply(
        200,
        b"data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n",
    )])
    .await;
    let mut provider = provider(Api::OpenAiCompletions, &server.url);
    let adapter = LuaAdapter::new(
        engine_with_builtins().expect("engine"),
        "openai-completions",
    )
    .expect("adapter");
    let context = Context {
        messages: vec![Message::user("é🦀\n\"")],
        ..Default::default()
    };
    let options = Options::default();
    let body = Prepared::new(
        &Call {
            adapter: &adapter,
            provider: &provider,
            model: &provider.models[0],
            context: &context,
            options: &options,
        },
        &options,
    )
    .expect("prepared")
    .body;
    let bytes = serde_json::to_vec(&body).expect("bytes");
    for (maximum, accepted) in [(bytes.len() - 1, false), (bytes.len(), true)] {
        provider.models[0].input_limits = Some(limits(json!({"max_request_bytes":maximum})));
        let mut retries = 0;
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            Client::with_base_delay(Duration::ZERO).stream_reporting(
                &Call {
                    adapter: &adapter,
                    provider: &provider,
                    model: &provider.models[0],
                    context: &context,
                    options: &options,
                },
                |_| {},
                |_| retries += 1,
            ),
        )
        .await
        .expect("deadline");
        assert_eq!(result.is_ok(), accepted, "{result:?}");
        assert_eq!(retries, 0);
        assert_eq!(
            server.requests.load(Ordering::SeqCst),
            usize::from(accepted)
        );
    }
    assert_eq!(server.bodies.lock().expect("bodies")[0], bytes);
}
