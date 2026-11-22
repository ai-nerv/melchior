use super::*;
use melchior::mind::{
    lua::adapter::{LuaAdapter, engine_with_builtins},
    model::{Context, ThinkingLevel},
    provider::{
        api::{Adapter, Options},
        model::Model,
    },
};

fn rows() -> Vec<Value> {
    serde_json::from_str::<Value>(include_str!(
        "../../catalog/reference/pi-1.6.0/data/mistral.json"
    ))
    .expect("source")["mistral-conversations"]
        .as_object()
        .expect("models")
        .values()
        .cloned()
        .collect()
}

fn expected(row: &Value, level: Option<ThinkingLevel>) -> Option<&str> {
    if row["reasoning"] != true {
        return None;
    }
    let map = row["thinkingLevelMap"].as_object().expect("mapped model");
    let Some(level) = level else {
        return map.get("off").and_then(Value::as_str);
    };
    let at = ThinkingLevel::ALL
        .iter()
        .position(|candidate| *candidate == level)
        .expect("level");
    ThinkingLevel::ALL
        .iter()
        .skip(at)
        .chain(ThinkingLevel::ALL.iter().take(at).rev())
        .find_map(|level| map.get(level.as_str()).and_then(Value::as_str))
}

#[test]
fn all_34_records_preserve_metadata_and_emit_native_controls_for_every_effort() {
    let rows = rows();
    assert_eq!(rows.len(), 34);
    let wire = LuaAdapter::new(
        engine_with_builtins().expect("builtins"),
        "mistral-conversations",
    )
    .expect("adapter");
    let mut requests = 0;
    for row in rows {
        let before = row.clone();
        let converted = compiler::convert(&row).expect("convert");
        assert_eq!(converted["pending"], json!([]), "{converted}");
        let candidate = &converted["candidate"];
        for (native, pinned) in [
            ("id", "id"),
            ("name", "name"),
            ("provider", "provider"),
            ("api", "api"),
            ("base_url", "baseUrl"),
            ("input", "input"),
            ("reasoning", "reasoning"),
            ("thinking", "thinkingLevelMap"),
            ("context_window", "contextWindow"),
            ("max_tokens", "maxTokens"),
        ] {
            assert_eq!(candidate.get(native), row.get(pinned), "{native}");
        }
        assert_eq!(
            candidate["compat"],
            json!({"supports_strict_mode":false,"supports_mid_convo_system_messages":true,"supports_prompt_cache_key":true,"send_session_affinity_headers":true})
        );
        assert_eq!(candidate["capabilities"]["strict_tools"], false);
        for (native, pinned) in [
            ("input", "input"),
            ("output", "output"),
            ("cache_read", "cacheRead"),
            ("cache_write", "cacheWrite"),
        ] {
            assert_eq!(
                candidate["cost"][native].as_f64(),
                row["cost"][pinned].as_f64()
            );
        }
        let model: Model = serde_json::from_value(candidate.clone()).expect("model");
        for selected in std::iter::once(None).chain(ThinkingLevel::ALL.map(Some)) {
            let mut options = Options::default();
            options.thinking = selected;
            options.max_tokens = Some(128);
            let body = wire.request(&model, &Context::default(), &options);
            assert_eq!(
                body.get("reasoning_effort").and_then(Value::as_str),
                expected(&row, selected),
                "{} {selected:?}",
                row["id"]
            );
            assert_eq!(body["max_tokens"], 128);
            assert!(body.get("prompt_mode").is_none());
            requests += 1;
        }
        assert_eq!(row, before);
    }
    assert_eq!(requests, 272);
}

#[test]
fn unknown_effort_values_stay_pending_and_are_not_advertised() {
    let mut row = rows()
        .into_iter()
        .find(|row| row["id"] == "mistral-large-4")
        .expect("model");
    row["thinkingLevelMap"]["high"] = json!("unreviewed");
    let converted = compiler::convert(&row).expect("convert");
    assert_eq!(
        converted["pending"],
        json!(["thinkingLevelMap.high:adapter"])
    );
    assert_eq!(
        converted["candidate"]["capabilities"]["thinking_levels"],
        json!(["low", "medium"])
    );
}
