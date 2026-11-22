use super::{support::*, *};
use crate::mind::model::{Content, Message, Role, StopReason, ThinkingLevel};
use crate::mind::provider::{
    api::Schema,
    compat::{AnthropicThinking, Compat},
    model::Api,
};
use serde_json::{Value, json};

mod streaming;
mod terminal;

fn model() -> Model {
    let mut model = plain_model();
    model.api = Api::AnthropicMessages;
    model.base_url = Some("https://fixture.invalid/anthropic".into());
    model.compat = Some(Compat {
        anthropic_thinking: Some(AnthropicThinking::AdaptiveEffort),
        anthropic_replay: Some(true),
        allow_empty_signature: Some(true),
        supports_forced_tool_choice: Some(false),
        ..Default::default()
    });
    model.thinking = ThinkingLevel::ALL
        .into_iter()
        .map(|level| {
            (
                level,
                (!matches!(level, ThinkingLevel::Off | ThinkingLevel::Minimal))
                    .then(|| level.as_str().into()),
            )
        })
        .collect();
    model
}

fn options() -> Options {
    Options {
        continuation_binding: Some("credential-lineage-a".into()),
        ..Default::default()
    }
}

fn begin(api: &LuaAdapter, model: &Model) -> StreamState {
    let (body, _) = api
        .prepare(model, &plain_context(), &options())
        .expect("request");
    StreamState {
        scratch: body["__scratch"].clone(),
        ..Default::default()
    }
}

fn event(api: &LuaAdapter, state: &mut StreamState, name: &str, value: Value) -> Vec<Delta> {
    api.on_event(
        state,
        &sse::Event {
            name: name.into(),
            data: value.to_string(),
        },
    )
}

fn started(api: &LuaAdapter, state: &mut StreamState) -> Vec<Delta> {
    event(
        api,
        state,
        "message_start",
        json!({"message":{"role":"assistant","content":[],
        "usage":{"input_tokens":60,"output_tokens":0,"cache_read_input_tokens":30,"cache_creation_input_tokens":2}}}),
    )
}

fn finished(api: &LuaAdapter, state: &mut StreamState) -> Vec<Delta> {
    event(
        api,
        state,
        "message_delta",
        json!({"delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":12}}),
    )
}

fn neutral(deltas: &[Delta]) -> Message {
    let (mut thinking, mut text, mut signature) = (String::new(), String::new(), None);
    let mut calls = Vec::<(String, String, String)>::new();
    for delta in deltas {
        match delta {
            Delta::Thinking(value) => thinking.push_str(value),
            Delta::Text(value) => text.push_str(value),
            Delta::Signature(value) => signature = Some(value.clone()),
            Delta::ToolCallStart { id, name } => {
                calls.push((id.clone(), name.clone(), String::new()))
            }
            Delta::ToolCallArgs(value) => calls.last_mut().expect("call").2.push_str(value),
            _ => {}
        }
    }
    let mut content = vec![
        Content::Thinking {
            thinking,
            signature,
        },
        Content::Text {
            text,
            signature: None,
        },
    ];
    content.extend(
        calls
            .into_iter()
            .map(|(id, name, arguments)| Content::ToolCall {
                id,
                name,
                arguments: serde_json::from_str(&arguments).expect("arguments"),
                thought_signature: None,
            }),
    );
    Message {
        role: Role::Assistant,
        content,
        stop_reason: Some(StopReason::ToolUse),
        usage: None,
        error: None,
    }
}

#[test]
fn adaptive_modes_preserve_null_levels_and_leave_budget_defaults_unchanged() {
    let api = adapter("anthropic-messages");
    for mode in [
        AnthropicThinking::Budget,
        AnthropicThinking::Adaptive,
        AnthropicThinking::AdaptiveEffort,
    ] {
        let mut model = model();
        model.compat.as_mut().expect("compat").anthropic_thinking = Some(mode);
        for level in std::iter::once(None).chain(ThinkingLevel::ALL.into_iter().map(Some)) {
            let options = Options {
                thinking: level,
                ..options()
            };
            let body = api
                .prepare(&model, &plain_context(), &options)
                .expect("request")
                .0;
            let supported = level.is_some_and(|level| {
                !matches!(level, ThinkingLevel::Off | ThinkingLevel::Minimal)
                    && (level != ThinkingLevel::XHigh || mode == AnthropicThinking::AdaptiveEffort)
            });
            if !supported {
                assert!(body.get("thinking").is_none(), "{mode:?} {level:?}: {body}");
            } else if mode == AnthropicThinking::Budget {
                assert_eq!(body["thinking"]["type"], "enabled");
                assert!(body["thinking"]["budget_tokens"].as_u64().expect("budget") <= 7168);
            } else {
                assert_eq!(body["thinking"], json!({"type":"adaptive"}));
            }
            if supported && mode == AnthropicThinking::AdaptiveEffort {
                assert_eq!(
                    body["output_config"]["effort"],
                    level.expect("level").as_str()
                );
            } else {
                assert!(body.get("output_config").is_none());
            }
        }
        model
            .thinking
            .insert(ThinkingLevel::Off, Some("off".into()));
        let body = api
            .prepare(
                &model,
                &plain_context(),
                &Options {
                    thinking: Some(ThinkingLevel::Off),
                    ..options()
                },
            )
            .expect("off")
            .0;
        if mode == AnthropicThinking::Budget {
            assert!(body.get("thinking").is_none());
        } else {
            assert_eq!(body["thinking"], json!({"type":"disabled"}));
        }
    }
    let defaults = crate::mind::provider::compat::resolve(None);
    assert_eq!(defaults.anthropic_thinking, AnthropicThinking::Budget);
    assert!(!defaults.anthropic_replay && !defaults.allow_empty_signature);
    assert!(defaults.supports_forced_tool_choice);
}

