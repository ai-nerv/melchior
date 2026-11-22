use super::support::*;
use crate::mind::model::{Capabilities, ThinkingLevel};
use crate::mind::provider::{
    api::{Adapter, Options},
    model::{Api, Model},
    thinking,
};
use serde_json::{Value, json};

const APIS: [Api; 2] = [Api::GoogleGenerativeAi, Api::GoogleVertex];

fn model(api: Api, id: &str) -> Model {
    let mut model = plain_model();
    model.api = api;
    model.id = id.into();
    model.max_tokens = 65536;
    model
}

fn request(model: &Model, level: Option<ThinkingLevel>) -> Value {
    adapter(model.api.as_str()).request(
        model,
        &plain_context(),
        &Options {
            thinking: level,
            max_tokens: Some(128),
            ..Default::default()
        },
    )
}

fn config(model: &Model, level: Option<ThinkingLevel>) -> Value {
    let body = request(model, level);
    assert_eq!(body["generationConfig"]["maxOutputTokens"], 128);
    assert!(body.get("__google_control").is_none());
    body["generationConfig"]["thinkingConfig"].clone()
}

#[test]
fn modern_families_use_discrete_levels_and_clamp_only_among_real_controls() {
    use ThinkingLevel::{High, Max, Medium, Minimal, Off, XHigh};
    for api in APIS {
        for (id, minimum, middle, maximum) in [
            ("gemini-3-pro", "LOW", "HIGH", "HIGH"),
            ("gemini-3-flash-preview", "MINIMAL", "MEDIUM", "HIGH"),
            ("gemini-3.1-pro-preview", "LOW", "MEDIUM", "HIGH"),
            ("gemini-3.1-flash-lite-preview", "MINIMAL", "MEDIUM", "HIGH"),
            ("gemini-3.6-flash", "MINIMAL", "MEDIUM", "HIGH"),
            ("gemini-3.7-flash", "LOW", "MEDIUM", "HIGH"),
            ("gemini-3.8-flash", "LOW", "MEDIUM", "HIGH"),
            ("gemini-4-argon", "LOW", "MEDIUM", "HIGH"),
            ("gemini-4-hydrogen", "LOW", "MEDIUM", "XHIGH"),
            ("gemini-4-neon", "LOW", "MEDIUM", "XHIGH"),
            ("gemini-flash-latest", "MINIMAL", "MEDIUM", "HIGH"),
            ("gemini-flash-lite-latest", "MINIMAL", "MEDIUM", "HIGH"),
            ("gemini-pro-latest", "LOW", "MEDIUM", "HIGH"),
            ("gemini-robotics-er-2-preview", "MINIMAL", "MEDIUM", "HIGH"),
        ] {
            let model = model(api, id);
            for selected in [None, Some(Off)] {
                assert_eq!(
                    config(&model, selected),
                    json!({"thinkingLevel":minimum}),
                    "{api:?}/{id}"
                );
            }
            for (selected, expected) in [
                (Minimal, minimum),
                (Medium, middle),
                (High, "HIGH"),
                (XHigh, maximum),
                (Max, maximum),
            ] {
                assert_eq!(
                    config(&model, Some(selected)),
                    json!({"thinkingLevel":expected,"includeThoughts":true}),
                    "{api:?}/{id}/{selected:?}"
                );
            }
            let controls = thinking::controls(&model).expect("known controls");
            assert!(!controls.contains(&"off".into()));
            assert!(!controls.contains(&"max".into()));
            assert_eq!(controls.contains(&"xhigh".into()), maximum == "XHIGH");
        }
    }
}

#[test]
fn gemini_25_budgets_have_model_specific_minima_ceilings_and_disable_rules() {
    use ThinkingLevel::{High, Low, Max, Medium, Minimal, Off, XHigh};
    for api in APIS {
        for (id, minimum, high, off) in [
            ("gemini-2.5-pro", 128, 32768, 128),
            ("gemini-2.5-flash", 128, 24576, 0),
            ("gemini-2.5-flash-lite", 512, 24576, 0),
        ] {
            let model = model(api, id);
            for selected in [None, Some(Off)] {
                assert_eq!(
                    config(&model, selected),
                    json!({"thinkingBudget":off}),
                    "{id}"
                );
            }
            for (selected, expected) in [
                (Minimal, minimum),
                (Low, 2048),
                (Medium, 8192),
                (High, high),
                (XHigh, high),
                (Max, high),
            ] {
                assert_eq!(
                    config(&model, Some(selected)),
                    json!({"thinkingBudget":expected,"includeThoughts":true}),
                    "{id}/{selected:?}"
                );
            }
            let controls = thinking::controls(&model).expect("known controls");
            assert_eq!(controls.contains(&"off".into()), off == 0);
            assert!(!controls.contains(&"xhigh".into()));
            assert!(!controls.contains(&"max".into()));
        }
    }
}

