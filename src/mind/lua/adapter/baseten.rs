use super::support::{adapter, plain_context, plain_model};
use crate::mind::model::ThinkingLevel;
use crate::mind::provider::{
    api::{Adapter, Options},
    compat::{ChatTemplateArgs, Compat, ThinkingFormat},
};
use serde_json::json;

fn template(value: serde_json::Value) -> ChatTemplateArgs {
    serde_json::from_value(value).expect("template")
}

#[test]
fn template_types_are_bounded_explicit_and_inherit_without_losing_false_or_empty_overrides() {
    let enabled = template(json!({"enable_thinking":{"$var":"thinking.enabled"}}));
    for value in [
        json!({}),
        json!({"enable_thinking":false}),
        json!({"enable_thinking":true}),
        json!({"enable_thinking":{"$var":"thinking.enabled","omitWhenOff":true}}),
    ] {
        let declared = template(value);
        let compat = Compat {
            chat_template_args: Some(declared),
            ..Compat::default()
        }
        .over(Some(Compat {
            chat_template_args: Some(enabled),
            supports_store: Some(true),
            ..Compat::default()
        }));
        assert_eq!(compat.resolve().chat_template_args, declared);
        assert!(compat.resolve().supports_store);
        assert_eq!(
            serde_json::from_value::<Compat>(serde_json::to_value(compat).expect("JSON"))
                .expect("round trip"),
            compat
        );
    }
    assert_eq!(
        Compat::default()
            .over(Some(Compat {
                chat_template_args: Some(enabled),
                ..Compat::default()
            }))
            .resolve()
            .chat_template_args,
        enabled
    );
    for invalid in [
        json!([]),
        json!(true),
        json!({"unknown":true}),
        json!({"enable_thinking":"true"}),
        json!({"enable_thinking":null}),
        json!({"enable_thinking":["thinking.enabled"]}),
        json!({"enable_thinking":{"$var":"thinking.enabled","omitWhenOff":null}}),
        json!({"enable_thinking":{"$var":"thinking.budget"}}),
        json!({"enable_thinking":{"$var":"thinking.enabled","unknown":true}}),
        json!({"enable_thinking":{"$var":"thinking.enabled","omitWhenOff":"yes"}}),
    ] {
        assert!(
            serde_json::from_value::<ChatTemplateArgs>(invalid.clone()).is_err(),
            "accepted {invalid}"
        );
    }
}

#[test]
fn neutral_off_is_not_confused_with_its_native_none_alias_and_unsupported_levels_do_not_disable() {
    let wire = adapter("openai-completions");
    let mut model = plain_model();
    model.reasoning = true;
    model.thinking = ThinkingLevel::ALL
        .into_iter()
        .map(|level| {
            (
                level,
                match level {
                    ThinkingLevel::Off => Some("none".into()),
                    ThinkingLevel::High => Some("high".into()),
                    ThinkingLevel::Max => Some("max".into()),
                    _ => None,
                },
            )
        })
        .collect();
    for template_value in [
        json!({}),
        json!({"enable_thinking":false}),
        json!({"enable_thinking":true}),
        json!({"enable_thinking":{"$var":"thinking.enabled"}}),
        json!({"enable_thinking":{"$var":"thinking.enabled","omitWhenOff":true}}),
    ] {
        for supports in [false, true] {
            model.compat = Some(Compat {
                thinking_format: Some(ThinkingFormat::Baseten),
                chat_template_args: Some(template(template_value.clone())),
                supports_reasoning_effort: Some(supports),
                ..Compat::default()
            });
            for level in std::iter::once(None).chain(ThinkingLevel::ALL.into_iter().map(Some)) {
                let body = wire.request(
                    &model,
                    &plain_context(),
                    &Options {
                        thinking: level,
                        ..Options::default()
                    },
                );
                let supported = level.is_none_or(|level| {
                    matches!(
                        level,
                        ThinkingLevel::Off | ThinkingLevel::High | ThinkingLevel::Max
                    )
                });
                let enabled = matches!(level, Some(ThinkingLevel::High | ThinkingLevel::Max));
                let expected = template_value
                    .get("enable_thinking")
                    .and_then(|value| {
                        if value.is_boolean() {
                            value.as_bool()
                        } else if value["omitWhenOff"] == true && !enabled {
                            None
                        } else {
                            Some(enabled)
                        }
                    })
                    .filter(|_| supported);
                assert_eq!(
                    body.get("chat_template_args"),
                    expected
                        .map(|value| json!({"enable_thinking":value}))
                        .as_ref(),
                    "{level:?}/{template_value}"
                );
                let effort = if supports && supported {
                    Some(match level {
                        Some(ThinkingLevel::High) => "high",
                        Some(ThinkingLevel::Max) => "max",
                        _ => "none",
                    })
                } else {
                    None
                };
                assert_eq!(
                    body.get("reasoning_effort")
                        .and_then(serde_json::Value::as_str),
                    effort
                );
                assert!(body.get("thinking").is_none());
                assert!(body.get("__thinking_enabled").is_none());
            }
        }
    }
}

#[test]
fn template_controls_are_scoped_to_baseten_reasoning_models() {
    for format in [
        ThinkingFormat::OpenAi,
        ThinkingFormat::OpenRouter,
        ThinkingFormat::DeepSeek,
        ThinkingFormat::Zai,
        ThinkingFormat::Qwen,
        ThinkingFormat::Baseten,
    ] {
        for reasoning in [false, true] {
            let mut model = plain_model();
            model.reasoning = reasoning;
            model.compat = Some(Compat {
                thinking_format: Some(format),
                chat_template_args: Some(template(
                    json!({"enable_thinking":{"$var":"thinking.enabled"}}),
                )),
                ..Compat::default()
            });
            let body = adapter("openai-completions").request(
                &model,
                &plain_context(),
                &Options {
                    thinking: Some(ThinkingLevel::High),
                    ..Options::default()
                },
            );
            assert_eq!(
                body.get("chat_template_args").is_some(),
                reasoning && format == ThinkingFormat::Baseten
            );
        }
    }
}
