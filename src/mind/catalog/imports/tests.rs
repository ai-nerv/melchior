use super::*;
use crate::mind::catalog::declarations::assemble;
use crate::mind::provider::model::Api;
use serde_json::json;

fn declaration() -> Value {
    json!({"name":"Selected","api":"openai-responses","base_url":"https://api.groq.com/openai/v1",
        "auth":{"kind":"none"},"discovery":{"kind":"bundled"},
        "catalog":{"source":"pi-1.6.0","provider":"groq","models":["chat:llama-3.3-70b-versatile"]}})
}

#[test]
fn selected_models_preserve_native_metadata_protocols_and_the_containing_provider_identity() {
    let source = declaration();
    let provider = assemble("owner", &source).expect("selection");
    assert_eq!(provider.models.len(), 1);
    let model = &provider.models[0];
    assert_eq!(model.provider, "owner");
    assert_eq!(model.api, Api::OpenAiCompletions);
    assert_eq!(model.id, "llama-3.3-70b-versatile");
    assert_eq!(model.context_window, 131072);
    assert_eq!(model.max_tokens, 32768);
    assert_eq!(model.cost.input, 0.59);
    assert_eq!(model.cost.output, 0.79);
    assert_eq!(
        model.base_url.as_deref(),
        Some("https://api.groq.com/openai/v1")
    );
    let encoded = serde_json::to_string(&provider).expect("native provider");
    assert!(!encoded.contains("source_key") && !encoded.contains("catalog"));
    assert_eq!(source, declaration());
}

#[test]
fn explicit_model_fields_overlay_imports_nested_values_replace_and_new_models_append() {
    let mut source = declaration();
    source["catalog"]["models"] =
        json!(["chat:openai/gpt-oss-120b", "chat:llama-3.3-70b-versatile"]);
    source["models"] = json!([
        {"id":"openai/gpt-oss-120b","name":"Local name","base_url":"http://127.0.0.1:1234/v1",
            "cost":{"input":0,"output":0},"thinking":{"high":null},"headers":{"x-route":"local"}},
        {"id":"extra","name":"Extra","context_window":8192,"max_tokens":128}
    ]);
    let provider = assemble("owner", &source).expect("overlays");
    assert_eq!(
        provider
            .models
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
        ["openai/gpt-oss-120b", "llama-3.3-70b-versatile", "extra"]
    );
    let first = &provider.models[0];
    assert_eq!(first.name, "Local name");
    assert_eq!(first.thinking.len(), 1);
    assert_eq!(
        first.thinking.get(&crate::mind::model::ThinkingLevel::High),
        Some(&None)
    );
    assert_eq!(first.cost.input, 0.0);
    assert_eq!(first.cost.output, 0.0);
    assert_eq!(
        first.headers.get("x-route").map(String::as_str),
        Some("local")
    );
    assert_eq!(first.api, Api::OpenAiCompletions);
    assert_eq!(provider.models[2].api, Api::OpenAiResponses);
    assert!(
        crate::mind::provider::routing::destination(
            &provider,
            first,
            "http://127.0.0.1:1234/v1/chat/completions"
        )
        .is_err()
    );
}

#[test]
fn derived_capabilities_are_recomputed_after_inheritance_and_explicit_model_overrides() {
    let mut source = declaration();
    source["catalog"]["models"] = json!(["chat:openai/gpt-oss-120b"]);
    source["compat"] = json!({"supports_reasoning_effort":true});
    source["models"] = json!([{"id":"openai/gpt-oss-120b", "input":["text","image"],
        "thinking":{"xhigh":"high"},"compat":{"supports_strict_mode":false}}]);
    let provider = assemble("owner", &source).expect("overrides");
    let model = &provider.models[0];
    assert!(model.capabilities.is_none());
    let caps = crate::mind::provider::thinking::capabilities(model).expect("derived capabilities");
    assert_eq!(caps.input, ["text", "image"]);
    assert_eq!(caps.strict_tools, Some(false));
    assert!(
        caps.thinking_levels
            .expect("controls")
            .contains(&"xhigh".to_owned())
    );
    source["models"][0]["capabilities"] = json!({"thinking_levels":["high"]});
    let provider = assemble("owner", &source).expect("explicit restriction");
    let caps =
        crate::mind::provider::thinking::capabilities(&provider.models[0]).expect("capabilities");
    assert_eq!(caps.thinking_levels, Some(vec!["high".into()]));
}

