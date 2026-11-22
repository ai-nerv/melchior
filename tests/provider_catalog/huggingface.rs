use super::*;
use melchior::mind::lua::adapter::{LuaAdapter, engine_with_builtins};
use melchior::mind::model::{Context, ThinkingLevel};
use melchior::mind::provider::{
    api::{Adapter, Options},
    model::Model,
};

fn rows() -> Vec<Value> {
    serde_json::from_str::<Value>(include_str!(
        "../../catalog/reference/pi-1.6.0/data/huggingface.json"
    ))
    .expect("source")["openai-completions"]
        .as_object()
        .expect("models")
        .values()
        .cloned()
        .collect()
}

#[test]
fn all_eighty_seven_records_keep_metadata_and_reviewed_pi_wire_defaults() {
    let rows = rows();
    assert_eq!(rows.len(), 87);
    let (mut reasoning, mut vision, mut unknown) = (0, 0, 0);
    for row in rows {
        let original = row.clone();
        let converted = compiler::convert(&row).expect("convert");
        assert_eq!(converted["pending"], json!([]), "{converted}");
        let model = &converted["candidate"];
        assert_eq!(model["provider"], "huggingface");
        assert_eq!(model["api"], "openai-completions");
        for (native, pinned) in [
            ("id", "id"),
            ("base_url", "baseUrl"),
            ("input", "input"),
            ("context_window", "contextWindow"),
            ("max_tokens", "maxTokens"),
            ("thinking", "thinkingLevelMap"),
        ] {
            assert_eq!(model.get(native), row.get(pinned), "{native}");
        }
        assert_eq!(
            model["compat"],
            json!({"supports_strict_mode":true,"supports_reasoning_effort":true,
            "supports_store":true,"max_tokens_field":"max_completion_tokens"})
        );
        assert_eq!(model["capabilities"]["strict_tools"], true);
        for (native, pinned) in [
            ("input", "input"),
            ("output", "output"),
            ("cache_read", "cacheRead"),
            ("cache_write", "cacheWrite"),
        ] {
            assert_eq!(model["cost"][native].as_f64(), row["cost"][pinned].as_f64());
            assert_eq!(
                model["cost"]["presence"][native],
                row["cost"][pinned].as_f64().expect("price") > 0.0
            );
        }
        reasoning += usize::from(row["reasoning"] == true);
        vision += usize::from(
            row["input"]
                .as_array()
                .expect("input")
                .contains(&json!("image")),
        );
        unknown += usize::from(row["cost"]["input"].as_f64() == Some(0.0));
        assert_eq!(row, original);
    }
    assert_eq!((reasoning, vision, unknown), (66, 35, 1));
}

#[test]
fn every_model_and_level_serializes_efforts_without_losing_nulls_or_unknown_prices() {
    let adapter = LuaAdapter::new(
        engine_with_builtins().expect("protocols"),
        "openai-completions",
    )
    .expect("adapter");
    let mut requests = 0;
    for row in rows() {
        let converted = compiler::convert(&row).expect("convert");
        assert_eq!(converted["pending"], json!([]));
        let model: Model = serde_json::from_value(converted["candidate"].clone()).expect("model");
        let original = serde_json::to_value(&model).expect("model");
        for level in std::iter::once(None).chain(ThinkingLevel::ALL.into_iter().map(Some)) {
            let alias = level
                .filter(|_| model.reasoning)
                .and_then(|level| match row["thinkingLevelMap"].get(level.as_str()) {
                    Some(Value::String(value)) => Some(value.as_str()),
                    Some(Value::Null) => None,
                    None if level != ThinkingLevel::XHigh => Some(level.as_str()),
                    _ => None,
                })
                .filter(|value| *value != "off");
            let mut options = Options::default();
            options.thinking = level;
            options.max_tokens = Some(128);
            let body = adapter.request(&model, &Context::default(), &options);
            assert_eq!(
                body.get("reasoning_effort").and_then(Value::as_str),
                alias,
                "{} {level:?}: {body}",
                model.id
            );
            assert_eq!(body["max_completion_tokens"], 128);
            assert_eq!(body["store"], false);
            for field in [
                "reasoning",
                "thinking",
                "enable_thinking",
                "max_tokens",
                "chat_template_kwargs",
            ] {
                assert!(body.get(field).is_none());
            }
            assert_eq!(serde_json::to_value(&model).expect("model"), original);
            requests += 1;
        }
    }
    assert_eq!(requests, 696);
}

#[test]
fn reviewed_defaults_are_route_and_protocol_scoped_and_explicit_overrides_win() {
    let original = rows()
        .into_iter()
        .find(|row| row["id"] == "deepseek-ai/DeepSeek-V4.1-Flash")
        .expect("model");
    let mut row = original.clone();
    row.as_object_mut().expect("row").remove("compat");
    let converted = compiler::convert(&row).expect("missing compat");
    assert_eq!(
        converted["candidate"]["compat"]["supports_reasoning_effort"],
        true
    );
    row["compat"] = json!({"supportsStore":false,"supportsReasoningEffort":false,"maxTokensField":"max_tokens"});
    let converted = compiler::convert(&row).expect("override");
    assert_eq!(converted["candidate"]["compat"]["supports_store"], false);
    assert_eq!(
        converted["candidate"]["compat"]["supports_reasoning_effort"],
        false
    );
    assert_eq!(
        converted["candidate"]["compat"]["max_tokens_field"],
        "max_tokens"
    );
    assert!(
        converted["pending"]
            .as_array()
            .expect("pending")
            .contains(&json!("thinkingLevelMap.xhigh:adapter"))
    );
    for (field, value) in [
        ("provider", "unreviewed"),
        ("baseUrl", "https://unreviewed.invalid/v1"),
        ("api", "openai-responses"),
    ] {
        let mut row = original.clone();
        row[field] = json!(value);
        let converted = compiler::convert(&row).expect("unreviewed");
        let compat = &converted["candidate"]["compat"];
        for name in [
            "supports_store",
            "supports_reasoning_effort",
            "max_tokens_field",
        ] {
            assert!(compat.get(name).is_none(), "{field}: {compat}");
        }
    }
}
