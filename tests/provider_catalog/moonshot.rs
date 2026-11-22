use super::*;

#[test]
fn eighteen_records_keep_exact_routes_prices_limits_and_null_efforts_without_runtime_overrides() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("catalog/reference/pi-1.6.0/data");
    let mut total = 0;
    for id in ["moonshotai", "moonshotai-cn", "kimi-coding"] {
        let raw: Value = serde_json::from_slice(
            &std::fs::read(root.join(format!("{id}.json"))).expect("source"),
        )
        .expect("JSON");
        let rows = raw["openai-completions"].as_object().expect("models");
        assert_eq!(rows.len(), 6);
        for row in rows.values() {
            let before = row.clone();
            let record = compiler::convert(row).expect("convert");
            assert_eq!(record["pending"], json!([]));
            let candidate = &record["candidate"];
            for (target, source) in [
                ("id", "id"),
                ("base_url", "baseUrl"),
                ("context_window", "contextWindow"),
                ("max_tokens", "maxTokens"),
                ("input", "input"),
            ] {
                assert_eq!(candidate[target], row[source]);
            }
            for (target, source) in [
                ("input", "input"),
                ("output", "output"),
                ("cache_read", "cacheRead"),
                ("cache_write", "cacheWrite"),
            ] {
                assert_eq!(
                    candidate["cost"][target]
                        .as_f64()
                        .expect("candidate rate")
                        .to_bits(),
                    row["cost"][source].as_f64().expect("source rate").to_bits()
                );
            }
            assert_eq!(candidate["compat"]["supports_strict_mode"], false);
            assert!(candidate["compat"].get("moonshot_keep_thinking").is_none());
            if let Some(thinking) = row.get("thinkingLevelMap") {
                assert_eq!(&candidate["thinking"], thinking);
                assert_eq!(candidate["thinking"]["off"], Value::Null);
            }
            assert_eq!(
                candidate["input_limits"]["images"]["resize"],
                json!({"max_width":2000,"max_height":2000,"max_bytes":4718592,"jpeg_quality":80})
            );
            assert_eq!(
                candidate["compat"].get("supports_mid_convo_tool_additions"),
                row["compat"].get("supportsMidConvoToolAdditions")
            );
            assert_eq!(row, &before);
            total += 1;
        }
    }
    assert_eq!(total, 18);
}
