use super::support::{adapter, plain_model};
use crate::mind::model::{Content, Context, Message, Tool};
use crate::mind::provider::api::{Adapter, Options, Schema};
use crate::mind::provider::compat::{Compat, PromptCache};
use crate::mind::provider::model::Modality;
use serde_json::{Value, json};

pub(super) fn model(mode: Option<PromptCache>) -> crate::mind::provider::model::Model {
    let mut model = plain_model();
    model.input.push(Modality::Image);
    model.compat = Some(Compat {
        prompt_cache: mode,
        ..Compat::default()
    });
    model
}

pub(super) fn context() -> Context {
    Context {
        system: Some("Exact system\r\n".into()),
        messages: vec![Message::user("hi")],
        tools: vec![
            Tool {
                name: "first".into(),
                description: "First".into(),
                parameters: json!({"type":"object"}),
            },
            Tool {
                name: "last".into(),
                description: "Last".into(),
                parameters: json!({"type":"object","required":[],
            "properties":{},"cache_control":{"type":"literal-schema-data"}}),
            },
        ],
    }
}

pub(super) fn request(context: &Context, options: &Options, mode: Option<PromptCache>) -> Value {
    adapter("anthropic-messages").request(&model(mode), context, options)
}

pub(super) fn projected(mut body: Value) -> Value {
    if let Some(system) = body.get("system").and_then(Value::as_array) {
        assert_eq!(system.len(), 1);
        assert_eq!(system[0]["type"], "text");
        assert_eq!(system[0]["cache_control"], json!({"type":"ephemeral"}));
        assert_eq!(system[0].as_object().expect("system block").len(), 3);
        body["system"] = system[0]["text"].clone();
    }
    if let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) {
        let last = tools.len().saturating_sub(1);
        for (index, tool) in tools.iter_mut().enumerate() {
            let marker = tool.as_object_mut().expect("tool").remove("cache_control");
            if index == last {
                assert_eq!(marker, Some(json!({"type":"ephemeral"})));
            } else {
                assert_eq!(marker, None);
            }
        }
    }
    if let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) {
        for message in messages {
            let role = message["role"].as_str().expect("role").to_owned();
            for block in message["content"].as_array_mut().expect("blocks") {
                if let Some(marker) = block
                    .as_object_mut()
                    .expect("block")
                    .remove("cache_control")
                {
                    assert_eq!(marker, json!({"type":"ephemeral"}));
                    let eligible = match block["type"].as_str().expect("type") {
                        "text" => block["text"].as_str().is_some_and(|text| !text.is_empty()),
                        "image" | "tool_result" => role == "user",
                        "tool_use" => role == "assistant",
                        _ => false,
                    };
                    assert!(eligible, "ineligible marked block: {block}");
                }
            }
        }
    }
    body
}

#[test]
fn supported_system_and_final_tool_markers_match_the_actual_wire_golden() {
    let body = request(
        &context(),
        &Options::default(),
        Some(PromptCache::AnthropicEphemeral),
    );
    assert_eq!(
        body,
        json!({"model":"m-1","stream":true,"max_tokens":8192,
        "system":[{"type":"text","text":"Exact system\r\n","cache_control":{"type":"ephemeral"}}],
        "tools":[{"name":"first","description":"First","input_schema":{"type":"object"}},
            {"name":"last","description":"Last","input_schema":{"type":"object","required":[],"properties":{},
                "cache_control":{"type":"literal-schema-data"}},"cache_control":{"type":"ephemeral"}}],
        "messages":[{"role":"user","content":[{"type":"text","text":"hi","cache_control":{"type":"ephemeral"}}]}]})
    );
}

#[test]
fn explicit_disable_and_absence_keep_the_request_bytes_equal() {
    let context = context();
    let implicit = request(&context, &Options::default(), None);
    let disabled = request(&context, &Options::default(), Some(PromptCache::None));
    assert_eq!(
        serde_json::to_vec(&implicit).expect("implicit"),
        serde_json::to_vec(&disabled).expect("disabled")
    );
    assert_eq!(implicit["system"], "Exact system\r\n");
    assert!(implicit["tools"][1].get("cache_control").is_none());
    assert_eq!(
        implicit["tools"][1]["input_schema"]["cache_control"]["type"],
        "literal-schema-data"
    );
}

#[test]
fn absent_empty_and_nonempty_systems_and_tools_select_only_eligible_targets() {
    for system in [None, Some(""), Some(" "), Some("rules")] {
        for count in 0..=2 {
            let mut context = context();
            context.system = system.map(str::to_owned);
            context.tools.truncate(count);
            let enabled = request(
                &context,
                &Options::default(),
                Some(PromptCache::AnthropicEphemeral),
            );
            let off = request(&context, &Options::default(), None);
            assert_eq!(projected(enabled.clone()), off);
            match system {
                Some("") => assert_eq!(enabled["system"], ""),
                None => assert!(enabled.get("system").is_none()),
                Some(_) => assert!(enabled["system"].is_array()),
            }
            if count == 0 {
                assert!(enabled.get("tools").is_none());
            }
            assert!(enabled.get("cache_control").is_none());
        }
    }
}

