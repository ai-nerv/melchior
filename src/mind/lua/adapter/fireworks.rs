use super::support::{adapter, plain_context, plain_model};
use crate::mind::model::{Context, ThinkingLevel, Tool, ToolStrictness};
use crate::mind::provider::{
    api::{Adapter, Options, Schema},
    compat::{AnthropicSchema, AnthropicThinking, Compat, PromptCache},
    model::Api,
};
use serde_json::{Value, json};

fn schema() -> Schema {
    Schema {
        name: "answer".into(),
        schema: json!({"type":"object","properties":{"value":{"type":"integer",
            "maximum":18446744073709551615u64}},"required":["value"],
            "additionalProperties":false,"examples":[{},[],null]}),
    }
}

fn context() -> Context {
    Context {
        system: Some("Keep the JSON exact".into()),
        tools: vec![Tool {
            name: "echo".into(),
            description: "Echo a value".into(),
            parameters: json!({"type":"object","properties":{"value":{"type":"string"}},
                "required":["value"],"additionalProperties":false}),
            strict: None,
        }],
        ..plain_context()
    }
}

#[test]
fn gateway_options_inherit_fieldwise_and_preserve_explicit_defaults() {
    let defaults = Compat::default().resolve();
    assert!(defaults.supports_cache_control_on_tools);
    assert!(!defaults.supports_eager_tool_input_streaming);
    assert_eq!(defaults.anthropic_schema, AnthropicSchema::ForcedTool);
    let base = Compat {
        supports_cache_control_on_tools: Some(false),
        supports_eager_tool_input_streaming: Some(true),
        anthropic_schema: Some(AnthropicSchema::OutputConfig),
        ..Default::default()
    };
    assert_eq!(Compat::default().over(Some(base)), base);
    let own = Compat {
        supports_cache_control_on_tools: Some(true),
        supports_eager_tool_input_streaming: Some(false),
        anthropic_schema: Some(AnthropicSchema::ForcedTool),
        ..Default::default()
    };
    assert_eq!(own.over(Some(base)), own);
    for options in [base, own] {
        let value = serde_json::to_value(options).expect("encode");
        assert_eq!(
            serde_json::from_value::<Compat>(value).expect("decode"),
            options
        );
    }
    for value in [json!("unknown"), json!(true), json!([]), json!({})] {
        assert!(serde_json::from_value::<Compat>(json!({"anthropic_schema":value})).is_err());
    }
    for field in [
        "supports_cache_control_on_tools",
        "supports_eager_tool_input_streaming",
    ] {
        for value in [json!("false"), json!(1), json!([]), json!({})] {
            assert!(serde_json::from_value::<Compat>(json!({field:value})).is_err());
        }
    }
}

#[test]
fn tool_cache_and_eager_controls_do_not_disable_system_or_history_cache() {
    let api = adapter("anthropic-messages");
    let mut model = plain_model();
    model.api = Api::AnthropicMessages;
    let context = context();
    let original = serde_json::to_value(&context).expect("context");
    for cache in [false, true] {
        for eager in [false, true] {
            for with_schema in [false, true] {
                model.compat = Some(Compat {
                    prompt_cache: Some(PromptCache::AnthropicEphemeral),
                    supports_cache_control_on_tools: Some(cache),
                    supports_eager_tool_input_streaming: Some(eager),
                    ..Default::default()
                });
                let options = Options {
                    schema: with_schema.then(schema),
                    ..Default::default()
                };
                let body = api.prepare(&model, &context, &options).expect("request").0;
                let tool = &body["tools"][0];
                assert_eq!(tool.get("cache_control").is_some(), cache);
                assert_eq!(
                    tool.get("eager_input_streaming"),
                    eager.then_some(&Value::Bool(true))
                );
                assert_eq!(
                    body["system"][0]["cache_control"],
                    json!({"type":"ephemeral"})
                );
                assert_eq!(
                    body["messages"][0]["content"][0]["cache_control"],
                    json!({"type":"ephemeral"})
                );
                assert!(!body.to_string().contains("\"ttl\""));
                assert_eq!(
                    tool["input_schema"],
                    if with_schema {
                        schema().schema
                    } else {
                        context.tools[0].parameters.clone()
                    }
                );
            }
        }
    }
    assert_eq!(serde_json::to_value(context).expect("context"), original);
}