#[test]
fn uppercase_gemma_mappings_and_explicit_capability_restrictions_are_preserved() {
    for api in APIS {
        let mut model = model(api, "gemma-4-31b-it");
        model.thinking = serde_json::from_value(
            json!({"off":null,"minimal":"MINIMAL","low":null,"medium":null,"high":"HIGH"}),
        )
        .expect("map");
        assert_eq!(
            thinking::controls(&model).expect("controls"),
            ["minimal", "high"]
        );
        assert_eq!(config(&model, None), json!({"thinkingLevel":"MINIMAL"}));
        assert_eq!(
            config(&model, Some(ThinkingLevel::Medium)),
            json!({"thinkingLevel":"HIGH","includeThoughts":true})
        );
        model.capabilities = Some(Capabilities {
            thinking_levels: Some(vec!["high".into()]),
            ..Default::default()
        });
        assert_eq!(thinking::controls(&model).expect("controls"), ["high"]);
        assert_eq!(config(&model, None), json!({"thinkingLevel":"HIGH"}));
        model
            .thinking
            .insert(ThinkingLevel::High, Some("not-a-native-level".into()));
        assert_eq!(thinking::controls(&model), Some(Vec::new()));
        assert!(config(&model, Some(ThinkingLevel::High)).is_null());
    }
}

#[test]
fn explicit_modes_support_custom_ids_without_guessing_from_unrelated_substrings() {
    for api in APIS {
        for id in [
            "m-1",
            "not-gemini-3-flash",
            "gemini-2.5-computer-use-preview-10-2025",
        ] {
            let model = model(api, id);
            assert_eq!(
                config(&model, Some(ThinkingLevel::High)),
                json!({"thinkingBudget":-1,"includeThoughts":true})
            );
        }
        let mut model = model(api, "private-model");
        model.compat = serde_json::from_value(json!({"google_thinking":"level"})).expect("mode");
        model
            .thinking
            .insert(ThinkingLevel::XHigh, Some("XHIGH".into()));
        assert_eq!(
            config(&model, Some(ThinkingLevel::XHigh)),
            json!({"thinkingLevel":"XHIGH","includeThoughts":true})
        );
        model.compat = serde_json::from_value(json!({"google_thinking":"budget"})).expect("mode");
        assert!(thinking::effective(&model, ThinkingLevel::XHigh).is_none());
        model
            .thinking
            .insert(ThinkingLevel::XHigh, Some("high".into()));
        assert_eq!(
            config(&model, Some(ThinkingLevel::XHigh)),
            json!({"thinkingBudget":-1,"includeThoughts":true})
        );
        assert!(
            serde_json::from_value::<crate::mind::provider::compat::Compat>(
                json!({"google_thinking":"invalid"})
            )
            .is_err()
        );
    }
}

#[test]
fn nonreasoning_and_explicit_empty_menus_emit_no_thinking_configuration() {
    for api in APIS {
        let mut model = model(api, "gemini-3-flash-preview");
        model.reasoning = false;
        for level in std::iter::once(None).chain(ThinkingLevel::ALL.map(Some)) {
            assert!(config(&model, level).is_null());
        }
        model.reasoning = true;
        model.capabilities = Some(Capabilities {
            thinking_levels: Some(Vec::new()),
            ..Default::default()
        });
        for level in std::iter::once(None).chain(ThinkingLevel::ALL.map(Some)) {
            assert!(config(&model, level).is_null());
        }
    }
}

#[test]
fn mapped_controls_are_used_once_and_do_not_mutate_caller_metadata() {
    for api in APIS {
        let mut model = model(api, "gemini-4-neon");
        model.thinking = serde_json::from_value(json!({"off":null,"minimal":null,"low":"HIGH","medium":null,"high":"low","xhigh":null,"max":null})).expect("map");
        let before = model.clone();
        assert_eq!(
            thinking::controls(&model).expect("controls"),
            ["low", "high"]
        );
        assert_eq!(config(&model, None), json!({"thinkingLevel":"HIGH"}));
        assert_eq!(
            config(&model, Some(ThinkingLevel::High)),
            json!({"thinkingLevel":"LOW","includeThoughts":true})
        );
        assert_eq!(model, before);
    }
}

#[test]
fn clamped_mandatory_reasoning_keeps_the_output_reservation_floor() {
    for api in APIS {
        for id in ["gemini-2.5-pro", "gemini-3-flash-preview", "gemini-4-neon"] {
            let model = model(api, id);
            let options = Options {
                thinking: Some(ThinkingLevel::Off),
                output_limit: Some(16),
                ..Default::default()
            };
            let body = adapter(api.as_str())
                .prepare(&model, &plain_context(), &options)
                .expect("prepared")
                .0;
            assert_eq!(body["generationConfig"]["maxOutputTokens"], 32000, "{id}");
            assert_eq!(body["__budget"]["minimum"], 32000, "{id}");
        }
        let model = model(api, "gemini-2.5-flash");
        let body = adapter(api.as_str())
            .prepare(
                &model,
                &plain_context(),
                &Options {
                    thinking: Some(ThinkingLevel::Off),
                    output_limit: Some(16),
                    ..Default::default()
                },
            )
            .expect("prepared")
            .0;
        assert_eq!(body["generationConfig"]["maxOutputTokens"], 16);
        assert_eq!(body["__budget"]["minimum"], 1);
    }
}