#[test]
fn forced_schema_selection_is_a_typed_preparation_refusal_only_when_declared() {
    let api = adapter("anthropic-messages");
    let mut model = model();
    let options = Options {
        schema: Some(Schema {
            name: "answer".into(),
            schema: json!({"type":"object"}),
        }),
        ..options()
    };
    assert!(
        api.prepare(&model, &plain_context(), &options)
            .expect_err("auto only")
            .contains("forced tool selection")
    );
    model
        .compat
        .as_mut()
        .expect("compat")
        .supports_forced_tool_choice = Some(true);
    let body = api
        .prepare(&model, &plain_context(), &options)
        .expect("forced supported")
        .0;
    assert_eq!(body["tool_choice"], json!({"type":"tool","name":"answer"}));
}

#[test]
fn complete_assistant_blocks_survive_flattening_without_losing_initial_values_or_json_shapes() {
    let api = adapter("anthropic-messages");
    let model = model();
    let mut state = begin(&api, &model);
    let mut deltas = started(&api, &mut state);
    let blocks = json!([
        {"type":"thinking","thinking":"first "}, {"type":"text","text":"before "},
        {"type":"thinking","thinking":"second","signature":""},
        {"type":"tool_use","id":"call-1","name":"echo","input":{"large":18446744073709551615u64,"empty":{},"array":[null,[]]}},
        {"type":"text","text":"after"}, {"type":"tool_use","id":"call-2","name":"echo","input":{}},
        {"type":"redacted_thinking","data":"opaque α+/="}
    ]);
    for (index, block) in blocks.as_array().expect("blocks").iter().enumerate() {
        deltas.extend(event(
            &api,
            &mut state,
            "content_block_start",
            json!({"index":index,"content_block":block}),
        ));
        deltas.extend(event(
            &api,
            &mut state,
            "content_block_stop",
            json!({"index":index}),
        ));
    }
    deltas.extend(finished(&api, &mut state));
    assert_eq!(
        deltas
            .iter()
            .filter(|d| matches!(d, Delta::Signature(_)))
            .count(),
        1,
        "{deltas:?}"
    );
    assert_eq!(deltas.last(), Some(&Delta::Stop(StopReason::ToolUse)));
    assert_eq!(
        (
            state.usage.input,
            state.usage.output,
            state.usage.cache_read,
            state.usage.cache_write
        ),
        (60, 12, 30, 2)
    );
    assert!(event(&api, &mut state, "message_stop", json!({})).is_empty());
    let message = neutral(&deltas);
    let context = Context {
        messages: vec![Message::user("hi"), message.clone()],
        ..Default::default()
    };
    let body = api
        .prepare(&model, &context, &options())
        .expect("followup")
        .0;
    assert_eq!(body["messages"][1]["content"], blocks);
    for changed in 0..6 {
        let (mut model, mut context, mut opts) = (model.clone(), context.clone(), options());
        match changed {
            0 => model.id.push_str("-other"),
            1 => model.provider.push_str("-other"),
            2 => model.base_url = Some("https://other.invalid".into()),
            3 => opts.continuation_binding = Some("credential-lineage-b".into()),
            4 => {
                context.messages[1].content[1] = Content::Text {
                    text: "edited text".into(),
                    signature: None,
                }
            }
            _ => {
                if let Content::ToolCall { arguments, .. } = &mut context.messages[1].content[2] {
                    *arguments = json!({"edited":true});
                }
            }
        }
        let body = api
            .prepare(&model, &context, &opts)
            .expect("changed context")
            .0;
        let blocks = body["messages"][1]["content"]
            .as_array()
            .expect("visible fallback");
        assert!(
            blocks
                .iter()
                .all(|b| b["type"] != "thinking" && b["type"] != "redacted_thinking")
        );
        assert!(
            !serde_json::to_string(blocks)
                .expect("visible")
                .contains("melchior.anthropic.v1:")
        );
    }
}