#[test]
fn adaptive_off_normalizes_only_disable_aliases_and_preserves_null_or_low_mappings() {
    let api = adapter("anthropic-messages");
    for mode in [
        AnthropicThinking::Adaptive,
        AnthropicThinking::AdaptiveEffort,
    ] {
        for mapped in [None, Some("none"), Some("off"), Some("low")] {
            let mut model = plain_model();
            model.api = Api::AnthropicMessages;
            model
                .thinking
                .insert(ThinkingLevel::Off, mapped.map(str::to_owned));
            model.compat = Some(Compat {
                anthropic_thinking: Some(mode),
                ..Default::default()
            });
            let before = model.thinking.clone();
            for thinking in [None, Some(ThinkingLevel::Off)] {
                let options = Options {
                    thinking,
                    ..Default::default()
                };
                let body = api
                    .prepare(&model, &plain_context(), &options)
                    .expect("request")
                    .0;
                let enabled = thinking.is_some() && mapped.is_some();
                assert_eq!(body.get("thinking").is_some(), enabled);
                if enabled {
                    assert_eq!(
                        body["thinking"]["type"],
                        if mapped == Some("low") {
                            "adaptive"
                        } else {
                            "disabled"
                        }
                    );
                }
                if enabled && mapped == Some("low") && mode == AnthropicThinking::AdaptiveEffort {
                    assert_eq!(body["output_config"], json!({"effort":"low"}));
                } else {
                    assert!(body.get("output_config").is_none());
                }
            }
            assert_eq!(model.thinking, before);
        }
    }
}

#[test]
fn native_schema_preserves_tools_json_and_effort_without_forced_choice() {
    let api = adapter("anthropic-messages");
    let mut model = plain_model();
    model.api = Api::AnthropicMessages;
    model.compat = Some(Compat {
        anthropic_schema: Some(AnthropicSchema::OutputConfig),
        anthropic_thinking: Some(AnthropicThinking::AdaptiveEffort),
        supports_forced_tool_choice: Some(false),
        ..Default::default()
    });
    model
        .thinking
        .insert(ThinkingLevel::Off, Some("none".into()));
    for context in [plain_context(), context()] {
        let before = serde_json::to_value(&context).expect("context");
        for thinking in [None, Some(ThinkingLevel::Off), Some(ThinkingLevel::High)] {
            let options = Options {
                schema: Some(schema()),
                thinking,
                ..Default::default()
            };
            let body = api
                .prepare(&model, &context, &options)
                .expect("native schema")
                .0;
            assert_eq!(
                body["output_config"]["format"],
                json!({"type":"json_schema","schema":schema().schema})
            );
            assert!(body.get("tool_choice").is_none());
            assert_eq!(body.get("tools").is_some(), !context.tools.is_empty());
            if !context.tools.is_empty() {
                assert_eq!(body["tools"][0]["name"], "echo");
                assert_eq!(
                    body["tools"][0]["input_schema"],
                    context.tools[0].parameters
                );
            }
            assert_eq!(
                body["output_config"].get("effort"),
                (thinking == Some(ThinkingLevel::High)).then_some(&json!("high"))
            );
            if thinking == Some(ThinkingLevel::Off) {
                assert_eq!(body["thinking"], json!({"type":"disabled"}));
            }
        }
        assert_eq!(serde_json::to_value(context).expect("context"), before);
    }
    model.compat.as_mut().expect("compat").supports_json_schema = Some(false);
    assert!(
        api.prepare(
            &model,
            &plain_context(),
            &Options {
                schema: Some(schema()),
                ..Default::default()
            }
        )
        .expect_err("unsupported schema")
        .contains("JSON-schema")
    );
}