#[test]
fn forced_answer_schema_marks_the_selected_tool_not_the_discarded_inventory() {
    let schema = json!({"type":"object","properties":{},"required":[],"default":null,
        "cache_control":{"type":"literal-forced-schema"}});
    let options = Options {
        schema: Some(Schema {
            name: "answer".into(),
            schema: schema.clone(),
        }),
        ..Options::default()
    };
    let on = request(&context(), &options, Some(PromptCache::AnthropicEphemeral));
    assert_eq!(on["tools"].as_array().expect("tools").len(), 1);
    assert_eq!(on["tools"][0]["name"], "answer");
    assert_eq!(on["tools"][0]["input_schema"], schema);
    assert_eq!(on["tools"][0]["cache_control"], json!({"type":"ephemeral"}));
    assert_eq!(projected(on), request(&context(), &options, None));
}

#[test]
fn marker_projection_preserves_actual_signed_blocks_ids_images_and_literal_payloads() {
    let mut context = context();
    context.messages[0].content.push(Content::Image {
        data: "image-bytes".into(),
        media_type: "image/png".into(),
    });
    let mut assistant = Message::assistant("");
    assistant.content = vec![
        Content::Thinking {
            thinking: "unsigned".into(),
            signature: None,
        },
        Content::Thinking {
            thinking: "signed\n".into(),
            signature: Some("exact-signature".into()),
        },
        Content::ToolCall {
            id: "same-id".into(),
            name: "last".into(),
            arguments: json!({"a":[null,[],{}],
            "cache_control":{"type":"literal-arguments"},"big":u64::MAX}),
            thought_signature: None,
        },
    ];
    context.messages.push(assistant);
    context.messages.push(Message {
        role: crate::mind::model::Role::Tool,
        content: vec![Content::ToolResult {
            id: "same-id".into(),
            name: "last".into(),
            content: "exact result\r\n".into(),
            is_error: false,
            images: vec![crate::mind::model::Image {
                data: "result-image-bytes".into(),
                media_type: "image/png".into(),
            }],
        }],
        stop_reason: None,
        usage: None,
        error: None,
    });
    let mut tail = Message::assistant("");
    tail.content = vec![Content::Thinking {
        thinking: "thinking-only tail".into(),
        signature: Some("tail-signature".into()),
    }];
    context.messages.push(tail);
    let off = request(&context, &Options::default(), None);
    let on = request(
        &context,
        &Options::default(),
        Some(PromptCache::AnthropicEphemeral),
    );
    let projected = projected(on);
    assert_eq!(projected["messages"], off["messages"]);
    assert_eq!(projected, off);
}

#[test]
fn repeated_and_appended_requests_keep_the_marked_system_and_tool_frames_stable() {
    let wire = adapter("anthropic-messages");
    let model = model(Some(PromptCache::AnthropicEphemeral));
    let mut context = context();
    context.messages.push(Message::assistant("answer"));
    let first = wire.request(&model, &context, &Options::default());
    let initial_content = projected(first.clone());
    assert_eq!(
        serde_json::to_vec(&first).expect("first request"),
        serde_json::to_vec(&wire.request(&model, &context, &Options::default()))
            .expect("repeated request")
    );
    for round in 1..=3 {
        context.messages.extend([
            Message::user(format!("user{round}")),
            Message::assistant(format!("answer{round}")),
        ]);
        let next = wire.request(&model, &context, &Options::default());
        for field in ["system", "tools"] {
            assert_eq!(
                serde_json::to_vec(&next[field]).expect("next"),
                serde_json::to_vec(&first[field]).expect("first")
            );
        }
        assert_eq!(
            projected(next)["messages"]
                .as_array()
                .expect("messages")
                .get(..2),
            initial_content["messages"].as_array().map(Vec::as_slice)
        );
    }
}

#[test]
fn cache_capability_does_not_add_fields_to_other_protocols_or_headers() {
    for protocol in [
        "openai-completions",
        "openai-responses",
        "azure-openai-responses",
        "openai-codex-responses",
        "google-generative-ai",
        "google-vertex",
        "pi-messages",
    ] {
        let wire = adapter(protocol);
        assert_eq!(
            wire.request(&model(None), &context(), &Options::default()),
            wire.request(
                &model(Some(PromptCache::AnthropicEphemeral)),
                &context(),
                &Options::default()
            )
        );
    }
    assert_eq!(
        adapter("anthropic-messages").headers(Some("synthetic")),
        vec![
            ("content-type".into(), "application/json".into()),
            ("anthropic-version".into(), "2023-06-01".into()),
            ("x-api-key".into(), "synthetic".into())
        ]
    );
}
