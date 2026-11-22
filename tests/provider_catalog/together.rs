use super::*;
use melchior::mind::lua::adapter::{LuaAdapter, engine_with_builtins};
use melchior::mind::model::{Context, ThinkingLevel};
use melchior::mind::provider::{
    api::{Adapter, Options},
    model::Model,
};

fn rows() -> Vec<Value> {
    serde_json::from_str::<Value>(include_str!(
        "../../catalog/reference/pi-1.6.0/data/together.json"
    ))
    .expect("pinned source")["openai-completions"]
        .as_object()
        .expect("models")
        .values()
        .cloned()
        .collect()
}

#[test]
fn all_sixteen_records_preserve_routes_prices_limits_and_nullable_controls() {
    let rows = rows();
    assert_eq!(rows.len(), 16);
    for row in rows {
        let original = row.clone();
        let converted = compiler::convert(&row).expect("convert");
        assert_eq!(converted["pending"], json!([]), "{converted}");
        let model = &converted["candidate"];
        assert_eq!(model["provider"], "together");
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
            model["compat"]["thinking_format"],
            row["compat"]["thinkingFormat"]
        );
        assert_eq!(model["compat"]["max_tokens_field"], "max_tokens");
        assert_eq!(
            model["compat"]["supports_reasoning_effort"],
            row["compat"]["supportsReasoningEffort"]
        );
        assert_eq!(model["capabilities"]["strict_tools"], false);
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
        assert_eq!(row, original);
    }
}

#[test]
fn all_pinned_controls_reach_the_correct_hybrid_or_effort_wire_fields() {
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
            requests += 1;
            let alias = level.and_then(|level| match row["thinkingLevelMap"].get(level.as_str()) {
                Some(Value::String(value)) => Some(value.as_str()),
                Some(Value::Null) => None,
                None if level != ThinkingLevel::XHigh => Some(level.as_str()),
                _ => None,
            });
            let mut options = Options::default();
            options.thinking = level;
            let body = adapter.request(&model, &Context::default(), &options);
            let toggle = (row["compat"]["thinkingFormat"] == "together")
                .then_some(alias)
                .flatten()
                .map(|value| json!({"enabled":value != "off"}));
            assert_eq!(
                body.get("reasoning"),
                toggle.as_ref(),
                "{} {level:?}: {body}",
                model.id
            );
            let effort = alias
                .filter(|value| *value != "off")
                .filter(|_| row["compat"]["supportsReasoningEffort"] == true);
            assert_eq!(body.get("reasoning_effort").and_then(Value::as_str), effort);
            assert!(body.get("thinking").is_none());
            assert!(body.get("enable_thinking").is_none());
            assert!(body.get("max_completion_tokens").is_none());
            assert!(body.get("max_tokens").is_some());
            assert_eq!(serde_json::to_value(&model).expect("model"), original);
        }
    }
    assert_eq!(requests, 128);
}
