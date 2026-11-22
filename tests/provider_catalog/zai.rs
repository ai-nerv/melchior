use super::*;

#[test]
fn tool_stream_mapping_requires_a_boolean_and_a_chat_adapter_when_enabled() {
    for api in [
        "openai-completions",
        "openai-responses",
        "anthropic-messages",
        "google-generative-ai",
    ] {
        for value in [
            json!(true),
            json!(false),
            json!(null),
            json!(1),
            json!("true"),
            json!({}),
        ] {
            let mut raw = conversion::model();
            raw["api"] = json!(api);
            raw["compat"] = json!({"zaiToolStream":value});
            let before = raw.clone();
            let record = compiler::convert(&raw).expect("convert");
            let supported = value.is_boolean() && (api == "openai-completions" || value == false);
            assert_eq!(
                record["pending"].as_array().expect("pending").is_empty(),
                supported
            );
            assert_eq!(
                record["candidate"]["compat"].get("zai_tool_stream"),
                supported.then_some(&value)
            );
            assert_eq!(raw, before);
        }
    }
}

#[test]
fn all_eleven_pinned_zai_records_retain_strict_claim_and_complete_streaming_mapping() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("catalog/reference/pi-1.6.0/data");
    let mut total = 0;
    for id in ["zai", "zai-coding-cn"] {
        let raw: Value = serde_json::from_slice(
            &std::fs::read(root.join(format!("{id}.json"))).expect("source"),
        )
        .expect("JSON");
        for row in raw["openai-completions"]
            .as_object()
            .expect("models")
            .values()
        {
            let before = row.clone();
            let record = compiler::convert(row).expect("convert");
            assert_eq!(record["pending"], json!([]));
            assert_eq!(record["candidate"]["compat"]["zai_tool_stream"], true);
            assert_eq!(record["candidate"]["compat"]["supports_strict_mode"], true);
            assert_eq!(
                record["candidate"]["compat"]["supports_reasoning_effort"],
                row["compat"]["supportsReasoningEffort"]
            );
            assert_eq!(record["candidate"]["thinking"], row["thinkingLevelMap"]);
            assert_eq!(record["candidate"]["input"], row["input"]);
            assert_eq!(row, &before);
            total += 1;
        }
    }
    assert_eq!(total, 11);
}
