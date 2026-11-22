use super::support::{adapter, plain_context, plain_model};
use crate::mind::model::ThinkingLevel;
use crate::mind::provider::{
    api::{Adapter, Options},
    compat::{Compat, ThinkingFormat},
    thinking,
};
use serde_json::json;

#[test]
fn qwen_effort_values_are_sent_only_when_the_model_accepts_them() {
    for supports in [false, true] {
        let mut model = plain_model();
        model.reasoning = true;
        model.compat = Some(Compat {
            thinking_format: Some(ThinkingFormat::Qwen),
            supports_reasoning_effort: Some(supports),
            ..Compat::default()
        });
        for (level, alias) in [
            (ThinkingLevel::Low, "low"),
            (ThinkingLevel::Medium, "medium"),
            (ThinkingLevel::High, "native-high"),
            (ThinkingLevel::XHigh, "xhigh"),
            (ThinkingLevel::Max, "max"),
        ] {
            model.thinking.insert(level, Some(alias.into()));
            let body = adapter("openai-completions").request(
                &model,
                &plain_context(),
                &Options {
                    thinking: Some(level),
                    ..Options::default()
                },
            );
            let supported = supports || level != ThinkingLevel::XHigh;
            assert_eq!(
                body.get("enable_thinking"),
                supported.then_some(&json!(true)),
                "{supports}/{level:?}: {body}"
            );
            assert_eq!(
                body.get("reasoning_effort").and_then(|v| v.as_str()),
                supports.then_some(alias),
                "{supports}/{level:?}: {body}"
            );
            assert!(body.get("thinking_budget").is_none());
            assert!(body.get("chat_template_kwargs").is_none());
            assert!(body.get("thinking").is_none());
        }
    }
}

#[test]
fn qwen_defaults_nulls_and_nonreasoning_models_do_not_gain_effort_controls() {
    let mut model = plain_model();
    model.reasoning = true;
    model.compat = Some(Compat {
        thinking_format: Some(ThinkingFormat::Qwen),
        supports_reasoning_effort: Some(true),
        ..Compat::default()
    });
    for level in [
        ThinkingLevel::Off,
        ThinkingLevel::High,
        ThinkingLevel::XHigh,
    ] {
        model.thinking.insert(level, None);
    }
    for level in [
        None,
        Some(ThinkingLevel::Off),
        Some(ThinkingLevel::High),
        Some(ThinkingLevel::XHigh),
    ] {
        let body = adapter("openai-completions").request(
            &model,
            &plain_context(),
            &Options {
                thinking: level,
                ..Options::default()
            },
        );
        assert!(body.get("enable_thinking").is_none(), "{body}");
        assert!(body.get("reasoning_effort").is_none(), "{body}");
    }
    model.thinking.clear();
    model.reasoning = false;
    for level in [ThinkingLevel::High, ThinkingLevel::XHigh] {
        let body = adapter("openai-completions").request(
            &model,
            &plain_context(),
            &Options {
                thinking: Some(level),
                ..Options::default()
            },
        );
        assert!(body.get("enable_thinking").is_none());
        assert!(body.get("reasoning_effort").is_none());
    }
}

#[test]
fn qwen_off_keeps_explicit_disabling_and_mandatory_minimum_aliases_distinct() {
    let mut model = plain_model();
    model.reasoning = true;
    model.compat = Some(Compat {
        thinking_format: Some(ThinkingFormat::Qwen),
        supports_reasoning_effort: Some(true),
        ..Compat::default()
    });
    for alias in [None, Some("low")] {
        if let Some(alias) = alias {
            model
                .thinking
                .insert(ThinkingLevel::Off, Some(alias.into()));
        }
        let body = adapter("openai-completions").request(
            &model,
            &plain_context(),
            &Options {
                thinking: Some(ThinkingLevel::Off),
                ..Options::default()
            },
        );
        assert_eq!(body["enable_thinking"], alias.is_some());
        assert_eq!(body.get("reasoning_effort").and_then(|v| v.as_str()), alias);
    }
}

#[test]
fn qwen_xhigh_is_advertised_only_for_explicit_supported_models() {
    let mut model = plain_model();
    model.reasoning = true;
    for supports in [false, true] {
        model.compat = Some(Compat {
            thinking_format: Some(ThinkingFormat::Qwen),
            supports_reasoning_effort: Some(supports),
            ..Compat::default()
        });
        model.thinking.clear();
        assert!(thinking::effective(&model, ThinkingLevel::XHigh).is_none());
        model
            .thinking
            .insert(ThinkingLevel::XHigh, Some("xhigh".into()));
        assert_eq!(
            thinking::effective(&model, ThinkingLevel::XHigh).as_deref(),
            supports.then_some("xhigh")
        );
        assert_eq!(
            thinking::controls(&model)
                .expect("controls")
                .contains(&"xhigh".into()),
            supports
        );
        model.thinking.insert(ThinkingLevel::XHigh, None);
        assert!(thinking::effective(&model, ThinkingLevel::XHigh).is_none());
    }
}
