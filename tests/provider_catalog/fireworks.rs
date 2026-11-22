use super::*;

#[test]
fn all_27_fireworks_records_keep_protocols_routes_nullable_efforts_and_prices() {
    let raw: Value = serde_json::from_str(include_str!(
        "../../catalog/reference/pi-1.6.0/data/fireworks.json"
    ))
    .expect("source");
    let (mut anthropic, mut chat) = (0, 0);
    for row in raw
        .as_object()
        .expect("protocols")
        .values()
        .flat_map(|rows| rows.as_object().expect("models").values())
    {
        let saved = row.clone();
        let converted = compiler::convert(row).expect("convert");
        assert_eq!(converted["pending"], json!([]), "{converted}");
        let candidate = &converted["candidate"];
        assert_eq!(candidate["id"], row["id"]);
        assert_eq!(candidate["api"], row["api"]);
        assert_eq!(candidate["base_url"], row["baseUrl"]);
        assert_eq!(candidate["input"], row["input"]);
        assert_eq!(candidate["context_window"], row["contextWindow"]);
        assert_eq!(candidate["max_tokens"], row["maxTokens"]);
        if let Some(map) = row.get("thinkingLevelMap") {
            assert_eq!(&candidate["thinking"], map);
        }
        assert_eq!(candidate["compat"]["send_session_affinity_headers"], true);
        if row["api"] == "anthropic-messages" {
            anthropic += 1;
            assert_eq!(candidate["base_url"], "https://api.fireworks.ai/inference");
            assert_eq!(candidate["compat"]["session_affinity_format"], "generic");
            assert_eq!(
                candidate["compat"]["supports_cache_control_on_tools"],
                false
            );
            assert_eq!(
                candidate["compat"]["supports_eager_tool_input_streaming"],
                false
            );
            assert_eq!(candidate["compat"]["anthropic_thinking"], "adaptive_effort");
            assert_eq!(candidate["compat"]["allow_empty_signature"], true);
        } else {
            chat += 1;
            assert_eq!(row["api"], "openai-completions");
            assert_eq!(
                candidate["base_url"],
                "https://api.fireworks.ai/inference/v1"
            );
            assert_eq!(candidate["compat"]["session_affinity_format"], "openai");
        }
        for (native, source) in [
            ("input", "input"),
            ("output", "output"),
            ("cache_read", "cacheRead"),
            ("cache_write", "cacheWrite"),
        ] {
            assert_eq!(
                candidate["cost"][native].as_f64(),
                row["cost"][source].as_f64()
            );
        }
        assert_eq!(row, &saved);
    }
    assert_eq!((anthropic, chat), (16, 11));
}

#[test]
fn tool_cache_and_eager_flags_require_boolean_anthropic_contracts() {
    for api in [
        "anthropic-messages",
        "openai-completions",
        "openai-responses",
    ] {
        for value in [
            json!(true),
            json!(false),
            json!(null),
            json!(0),
            json!("false"),
            json!([]),
        ] {
            for (source, native) in [
                (
                    "supportsCacheControlOnTools",
                    "supports_cache_control_on_tools",
                ),
                (
                    "supportsEagerToolInputStreaming",
                    "supports_eager_tool_input_streaming",
                ),
            ] {
                let mut row = conversion::model();
                row["api"] = json!(api);
                row["compat"] = json!({source:value});
                let before = row.clone();
                let converted = compiler::convert(&row).expect("convert");
                let supported = api == "anthropic-messages" && value.is_boolean();
                assert_eq!(
                    converted["candidate"]["compat"].get(native),
                    supported.then_some(&value)
                );
                assert_eq!(
                    converted["pending"]
                        .as_array()
                        .expect("pending")
                        .contains(&json!(format!("compat.{source}"))),
                    !supported
                );
                assert_eq!(row, before);
            }
        }
    }
}