#[test]
fn pending_unknown_nonchat_and_duplicate_selections_fail_atomically_without_echoing_values() {
    for (field, value) in [
        ("source", json!("private-sentinel")),
        ("provider", json!("private-sentinel")),
        ("models", json!(["chat:private-sentinel"])),
        (
            "models",
            json!([
                "chat:llama-3.3-70b-versatile",
                "chat:llama-3.3-70b-versatile"
            ]),
        ),
        ("models", json!([])),
        ("models", json!("private-sentinel")),
        ("unknown", json!("private-sentinel")),
    ] {
        let mut source = declaration();
        source["catalog"][field] = value;
        let before = source.clone();
        let error = apply(source.as_object_mut().expect("provider")).expect_err("refused");
        assert!(!error.contains("private-sentinel"));
        assert_eq!(source, before);
    }
    let mut pending = declaration();
    pending["catalog"]["provider"] = json!("openrouter");
    pending["catalog"]["models"] = json!(["chat:anthropic/claude-fable-5.1"]);
    let before = pending.clone();
    let error =
        apply(pending.as_object_mut().expect("provider")).expect_err("pending compatibility");
    assert!(error.contains("unresolved mappings"));
    assert_eq!(pending, before);
    let mut source = declaration();
    source["catalog"]["provider"] = json!("openai");
    source["catalog"]["models"] = json!(["classifier:gpt-6-luna"]);
    let error = assemble("owner", &source)
        .expect_err("classifier is not chat")
        .to_string();
    assert!(error.contains("operation"));
}

#[test]
fn qualified_image_profiles_survive_explicit_selection_without_activating_other_models() {
    let mut source = declaration();
    source["catalog"]["models"] = json!(["chat:qwen/qwen3.6-27b"]);
    let provider = assemble("owner", &source).expect("qualified image profile");
    assert_eq!(provider.models.len(), 1);
    assert_eq!(provider.models[0].id, "qwen/qwen3.6-27b");
    assert_eq!(provider.models[0].provider, "owner");
    assert_eq!(
        serde_json::to_value(&provider.models[0].input_limits).expect("limits"),
        json!({"images":{"resize":{"jpeg_quality":80,"max_bytes":4718592,"max_height":2000,"max_width":2000}}})
    );
}

#[test]
fn imported_metadata_does_not_skip_routing_price_or_api_validation() {
    for overrides in [
        json!([{"id":"llama-3.3-70b-versatile","headers":{"Authorization":"private-sentinel"}}]),
        json!([{"id":"llama-3.3-70b-versatile","api":"private-sentinel"}]),
        json!([{"id":"llama-3.3-70b-versatile","cost":{"input":-1}}]),
        json!([{"id":"llama-3.3-70b-versatile"},{"id":"llama-3.3-70b-versatile"}]),
        json!([{"name":"private-sentinel"}]),
    ] {
        let mut source = declaration();
        source["models"] = overrides;
        let error = assemble("owner", &source)
            .expect_err("invalid overlay")
            .to_string();
        assert!(!error.contains("private-sentinel"));
    }
    let mut source = declaration();
    source.as_object_mut().expect("provider").remove("catalog");
    let before = source.clone();
    apply(source.as_object_mut().expect("provider")).expect("legacy");
    assert_eq!(source, before);
}

#[test]
fn selectors_and_embedded_indexes_enforce_bounds_and_identity_integrity() {
    for keys in [
        json!(vec!["chat:llama-3.3-70b-versatile"; MOST + 1]),
        json!(["x".repeat(KEY_BYTES + 1)]),
    ] {
        let mut source = declaration();
        source["catalog"]["models"] = keys;
        assert!(assemble("bounded", &source).is_err());
    }
    assert!(Snapshot::parse(&" ".repeat(8 * 1024 * 1024 + 1)).is_err());
    let row = json!({"id":"m","source_key":"chat:m","operation":"chat","pending":[],
        "candidate":{"id":"m","provider":"p"}});
    let valid = json!({"schema":1,"reference":{"model_catalog":{"version":"1.6.0"}},
        "providers":[{"id":"p","models":[row.clone()]}]});
    assert!(Snapshot::parse(&valid.to_string()).is_ok());
    for mutation in 0..5 {
        let mut invalid = valid.clone();
        match mutation {
            0 => invalid["schema"] = json!(2),
            1 => invalid["providers"][0]["models"] = json!([row.clone(), row.clone()]),
            2 => invalid["providers"][0]["models"][0]["source_key"] = json!("classifier:m"),
            3 => invalid["providers"][0]["models"][0]["candidate"]["provider"] = json!("other"),
            _ => invalid["providers"]
                .as_array_mut()
                .expect("providers")
                .push(valid["providers"][0].clone()),
        }
        assert!(Snapshot::parse(&invalid.to_string()).is_err());
    }
}
