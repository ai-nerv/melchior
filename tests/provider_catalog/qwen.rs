use super::*;
use melchior::mind::lua::adapter::{LuaAdapter, engine_with_builtins};
use melchior::mind::model::{Context, ThinkingLevel};
use melchior::mind::provider::{
    api::{Adapter, Options},
    model::Model,
};

fn sources() -> [(&'static str, usize, &'static str); 4] {
    [
        (
            "qwen-token-plan",
            20,
            include_str!("../../catalog/reference/pi-1.6.0/data/qwen-token-plan.json"),
        ),
        (
            "qwen-token-plan-cn",
            20,
            include_str!("../../catalog/reference/pi-1.6.0/data/qwen-token-plan-cn.json"),
        ),
        (
            "qwen-token-plan-individual",
            13,
            include_str!("../../catalog/reference/pi-1.6.0/data/qwen-token-plan-individual.json"),
        ),
        (
            "qwen-token-plan-individual-cn",
            13,
            include_str!(
                "../../catalog/reference/pi-1.6.0/data/qwen-token-plan-individual-cn.json"
            ),
        ),
    ]
}

#[test]
fn all_66_qwen_plan_records_preserve_routes_nulls_limits_and_unknown_plan_prices() {
    let mut total = 0;
    for (provider, count, source) in sources() {
        let raw: Value = serde_json::from_str(source).expect("source");
        let rows = raw["openai-completions"].as_object().expect("models");
        assert_eq!(rows.len(), count);
        for row in rows.values() {
            total += 1;
            let original = row.clone();
            let converted = compiler::convert(row).expect("convert");
            assert_eq!(converted["pending"], json!([]), "{converted}");
            let model = &converted["candidate"];
            assert_eq!(model["provider"], provider);
            assert_eq!(model["api"], "openai-completions");
            for (native, pinned) in [
                ("id", "id"),
                ("base_url", "baseUrl"),
                ("input", "input"),
                ("context_window", "contextWindow"),
                ("max_tokens", "maxTokens"),
                ("thinking", "thinkingLevelMap"),
            ] {
                assert_eq!(model.get(native), row.get(pinned), "{provider}/{native}");
            }
            assert_eq!(model["compat"]["thinking_format"], "qwen");
            assert_eq!(
                model["compat"]["supports_reasoning_effort"],
                row["compat"]["supportsReasoningEffort"]
            );
            assert_eq!(model["capabilities"]["strict_tools"], true);
            for field in ["input", "output", "cache_read", "cache_write"] {
                assert_eq!(model["cost"][field], 0.0);
                assert_eq!(model["cost"]["presence"][field], false);
            }
            assert_eq!(*row, original);
        }
    }
    assert_eq!(total, 66);
}

#[test]
fn all_66_models_send_only_the_pinned_efforts_without_inventing_budget_or_default_controls() {
    let adapter = LuaAdapter::new(
        engine_with_builtins().expect("protocols"),
        "openai-completions",
    )
    .expect("adapter");
    let mut requests = 0;
    for (provider, _, source) in sources() {
        let raw: Value = serde_json::from_str(source).expect("source");
        for row in raw["openai-completions"]
            .as_object()
            .expect("models")
            .values()
        {
            let model: Model = serde_json::from_value(
                compiler::convert(row).expect("convert")["candidate"].clone(),
            )
            .expect("model");
            let original = serde_json::to_value(&model).expect("model");
            for level in std::iter::once(None).chain(ThinkingLevel::ALL.into_iter().map(Some)) {
                requests += 1;
                let alias =
                    level.and_then(|level| match row["thinkingLevelMap"].get(level.as_str()) {
                        Some(Value::String(value)) => Some(value.as_str()),
                        Some(Value::Null) => None,
                        None if level != ThinkingLevel::XHigh => Some(level.as_str()),
                        _ => None,
                    });
                let mut options = Options::default();
                options.thinking = level;
                let body = adapter.request(&model, &Context::default(), &options);
                let enabled = alias.map(|alias| alias != "off");
                let effort = alias
                    .filter(|value| *value != "off")
                    .filter(|_| row["compat"]["supportsReasoningEffort"] == true);
                assert_eq!(
                    body.get("enable_thinking").and_then(Value::as_bool),
                    enabled,
                    "{provider}/{} {level:?}: {body}",
                    model.id
                );
                assert_eq!(
                    body.get("reasoning_effort").and_then(Value::as_str),
                    effort,
                    "{provider}/{} {level:?}: {body}",
                    model.id
                );
                assert!(body.get("thinking_budget").is_none());
                assert!(body.get("chat_template_kwargs").is_none());
                assert_eq!(body["model"], model.id);
                assert_eq!(serde_json::to_value(&model).expect("model"), original);
            }
        }
    }
    assert_eq!(requests, 528);
}
