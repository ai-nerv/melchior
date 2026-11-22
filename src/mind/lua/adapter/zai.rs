use super::support::{adapter, plain_context, plain_model};
use crate::mind::model::{ThinkingLevel, Tool};
use crate::mind::provider::{
    api::{Adapter, Options},
    compat::{Compat, ThinkingFormat},
};
use serde_json::{Value, json};

#[test]
fn tool_stream_is_explicit_inherited_and_only_sent_with_tools() {
    assert!(!Compat::default().resolve().zai_tool_stream);
    let base = Some(Compat {
        zai_tool_stream: Some(true),
        supports_store: Some(true),
        ..Compat::default()
    });
    for value in [None, Some(false), Some(true)] {
        let compat = Compat {
            zai_tool_stream: value,
            ..Compat::default()
        }
        .over(base);
        assert!(compat.resolve().supports_store);
        assert_eq!(compat.resolve().zai_tool_stream, value.unwrap_or(true));
        let encoded = serde_json::to_value(compat).expect("compat");
        assert_eq!(
            serde_json::from_value::<Compat>(encoded).expect("decode"),
            compat
        );
        let mut model = plain_model();
        model.compat = Some(compat);
        for with_tools in [false, true] {
            let mut ctx = plain_context();
            if with_tools {
                ctx.tools.push(
                    serde_json::from_value::<Tool>(json!({
                        "name":"echo","description":"Echo","parameters":{"type":"object"}
                    }))
                    .expect("tool"),
                );
            }
            let body = adapter("openai-completions").request(&model, &ctx, &Options::default());
            assert_eq!(
                body.get("tool_stream"),
                (with_tools && value != Some(false)).then_some(&json!(true))
            );
            assert_eq!(body["stream"], true);
        }
    }
}

#[test]
fn zai_controls_preserve_thinking_and_native_effort_without_affecting_other_dialects() {
    let wire = adapter("openai-completions");
    for format in [
        ThinkingFormat::Zai,
        ThinkingFormat::DeepSeek,
        ThinkingFormat::OpenAi,
    ] {
        for enabled in [false, true] {
            for level in [
                None,
                Some(ThinkingLevel::Off),
                Some(ThinkingLevel::High),
                Some(ThinkingLevel::Max),
            ] {
                let mut model = plain_model();
                model.compat = Some(Compat {
                    thinking_format: Some(format),
                    supports_reasoning_effort: Some(enabled),
                    ..Compat::default()
                });
                let body = wire.request(
                    &model,
                    &plain_context(),
                    &Options {
                        thinking: level,
                        ..Options::default()
                    },
                );
                let reasoning = level.filter(|level| *level != ThinkingLevel::Off);
                assert_eq!(
                    body.get("reasoning_effort"),
                    reasoning
                        .filter(|_| enabled)
                        .map(|level| json!(level.as_str()))
                        .as_ref()
                );
                if format == ThinkingFormat::Zai {
                    assert_eq!(
                        body["thinking"],
                        if level == Some(ThinkingLevel::Off) {
                            json!({"type":"disabled"})
                        } else {
                            json!({"type":"enabled","clear_thinking":false})
                        }
                    );
                } else {
                    assert!(body["thinking"].get("clear_thinking").is_none());
                    if level.is_none() {
                        assert!(body.get("thinking").is_none());
                    }
                }
            }
        }
    }
}

#[test]
fn mapped_none_and_null_off_remain_distinct_and_nonreasoning_models_do_not_gain_thinking() {
    for mapped in [Some("none".to_owned()), None] {
        let mut model = plain_model();
        model.thinking.insert(ThinkingLevel::Off, mapped.clone());
        model.compat = Some(Compat {
            thinking_format: Some(ThinkingFormat::Zai),
            supports_reasoning_effort: Some(true),
            ..Compat::default()
        });
        for reasoning in [false, true] {
            model.reasoning = reasoning;
            let body = adapter("openai-completions").request(
                &model,
                &plain_context(),
                &Options {
                    thinking: Some(ThinkingLevel::Off),
                    ..Options::default()
                },
            );
            if reasoning {
                assert_eq!(
                    body["thinking"],
                    json!({"type":"enabled","clear_thinking":false})
                );
                assert_eq!(
                    body.get("reasoning_effort"),
                    mapped.as_ref().map(|s| json!(s)).as_ref()
                );
            } else {
                assert!(body.get("thinking").is_none());
                assert!(body.get("reasoning_effort").is_none());
            }
        }
    }
    let mut model = plain_model();
    model.reasoning = false;
    model.compat = Some(Compat {
        thinking_format: Some(ThinkingFormat::Zai),
        ..Compat::default()
    });
    let body: Value =
        adapter("openai-completions").request(&model, &plain_context(), &Options::default());
    assert!(body.get("thinking").is_none());
}
