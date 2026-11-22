use super::*;
use melchior::mind::lua::adapter::{LuaAdapter, engine_with_builtins};
use melchior::mind::model::{Context, ThinkingLevel};
use melchior::mind::provider::{
    api::{Adapter, Options},
    model::Model,
    thinking,
};

fn rows() -> Vec<Value> {
    serde_json::from_str::<Value>(include_str!(
        "../../catalog/reference/pi-1.6.0/data/ant-ling.json"
    ))
    .expect("source")["openai-completions"]
        .as_object()
        .expect("models")
        .values()
        .cloned()
        .collect()
}

#[test]
fn all_three_records_preserve_routes_prices_strictness_and_explicit_effort_menus() {
    let rows = rows();
    assert_eq!(rows.len(), 3);
    for row in rows {
        let original = row.clone();
        let converted = compiler::convert(&row).expect("convert");
        assert_eq!(converted["pending"], json!([]), "{converted}");
        let model = &converted["candidate"];
        for (native, pinned) in [
            ("id", "id"),
            ("name", "name"),
            ("provider", "provider"),
            ("api", "api"),
            ("base_url", "baseUrl"),
            ("reasoning", "reasoning"),
            ("input", "input"),
            ("context_window", "contextWindow"),
            ("max_tokens", "maxTokens"),
            ("thinking", "thinkingLevelMap"),
        ] {
            assert_eq!(model.get(native), row.get(pinned), "{native}");
        }
        assert_eq!(
            model["compat"],
            json!({"supports_store":false,"supports_reasoning_effort":false,"max_tokens_field":"max_tokens",
                "thinking_format":"ant-ling","supports_strict_mode":true})
        );
        assert_eq!(model["capabilities"]["strict_tools"], true);
        assert_eq!(
            model["capabilities"]["thinking_levels"],
            if row["reasoning"] == true {
                json!(["high", "xhigh"])
            } else {
                json!([])
            }
        );
        for (native, pinned) in [
            ("input", "input"),
            ("output", "output"),
            ("cache_read", "cacheRead"),
            ("cache_write", "cacheWrite"),
        ] {
            assert_eq!(model["cost"][native].as_f64(), row["cost"][pinned].as_f64());
            assert_eq!(
                model["cost"]["presence"][native],
                row["cost"][pinned].as_f64().expect("rate") > 0.0
            );
        }
        assert_eq!(row, original);
    }
}

#[test]
fn every_default_and_effort_uses_only_explicit_pinned_aliases_and_never_claims_generic_max() {
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
            let effort = level
                .filter(|_| row["reasoning"] == true)
                .and_then(|level| row["thinkingLevelMap"][level.as_str()].as_str());
            let mut options = Options::default();
            options.thinking = level;
            options.max_tokens = Some(128);
            let body = adapter.request(&model, &Context::default(), &options);
            assert_eq!(
                body.get("reasoning"),
                effort.map(|effort| json!({"effort":effort})).as_ref()
            );
            assert_eq!(body["max_tokens"], 128);
            for field in [
                "reasoning_effort",
                "thinking",
                "enable_thinking",
                "chat_template_kwargs",
                "store",
                "max_completion_tokens",
            ] {
                assert!(body.get(field).is_none(), "{body}");
            }
            if let Some(level) = level {
                assert_eq!(thinking::effective(&model, level).as_deref(), effort);
            }
            assert_eq!(serde_json::to_value(&model).expect("unchanged"), original);
            requests += 1;
        }
    }
    assert_eq!(requests, 24);
}
