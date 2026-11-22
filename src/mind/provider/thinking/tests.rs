use super::*;
use crate::mind::lua::adapter::{LuaAdapter, engine_with_builtins};
use crate::mind::provider::api::{Adapter, Options};
use serde_json::json;

fn model() -> Model {
    serde_json::from_value(json!({"id":"fixture","provider":"fixture","name":"Fixture",
        "api":"openai-responses","reasoning":true,"context_window":100000,"max_tokens":64000,
        "thinking":{"off":null,"minimal":null,"low":"low","medium":null,"high":"high","xhigh":"xhigh","max":null}})).expect("model")
}

#[test]
fn input_limits_are_advertised_from_enforced_model_metadata_not_capability_overrides() {
    let mut model = model();
    model.reasoning = false;
    model.thinking.clear();
    model.api = Api::GoogleGenerativeAi;
    model.capabilities = None;
    assert!(capabilities(&model).is_none());
    let limits = serde_json::from_value(json!({"max_request_bytes":1024,"images":{"resize":{
        "max_width":2000,"max_height":1500,"max_bytes":4718592,"jpeg_quality":80}}}))
    .expect("limits");
    model.input_limits = Some(limits);
    assert_eq!(
        capabilities(&model).expect("caps").input_limits,
        model.input_limits
    );
    model.capabilities = Some(Capabilities {
        input_limits: Some(
            serde_json::from_value(json!({"max_request_bytes":1})).expect("override"),
        ),
        ..Default::default()
    });
    assert_eq!(
        capabilities(&model).expect("caps").input_limits,
        model.input_limits
    );
    model.input_limits = None;
    assert!(capabilities(&model).expect("caps").input_limits.is_none());
}

#[test]
fn explicit_maps_publish_only_supported_neutral_controls_and_roundtrip_xhigh() {
    let model = model();
    assert_eq!(
        controls(&model),
        Some(vec!["low".into(), "high".into(), "xhigh".into()])
    );
    assert_eq!(
        effective(&model, ThinkingLevel::XHigh).as_deref(),
        Some("xhigh")
    );
    assert!(effective(&model, ThinkingLevel::Off).is_none());
    let caps = capabilities(&model).expect("controls");
    assert!(caps.thinking.is_none());
    assert_eq!(caps.thinking_levels, controls(&model));
    for level in ThinkingLevel::ALL {
        assert_eq!(serde_json::to_value(level).expect("JSON"), level.as_str());
    }
    let mut bytes = Vec::new();
    ciborium::into_writer(&model, &mut bytes).expect("CBOR");
    let decoded: Model = ciborium::from_reader(bytes.as_slice()).expect("CBOR model");
    assert_eq!(decoded.thinking, model.thinking);
}

#[test]
fn unknown_and_explicit_empty_controls_remain_distinct_and_native_metadata_survives() {
    let mut model = model();
    model.thinking.clear();
    assert!(controls(&model).is_none());
    assert!(effective(&model, ThinkingLevel::XHigh).is_none());
    model.capabilities = Some(Capabilities {
        thinking: Some(vec!["native-effort".into()]),
        thinking_levels: Some(Vec::new()),
        ..Default::default()
    });
    assert_eq!(controls(&model), Some(Vec::new()));
    assert!(effective(&model, ThinkingLevel::High).is_none());
    assert_eq!(
        capabilities(&model).expect("caps").thinking,
        Some(vec!["native-effort".into()])
    );
}

#[test]
fn xhigh_reaches_effort_adapters_but_is_not_invented_for_budget_or_binary_dialects() {
    use crate::mind::model::Context;
    let mut model = model();
    for api in [
        Api::OpenAiResponses,
        Api::AzureOpenAiResponses,
        Api::OpenAiCodexResponses,
        Api::OpenAiCompletions,
    ] {
        model.api = api;
        model.compat = Some(super::super::compat::Compat {
            supports_reasoning_effort: Some(true),
            ..Default::default()
        });
        let adapter = LuaAdapter::new(engine_with_builtins().expect("protocols"), api.as_str())
            .expect("adapter");
        let body = adapter.request(
            &model,
            &Context::default(),
            &Options {
                thinking: Some(ThinkingLevel::XHigh),
                ..Default::default()
            },
        );
        if api == Api::OpenAiCompletions {
            assert_eq!(body["reasoning_effort"], "xhigh");
        } else {
            assert_eq!(body["reasoning"]["effort"], "xhigh");
        }
    }
    for api in [
        Api::AnthropicMessages,
        Api::GoogleGenerativeAi,
        Api::GoogleVertex,
        Api::BedrockConverseStream,
    ] {
        model.api = api;
        assert!(effective(&model, ThinkingLevel::XHigh).is_none());
        assert!(
            !controls(&model)
                .expect("controls")
                .contains(&"xhigh".to_owned())
        );
    }
    model.api = Api::AnthropicMessages;
    model
        .thinking
        .insert(ThinkingLevel::XHigh, Some("high".into()));
    assert_eq!(
        effective(&model, ThinkingLevel::XHigh).as_deref(),
        Some("high")
    );
    model.api = Api::OpenAiCompletions;
    model.compat = None;
    assert!(effective(&model, ThinkingLevel::XHigh).is_none());
}

#[test]
fn discovered_efforts_restrict_actual_requests_and_keep_the_mandatory_minimum_alias() {
    let models = crate::mind::discovering::parse(&json!({"data":[{"id":"fixture",
        "reasoning":{"mandatory":true,"supported_efforts":["xhigh","low"]}}]}));
    let mut model = models.into_iter().next().expect("discovered");
    model.api = Api::OpenAiResponses;
    assert!(model.reasoning);
    assert_eq!(
        effective(&model, ThinkingLevel::Off).as_deref(),
        Some("low")
    );
    assert!(effective(&model, ThinkingLevel::High).is_none());
    assert_eq!(
        effective(&model, ThinkingLevel::XHigh).as_deref(),
        Some("xhigh")
    );
    assert_eq!(
        controls(&model),
        Some(vec!["off".into(), "low".into(), "xhigh".into()])
    );
}

#[test]
fn explicit_empty_or_unknown_mandatory_efforts_do_not_invent_a_minimum() {
    for efforts in [json!([]), json!(["future-effort"])] {
        let models = crate::mind::discovering::parse(&json!({"data":[{"id":"fixture",
            "reasoning":{"mandatory":true,"supported_efforts":efforts}}]}));
        let mut model = models.into_iter().next().expect("model");
        model.api = Api::OpenAiResponses;
        assert!(model.reasoning);
        assert_eq!(controls(&model), Some(Vec::new()));
        for level in ThinkingLevel::ALL {
            assert!(effective(&model, level).is_none());
        }
    }
}