#[test]
fn native_answers_and_strict_tool_guarantees_are_independent() {
    let api = adapter("anthropic-messages");
    let mut model = plain_model();
    model.api = Api::AnthropicMessages;
    let mut context = context();
    context.tools[0].strict = Some(ToolStrictness::Require);
    let options = Options {
        schema: Some(schema()),
        ..Default::default()
    };
    for native in [false, true] {
        for strict in [false, true] {
            model.compat = Some(Compat {
                anthropic_schema: Some(if native {
                    AnthropicSchema::OutputConfig
                } else {
                    AnthropicSchema::ForcedTool
                }),
                supports_strict_tools: Some(strict),
                ..Default::default()
            });
            let result = api.prepare(&model, &context, &options);
            if native && strict {
                let body = result.expect("strict native tools").0;
                assert_eq!(body["tools"][0]["strict"], true);
                assert_eq!(body["tools"][0]["name"], "echo");
            } else {
                assert!(
                    result
                        .expect_err("cannot guarantee strict tool")
                        .contains("requires strict arguments")
                );
            }
        }
    }
}

#[test]
fn anthropic_tool_and_schema_controls_do_not_change_other_protocols() {
    for (name, protocol) in [
        ("openai-completions", Api::OpenAiCompletions),
        ("openai-responses", Api::OpenAiResponses),
    ] {
        let api = adapter(name);
        let mut model = plain_model();
        model.api = protocol;
        let options = Options {
            schema: Some(schema()),
            ..Default::default()
        };
        let context = context();
        let baseline = api.prepare(&model, &context, &options).expect("baseline").0;
        model.compat = Some(Compat {
            supports_cache_control_on_tools: Some(false),
            supports_eager_tool_input_streaming: Some(true),
            anthropic_schema: Some(AnthropicSchema::OutputConfig),
            ..Default::default()
        });
        assert_eq!(
            api.prepare(&model, &context, &options)
                .expect("unchanged")
                .0,
            baseline
        );
    }
}

#[test]
fn final_anthropic_usage_replaces_initial_zeros_and_preserves_omitted_measurements() {
    use crate::mind::provider::{api::StreamState, sse};
    let api = adapter("anthropic-messages");
    for replay in [false, true] {
        let mut model = plain_model();
        model.api = Api::AnthropicMessages;
        model.compat = Some(Compat {
            anthropic_replay: Some(replay),
            ..Default::default()
        });
        let body = api
            .prepare(&model, &plain_context(), &Options::default())
            .expect("request")
            .0;
        let mut state = StreamState {
            scratch: body.get("__scratch").cloned().unwrap_or(json!({})),
            ..Default::default()
        };
        for (name, value, expected) in [
            (
                "message_start",
                json!({"message":{"role":"assistant","content":[],"usage":{"input_tokens":0,"output_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}),
                (0, 0, 0, 0),
            ),
            (
                "message_delta",
                json!({"usage":{"input_tokens":60,"output_tokens":7,"cache_read_input_tokens":40,"cache_creation_input_tokens":2}}),
                (60, 7, 40, 2),
            ),
            (
                "message_delta",
                json!({"usage":{"output_tokens":9}}),
                (60, 9, 40, 2),
            ),
            (
                "message_delta",
                json!({"usage":{"input_tokens":"bad","cache_read_input_tokens":-1,"cache_creation_input_tokens":[],"output_tokens":10}}),
                (60, 10, 40, 2),
            ),
            (
                "message_delta",
                json!({"usage":{"input_tokens":0,"cache_read_input_tokens":100}}),
                (0, 10, 100, 2),
            ),
            (
                "message_delta",
                json!({"usage":{"input_tokens":0,"cache_read_input_tokens":0}}),
                (0, 10, 100, 2),
            ),
        ] {
            api.on_event(
                &mut state,
                &sse::Event {
                    name: name.into(),
                    data: value.to_string(),
                },
            );
            assert_eq!(
                (
                    state.usage.input,
                    state.usage.output,
                    state.usage.cache_read,
                    state.usage.cache_write
                ),
                expected,
                "replay={replay}: {value}"
            );
        }
        assert_eq!(state.usage_anomalies, 3);
        assert_eq!(state.measured_usage.input, Some(0));
        assert_eq!(state.measured_usage.output, Some(10));
        assert_eq!(state.measured_usage.cache_read, Some(100));
        assert_eq!(state.measured_usage.cache_write, Some(2));
    }
}
