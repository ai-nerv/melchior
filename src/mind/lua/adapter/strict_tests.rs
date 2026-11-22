use super::{support::*, *};
use crate::mind::model::{Tool, ToolStrictness};
use crate::mind::provider::{api::Schema, compat::Compat};
use serde_json::{Value, json};

mod deepseek;

fn context(strict: Option<ToolStrictness>, parameters: Value) -> Context {
    Context {
        tools: vec![Tool {
            name: "lookup".into(),
            description: "Lookup".into(),
            parameters,
            strict,
        }],
        ..plain_context()
    }
}

fn schema() -> Value {
    json!({"type":"object","properties":{"term":{"type":"string"},"count":{"type":"integer"}},"required":["term"]})
}

fn declared(api: &str, supported: bool) -> Model {
    let mut model = plain_model();
    model.api = serde_json::from_value(json!(api)).expect("API");
    model.compat = Some(Compat {
        supports_strict_mode: Some(supported),
        supports_strict_tools: Some(supported),
        ..Default::default()
    });
    model
}

#[test]
fn strict_tool_policy_reaches_each_supported_protocol_without_changing_the_source() {
    for api in [
        "openai-completions",
        "openai-responses",
        "azure-openai-responses",
        "openai-codex-responses",
        "anthropic-messages",
    ] {
        for policy in [ToolStrictness::Prefer, ToolStrictness::Require] {
            let context = context(Some(policy), schema());
            let before = context.clone();
            let (body, _) = adapter(api)
                .prepare(&declared(api, true), &context, &Options::default())
                .expect("request");
            assert_eq!(context, before);
            let tool = if api == "openai-completions" {
                &body["tools"][0]["function"]
            } else {
                &body["tools"][0]
            };
            assert_eq!(tool["strict"], true, "{api}: {body}");
            let parameters = if api == "anthropic-messages" {
                &tool["input_schema"]
            } else {
                &tool["parameters"]
            };
            assert_eq!(parameters["additionalProperties"], false);
            assert_eq!(parameters["required"], json!(["term", "count"]));
            assert_eq!(
                parameters["properties"]["count"]["anyOf"][1]["type"],
                "null"
            );
            assert!(!body.to_string().contains("__strict"));
        }
    }
}

#[test]
fn capability_is_not_consent_and_unsupported_strict_fields_are_omitted() {
    for api in [
        "openai-completions",
        "openai-responses",
        "anthropic-messages",
    ] {
        for supported in [false, true] {
            let context = context(None, schema());
            let (body, _) = adapter(api)
                .prepare(&declared(api, supported), &context, &Options::default())
                .expect("legacy");
            let tool = if api == "openai-completions" {
                &body["tools"][0]["function"]
            } else {
                &body["tools"][0]
            };
            assert_eq!(
                tool.get("strict"),
                (supported && api != "anthropic-messages").then_some(&Value::Bool(false))
            );
            let parameters = if api == "anthropic-messages" {
                &tool["input_schema"]
            } else {
                &tool["parameters"]
            };
            assert_eq!(parameters, &schema());
        }
    }
}

#[test]
fn prefer_degrades_but_require_refuses_unsupported_dialects_and_schema_constructs() {
    for api in [
        "openai-completions",
        "openai-responses",
        "anthropic-messages",
        "google-generative-ai",
        "google-vertex",
        "pi-messages",
        "decisions",
    ] {
        for (supported, parameters) in [
            (false, schema()),
            (true, json!({"type":"object","$ref":"private-sentinel"})),
        ] {
            for policy in [ToolStrictness::Prefer, ToolStrictness::Require] {
                let result = adapter(api).prepare(
                    &declared(api, supported),
                    &context(Some(policy), parameters.clone()),
                    &Options::default(),
                );
                if policy == ToolStrictness::Require {
                    let error = result.expect_err("required policy");
                    assert!(error.contains("requires strict arguments"));
                    assert!(!error.contains("private-sentinel"));
                } else {
                    let (body, _) = result.expect("preferred fallback");
                    assert!(!body.to_string().contains("\"strict\":true"));
                    assert!(!body.to_string().contains("prefer"));
                }
            }
        }
    }
}

#[test]
fn anthropic_constraints_and_structured_answer_conflicts_are_checked_before_conversion() {
    let context = context(
        Some(ToolStrictness::Require),
        json!({"type":"object","properties":{"n":{"type":"integer","minimum":0}}}),
    );
    assert!(
        adapter("anthropic-messages")
            .prepare(
                &declared("anthropic-messages", true),
                &context,
                &Options::default()
            )
            .is_err()
    );
    assert!(
        adapter("openai-completions")
            .prepare(
                &declared("openai-completions", true),
                &context,
                &Options::default()
            )
            .is_ok()
    );
    let options = Options {
        schema: Some(Schema {
            name: "answer".into(),
            schema: schema(),
        }),
        ..Default::default()
    };
    let context = super::strict_tests::context(Some(ToolStrictness::Require), schema());
    assert!(
        adapter("anthropic-messages")
            .prepare(&declared("anthropic-messages", true), &context, &options)
            .is_err()
    );
    let (body, _) = adapter("openai-completions")
        .prepare(&declared("openai-completions", true), &context, &options)
        .expect("independent schema");
    assert_eq!(body["response_format"]["json_schema"]["strict"], true);
}

#[test]
fn strict_compatibility_inherits_fieldwise_and_cards_advertise_effective_support() {
    let base = Compat {
        supports_strict_mode: Some(true),
        supports_strict_tools: Some(true),
        ..Default::default()
    };
    let merged = Compat {
        supports_strict_mode: Some(false),
        ..Default::default()
    }
    .over(Some(base));
    assert!(!merged.resolve().supports_strict_mode);
    assert!(merged.resolve().supports_strict_tools);
    for (api, expected) in [
        ("openai-completions", true),
        ("anthropic-messages", true),
        ("google-generative-ai", false),
        ("bedrock-converse-stream", false),
    ] {
        let model = declared(api, true);
        assert_eq!(
            crate::mind::provider::thinking::capabilities(&model)
                .expect("caps")
                .strict_tools,
            Some(expected)
        );
    }
}

#[test]
fn a_strict_fallback_changes_the_content_free_provider_policy_fingerprint() {
    let adapter = adapter("anthropic-messages");
    let model = declared("anthropic-messages", true);
    let context = context(Some(ToolStrictness::Prefer), schema());
    let (first_body, first_frame) = adapter
        .prepare(&model, &context, &Options::default())
        .expect("strict request");
    let options = Options {
        disable_preferred_strict: true,
        ..Default::default()
    };
    let (next_body, next_frame) = adapter
        .prepare(&model, &context, &options)
        .expect("fallback request");
    assert_ne!(first_body, next_body);
    let first = first_frame.expect("first frame");
    let next = next_frame.expect("next frame");
    assert_eq!(first.dialect, next.dialect);
    assert_ne!(first.policy, next.policy);
    assert!(!first.disable_preferred_strict);
    assert!(next.disable_preferred_strict);
}
