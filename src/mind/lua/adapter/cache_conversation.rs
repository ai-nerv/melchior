use super::cache_markers::{context, model, projected, request};
use super::support::adapter;
use crate::mind::model::{Content, Context, Image, Message, Role};
use crate::mind::provider::api::{Adapter, Options, Schema};
use crate::mind::provider::compat::PromptCache;
use serde_json::{Value, json};

fn enabled(context: &Context) -> Value {
    request(
        context,
        &Options::default(),
        Some(PromptCache::AnthropicEphemeral),
    )
}

fn locations(body: &Value) -> Vec<(usize, usize)> {
    let mut locations = Vec::new();
    for (message, value) in body["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .enumerate()
    {
        for (block, value) in value["content"]
            .as_array()
            .expect("content")
            .iter()
            .enumerate()
        {
            if let Some(marker) = value.get("cache_control") {
                assert_eq!(marker, &json!({"type":"ephemeral"}));
                locations.push((message, block));
            }
        }
    }
    locations
}

fn thinking(signature: Option<&str>) -> Content {
    Content::Thinking {
        thinking: "reasoning\r\n".into(),
        signature: signature.map(str::to_owned),
    }
}

fn assistant(content: Vec<Content>) -> Message {
    let mut message = Message::assistant("");
    message.content = content;
    message
}

fn result(id: &str, images: Vec<Image>) -> Message {
    Message {
        role: Role::Tool,
        content: vec![Content::ToolResult {
            id: id.into(),
            name: "last".into(),
            content: "literal cache_control {\"type\":\"ephemeral\"}\r\n".into(),
            is_error: false,
            images,
        }],
        stop_reason: None,
        usage: None,
        error: None,
    }
}

#[test]
fn exact_wire_golden_marks_the_last_blocks_of_only_the_two_latest_messages() {
    let context = Context {
        system: None,
        tools: Vec::new(),
        messages: vec![
            Message::user("one"),
            Message::assistant("two"),
            Message::user("three"),
            Message::assistant("four"),
        ],
    };
    let body = enabled(&context);
    assert_eq!(
        body,
        json!({"model":"m-1","stream":true,"max_tokens":8192,"messages":[
            {"role":"user","content":[{"type":"text","text":"one"}]},
            {"role":"assistant","content":[{"type":"text","text":"two"}]},
            {"role":"user","content":[{"type":"text","text":"three","cache_control":{"type":"ephemeral"}}]},
            {"role":"assistant","content":[{"type":"text","text":"four","cache_control":{"type":"ephemeral"}}]}
        ]})
    );
    assert_eq!(locations(&body), vec![(2, 0), (3, 0)]);
    assert_eq!(
        projected(body),
        request(&context, &Options::default(), None)
    );
}

#[test]
fn selection_occurs_after_same_role_merge_and_never_adds_or_moves_content() {
    let context = Context {
        system: None,
        tools: Vec::new(),
        messages: vec![
            Message::user("a"),
            Message::user("b"),
            Message::assistant("c"),
            Message::assistant("d"),
            Message::user("e"),
            Message::user("f"),
        ],
    };
    let body = enabled(&context);
    assert_eq!(locations(&body), vec![(1, 1), (2, 1)]);
    assert_eq!(
        body["messages"].as_array().expect("merged messages").len(),
        3
    );
    assert_eq!(
        projected(body),
        request(&context, &Options::default(), None)
    );
}

#[test]
fn thinking_only_and_empty_tails_never_receive_direct_markers() {
    let context = Context {
        system: None,
        tools: Vec::new(),
        messages: vec![
            Message::user("a"),
            Message::assistant("b"),
            Message::user("c"),
            assistant(vec![thinking(Some("signed-tail"))]),
            Message::user(""),
            assistant(vec![thinking(None)]),
        ],
    };
    let body = enabled(&context);
    assert_eq!(locations(&body), vec![(1, 0), (2, 0)]);
    assert_eq!(
        body["messages"][3]["content"][0]["signature"],
        "signed-tail"
    );
    assert!(
        body["messages"][3]["content"][0]
            .get("cache_control")
            .is_none()
    );
    assert_eq!(
        projected(body),
        request(&context, &Options::default(), None)
    );
    for messages in [
        vec![],
        vec![Message::user("")],
        vec![assistant(vec![thinking(Some("only-signed"))])],
        vec![assistant(vec![thinking(None)])],
    ] {
        let context = Context {
            messages,
            ..Context::default()
        };
        assert!(locations(&enabled(&context)).is_empty());
    }
}

#[test]
fn text_before_a_signed_thinking_tail_is_eligible_but_the_thinking_is_unchanged() {
    let context = Context {
        messages: vec![
            Message::user("user"),
            assistant(vec![
                Content::Text {
                    text: "answer".into(),
                    signature: None,
                },
                thinking(Some("signature")),
            ]),
        ],
        ..Context::default()
    };
    let body = enabled(&context);
    assert_eq!(locations(&body), vec![(0, 0), (1, 0)]);
    assert_eq!(
        body["messages"][1]["content"][1],
        json!({"type":"thinking","thinking":"reasoning\r\n","signature":"signature"})
    );
    assert_eq!(
        projected(body),
        request(&context, &Options::default(), None)
    );
}

#[test]
fn parallel_tool_calls_and_results_keep_exact_units_signatures_and_argument_values() {
    let args =
        json!({"cache_control":{"type":"literal-arguments"},"nested":[null,[],{}],"big":u64::MAX});
    let context = Context {
        messages: vec![
            Message::user("calls"),
            assistant(vec![
                thinking(Some("tool-signature")),
                Content::ToolCall {
                    id: "id-one".into(),
                    name: "first".into(),
                    arguments: args.clone(),
                    thought_signature: None,
                },
                Content::ToolCall {
                    id: "id-two".into(),
                    name: "last".into(),
                    arguments: args.clone(),
                    thought_signature: None,
                },
            ]),
            result("id-one", vec![]),
            result(
                "id-two",
                vec![Image {
                    data: "result-image".into(),
                    media_type: "image/png".into(),
                }],
            ),
        ],
        ..Context::default()
    };
    let body = enabled(&context);
    assert_eq!(locations(&body), vec![(1, 2), (2, 1)]);
    assert_eq!(body["messages"][1]["content"][1]["input"], args);
    assert_eq!(body["messages"][1]["content"][2]["input"], args);
    assert_eq!(body["messages"][2]["content"][0]["tool_use_id"], "id-one");
    assert_eq!(body["messages"][2]["content"][1]["tool_use_id"], "id-two");
    assert!(
        body["messages"][2]["content"][1]["content"][1]
            .get("cache_control")
            .is_none()
    );
    assert_eq!(
        projected(body),
        request(&context, &Options::default(), None)
    );
}

#[test]
fn user_images_are_eligible_without_marking_assistant_images_or_nested_sources() {
    let image = Content::Image {
        data: "opaque-image".into(),
        media_type: "image/png".into(),
    };
    let mut user = Message::user("inspect");
    user.content.push(image.clone());
    let context = Context {
        messages: vec![user, assistant(vec![image])],
        ..Context::default()
    };
    let body = enabled(&context);
    assert_eq!(locations(&body), vec![(0, 1)]);
    assert_eq!(
        body["messages"][0]["content"][1]["source"]["data"],
        "opaque-image"
    );
    assert!(
        body["messages"][0]["content"][1]["source"]
            .get("cache_control")
            .is_none()
    );
    assert_eq!(
        projected(body),
        request(&context, &Options::default(), None)
    );
}

#[test]
fn marker_budget_is_bounded_for_every_system_tool_and_forced_schema_combination() {
    for system in [None, Some(""), Some("rules")] {
        for tools in 0..=2 {
            for forced in [false, true] {
                let mut context = context();
                context.system = system.map(str::to_owned);
                context.tools.truncate(tools);
                for turn in 0..40 {
                    context.messages.extend([
                        Message::assistant(format!("assistant{turn}")),
                        Message::user(format!("user{turn}")),
                    ]);
                }
                let options = Options {
                    schema: forced.then(|| Schema {
                        name: "answer".into(),
                        schema: json!({"type":"object","required":[]}),
                    }),
                    ..Options::default()
                };
                let body = request(&context, &options, Some(PromptCache::AnthropicEphemeral));
                assert_eq!(locations(&body), vec![(79, 0), (80, 0)]);
                let structural = usize::from(system.is_some_and(|s| !s.is_empty()))
                    + usize::from(forced || tools > 0);
                assert!(locations(&body).len() + structural <= 4);
                assert!(body.get("cache_control").is_none());
                assert_eq!(projected(body), request(&context, &options, None));
            }
        }
    }
}

#[test]
fn repeated_and_appended_requests_preserve_exact_normalized_prefixes_and_input() {
    let wire = adapter("anthropic-messages");
    let model = model(Some(PromptCache::AnthropicEphemeral));
    let mut context = context();
    context.messages.push(Message::assistant("initial answer"));
    for round in 0..5 {
        let original = serde_json::to_value(&context).expect("original context");
        let previous = wire.request(&model, &context, &Options::default());
        let repeated = wire.request(&model, &context, &Options::default());
        assert_eq!(
            serde_json::to_vec(&previous).expect("previous"),
            serde_json::to_vec(&repeated).expect("repeated")
        );
        assert_eq!(
            serde_json::to_value(&context).expect("unchanged context"),
            original
        );
        let prior = projected(previous.clone());
        context.messages.extend([
            Message::user(format!("user{round}")),
            Message::assistant(format!("answer{round}")),
        ]);
        let next = wire.request(&model, &context, &Options::default());
        assert_eq!(
            locations(&next),
            vec![
                (context.messages.len() - 2, 0),
                (context.messages.len() - 1, 0)
            ]
        );
        for field in ["system", "tools"] {
            assert_eq!(next[field], previous[field]);
        }
        let next = projected(next);
        assert_eq!(
            next["messages"]
                .as_array()
                .expect("next messages")
                .get(..context.messages.len() - 2),
            prior["messages"].as_array().map(Vec::as_slice)
        );
    }
}

#[test]
fn nonvision_adaptation_precedes_marker_selection_and_off_modes_remain_unmarked() {
    let mut user = Message::user("inspect");
    user.content.push(Content::Image {
        data: "image-that-must-be-adapted".into(),
        media_type: "image/png".into(),
    });
    let context = Context {
        messages: vec![user, Message::assistant("answer")],
        ..Context::default()
    };
    let wire = adapter("anthropic-messages");
    let mut capable = model(Some(PromptCache::AnthropicEphemeral));
    capable.input = vec![crate::mind::provider::model::Modality::Text];
    let body = wire.request(&capable, &context, &Options::default());
    assert_eq!(locations(&body), vec![(0, 1), (1, 0)]);
    assert_eq!(body["messages"][0]["content"][1]["type"], "text");
    assert!(!body.to_string().contains("image-that-must-be-adapted"));
    capable.compat.as_mut().expect("compat").prompt_cache = Some(PromptCache::None);
    let off = wire.request(&capable, &context, &Options::default());
    assert!(locations(&off).is_empty());
    assert_eq!(projected(body), off);
    assert!(locations(&request(&context, &Options::default(), None)).is_empty());
}
