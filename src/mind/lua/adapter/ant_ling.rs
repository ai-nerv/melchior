use super::support::{adapter, plain_context, plain_model};
use crate::mind::model::{Capabilities, ThinkingLevel};
use crate::mind::provider::{
    api::{Adapter, Options},
    model::Model,
    thinking,
};
use serde_json::{Value, json};

fn model() -> Model {
    let mut model = plain_model();
    model.reasoning = true;
    model.compat = Some(
        serde_json::from_value(json!({
            "thinking_format":"ant-ling", "supports_reasoning_effort":false
        }))
        .expect("Ant Ling dialect"),
    );
    model
        .thinking
        .insert(ThinkingLevel::High, Some("native-high".into()));
    model
        .thinking
        .insert(ThinkingLevel::XHigh, Some("native-xhigh".into()));
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
fn only_explicit_effort_mappings_emit_nested_reasoning_without_enabling_top_level_efforts() {
    let model = model();
    for level in ThinkingLevel::ALL {
        let body = request(&model, Some(level));
        let alias = model.thinking.get(&level).and_then(Option::as_deref);
        assert_eq!(thinking::effective(&model, level).as_deref(), alias);
        assert_eq!(
            body.get("reasoning"),
            alias.map(|effort| json!({"effort":effort})).as_ref()
        );
        for field in [
            "reasoning_effort",
            "thinking",
            "enable_thinking",
            "chat_template_kwargs",
        ] {
            assert!(body.get(field).is_none());
        }
    }
    assert_eq!(
        thinking::controls(&model),
        Some(vec!["high".into(), "xhigh".into()])
    );
}

#[test]
fn absent_null_empty_and_nonreasoning_mappings_never_create_controls() {
    let mut model = model();
    assert!(request(&model, None).get("reasoning").is_none());
    model.thinking.insert(ThinkingLevel::High, None);
    model
        .thinking
        .insert(ThinkingLevel::XHigh, Some(String::new()));
    for level in ThinkingLevel::ALL {
        assert!(request(&model, Some(level)).get("reasoning").is_none());
    }
    model.thinking.clear();
    assert_eq!(thinking::controls(&model), Some(Vec::new()));
    model
        .thinking
        .insert(ThinkingLevel::Off, Some("forced".into()));
    model.reasoning = false;
    assert!(
        request(&model, Some(ThinkingLevel::Off))
            .get("reasoning")
            .is_none()
    );
    assert_eq!(thinking::controls(&model), Some(Vec::new()));
}

#[test]
fn explicit_capabilities_restrict_but_never_invent_an_effort_mapping() {
    let mut model = model();
    model.capabilities = Some(Capabilities {
        thinking_levels: Some(vec!["max".into(), "xhigh".into()]),
        ..Capabilities::default()
    });
    assert_eq!(thinking::controls(&model), Some(vec!["xhigh".into()]));
    assert!(
        request(&model, Some(ThinkingLevel::High))
            .get("reasoning")
            .is_none()
    );
    assert!(
        request(&model, Some(ThinkingLevel::Max))
            .get("reasoning")
            .is_none()
    );
    assert_eq!(
        request(&model, Some(ThinkingLevel::XHigh))["reasoning"],
        json!({"effort":"native-xhigh"})
    );
}

#[test]
fn explicit_off_aliases_are_values_not_generic_disable_toggles() {
    let mut model = model();
    for alias in ["off", "none", "low"] {
        model
            .thinking
            .insert(ThinkingLevel::Off, Some(alias.into()));
        let body = request(&model, Some(ThinkingLevel::Off));
        assert_eq!(body["reasoning"], json!({"effort":alias}));
        assert!(body.get("reasoning_effort").is_none());
    }
}
