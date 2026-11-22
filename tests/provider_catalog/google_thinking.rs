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
    ["google", "google-vertex", "opencode", "opencode-go"]
        .into_iter()
        .flat_map(|provider| {
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("catalog/reference/pi-1.6.0/data/{provider}.json"));
            let source: Value =
                serde_json::from_slice(&std::fs::read(path).expect("reference")).expect("JSON");
            source
                .as_object()
                .expect("APIs")
                .iter()
                .filter(|(api, _)| matches!(api.as_str(), "google-generative-ai" | "google-vertex"))
                .flat_map(|(_, rows)| rows.as_object().expect("rows").values().cloned())
                .collect::<Vec<_>>()
        })
        .collect()
}

fn expected(raw: &Value, selected: Option<ThinkingLevel>) -> Value {
    let requested = selected.unwrap_or(ThinkingLevel::Off);
    let names = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];
    let index = names
        .iter()
        .position(|name| *name == requested.as_str())
        .expect("index");
    let id = raw["id"].as_str().expect("id");
    let map = raw["thinkingLevelMap"].as_object();
    let pro = id == "gemini-2.5-pro";
    let known_budget = matches!(
        id,
        "gemini-2.5-pro" | "gemini-2.5-flash" | "gemini-2.5-flash-lite"
    );
    let available = |name: &str| {
        if let Some(map) = map {
            map.get(name)
                .and_then(Value::as_str)
                .map(str::to_ascii_lowercase)
        } else if known_budget {
            (!(matches!(name, "xhigh" | "max") || pro && name == "off")).then(|| name.to_owned())
        } else {
            (name != "xhigh").then(|| name.to_owned())
        }
    };
    let resolved = names[index..]
        .iter()
        .chain(names[..index].iter().rev())
        .find_map(|name| available(name))
        .expect("control");
    let mut config = if map.is_some() {
        json!({"thinkingLevel":resolved.to_ascii_uppercase()})
    } else {
        json!({"thinkingBudget":match resolved.as_str() {
            "off" => 0,
            _ if !known_budget => -1,
            "minimal" if id == "gemini-2.5-flash-lite" => 512,
            "minimal" => 128, "low" => 2048, "medium" => 8192,
            "high" if pro => 32768, "high" => 24576,
            _ => panic!("unexpected budget control"),
        }})
    };
    if selected.is_some_and(|level| level != ThinkingLevel::Off) && resolved != "off" {
        config["includeThoughts"] = json!(true);
    }
    config
}

#[test]
fn all_53_google_api_records_keep_metadata_and_emit_reviewed_controls_for_every_effort() {
    let rows = rows();
    assert_eq!(rows.len(), 53);
    let mut requests = 0;
    for raw in rows {
        let before = raw.clone();
        let converted = compiler::convert(&raw).expect("convert");
        assert_eq!(converted["pending"], json!([]), "{converted}");
        for (native, original) in [
            ("id", "id"),
            ("provider", "provider"),
            ("api", "api"),
            ("base_url", "baseUrl"),
            ("reasoning", "reasoning"),
            ("thinking", "thinkingLevelMap"),
        ] {
            assert_eq!(
                converted["candidate"].get(native),
                raw.get(original),
                "{native}"
            );
        }
        let model: Model = serde_json::from_value(converted["candidate"].clone()).expect("model");
        let adapter = LuaAdapter::new(engine_with_builtins().expect("engine"), model.api.as_str())
            .expect("adapter");
        for selected in std::iter::once(None).chain(ThinkingLevel::ALL.map(Some)) {
            let mut options = Options::default();
            options.thinking = selected;
            options.max_tokens = Some(128);
            let body = adapter.request(&model, &Context::default(), &options);
            assert_eq!(
                body["generationConfig"]["thinkingConfig"],
                expected(&raw, selected),
                "{}/{:?}",
                raw["id"],
                selected
            );
            assert_eq!(body["generationConfig"]["maxOutputTokens"], 128);
            assert!(body.get("__google_control").is_none());
            requests += 1;
        }
        assert_eq!(raw, before);
    }
    assert_eq!(requests, 424);
}

#[test]
fn invalid_google_mappings_remain_pending_instead_of_faking_coverage() {
    let mut raw = rows()
        .into_iter()
        .find(|row| row["id"] == "gemini-4-neon")
        .expect("neon");
    raw["thinkingLevelMap"]["high"] = json!("not-a-native-level");
    let converted = compiler::convert(&raw).expect("convert");
    assert_eq!(
        converted["pending"],
        json!(["thinkingLevelMap.high:adapter"])
    );
    assert_eq!(
        converted["candidate"]["thinking"]["high"],
        "not-a-native-level"
    );
    assert_eq!(
        converted["candidate"]["capabilities"]["thinking_levels"],
        json!(["low", "medium", "xhigh"])
    );
}
