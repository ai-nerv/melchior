use super::support::{adapter, plain_context, plain_model};
use crate::mind::model::ThinkingLevel;
use crate::mind::provider::{
    api::{Adapter, Options},
    model::Model,
    thinking,
};
use serde_json::{Value, json};

fn model(supports: bool) -> Model {
    let mut model = plain_model();
    model.reasoning = true;
    model.compat = Some(
        serde_json::from_value(json!({
            "thinking_format":"together", "supports_reasoning_effort":supports
        }))
        .expect("Together dialect"),
    );
    model
}

fn request(model: &Model, level: Option<ThinkingLevel>) -> Value {
    adapter("openai-completions").request(
        model,
        &plain_context(),
        &Options {
            thinking: level,
            ..Options::default()
        },
    )
}

#[test]
fn hybrid_toggle_and_model_supported_efforts_are_independent() {
    for supports in [false, true] {
        let mut model = model(supports);
        for level in [ThinkingLevel::Off, ThinkingLevel::High, ThinkingLevel::Max] {
            let body = request(&model, Some(level));
            assert_eq!(
                body["reasoning"],
                json!({"enabled":level != ThinkingLevel::Off})
            );
            assert_eq!(
                body.get("reasoning_effort").and_then(Value::as_str),
                (supports && level != ThinkingLevel::Off).then_some(level.as_str())
            );
            assert!(body.get("enable_thinking").is_none());
            assert!(body.get("thinking").is_none());
            assert!(body.get("chat_template_kwargs").is_none());
        }
        model
            .thinking
            .insert(ThinkingLevel::High, Some("native-high".into()));
        assert_eq!(
            request(&model, Some(ThinkingLevel::High))
                .get("reasoning_effort")
                .and_then(Value::as_str),
            supports.then_some("native-high")
        );
    }
}

#[test]
fn unspecified_null_and_nonreasoning_controls_remain_absent() {
    let mut model = model(true);
    for level in ThinkingLevel::ALL {
        model.thinking.insert(level, None);
    }
    for level in std::iter::once(None).chain(ThinkingLevel::ALL.into_iter().map(Some)) {
        let body = request(&model, level);
        assert!(body.get("reasoning").is_none(), "{body}");
        assert!(body.get("reasoning_effort").is_none(), "{body}");
    }
    model.thinking.clear();
    model.reasoning = false;
    for level in std::iter::once(None).chain(ThinkingLevel::ALL.into_iter().map(Some)) {
        let body = request(&model, level);
        assert!(body.get("reasoning").is_none(), "{body}");
        assert!(body.get("reasoning_effort").is_none(), "{body}");
    }
}

#[test]
fn off_aliases_preserve_required_minimum_effort() {
    let mut model = model(true);
    model
        .thinking
        .insert(ThinkingLevel::Off, Some("low".into()));
    let body = request(&model, Some(ThinkingLevel::Off));
    assert_eq!(body["reasoning"], json!({"enabled":true}));
    assert_eq!(body["reasoning_effort"], "low");
}

#[test]
fn xhigh_requires_explicit_supported_effort_metadata() {
    for supports in [false, true] {
        let mut model = model(supports);
        assert!(thinking::effective(&model, ThinkingLevel::XHigh).is_none());
        model
            .thinking
            .insert(ThinkingLevel::XHigh, Some("native-xhigh".into()));
        assert_eq!(
            thinking::effective(&model, ThinkingLevel::XHigh).as_deref(),
            supports.then_some("native-xhigh")
        );
        let body = request(&model, Some(ThinkingLevel::XHigh));
        assert_eq!(
            body.get("reasoning_effort").and_then(Value::as_str),
            supports.then_some("native-xhigh")
        );
        assert_eq!(
            body.get("reasoning"),
            supports.then_some(&json!({"enabled":true}))
        );
        model.thinking.insert(ThinkingLevel::XHigh, None);
        assert!(
            request(&model, Some(ThinkingLevel::XHigh))
                .get("reasoning")
                .is_none()
        );
    }
}
