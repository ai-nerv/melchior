use super::*;

#[test]
fn all_fifteen_baseten_records_keep_routes_templates_null_efforts_and_prices_without_source_rewrites()
 {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("catalog/reference/pi-1.6.0/data/baseten.json");
    let raw: Value = serde_json::from_slice(&std::fs::read(path).expect("source")).expect("JSON");
    let (mut count, mut templates) = (0, 0);
    for row in raw
        .as_object()
        .expect("protocols")
        .values()
        .flat_map(|rows| rows.as_object().expect("models").values())
    {
        let saved = row.clone();
        let converted = compiler::convert(row).expect("convert");
        assert!(
            converted["pending"].as_array().expect("pending").is_empty(),
            "{converted}"
        );
        let candidate = &converted["candidate"];
        assert_eq!(candidate["id"], row["id"]);
        assert_eq!(candidate["api"], row["api"]);
        assert_eq!(candidate["base_url"], row["baseUrl"]);
        assert_eq!(candidate["compat"]["session_affinity_format"], "openai");
        assert_eq!(candidate["compat"]["send_session_affinity_headers"], true);
        if let Some(thinking) = row.get("thinkingLevelMap") {
            assert_eq!(&candidate["thinking"], thinking);
        }
        if let Some(args) = row["compat"].get("chatTemplateArgs") {
            assert_eq!(&candidate["compat"]["chat_template_args"], args);
            assert_eq!(candidate["compat"]["thinking_format"], "baseten");
            templates += 1;
        }
        for field in ["input", "output", "cacheRead"] {
            let native = if field == "cacheRead" {
                "cache_read"
            } else {
                field
            };
            assert_eq!(
                candidate["cost"][native].as_f64(),
                row["cost"][field].as_f64()
            );
        }
        assert_eq!(row, &saved);
        count += 1;
    }
    assert_eq!((count, templates), (15, 2));
}

#[test]
fn templates_are_qualified_only_for_the_typed_baseten_chat_contract() {
    for api in [
        "openai-completions",
        "anthropic-messages",
        "openai-responses",
    ] {
        for format in ["baseten", "openai"] {
            for args in [
                json!({}),
                json!({"enable_thinking":false}),
                json!({"enable_thinking":{"$var":"thinking.enabled"}}),
                json!({"enable_thinking":{"$var":"thinking.budget"}}),
                json!({"enable_thinking":null}),
                json!({"enable_thinking":["thinking.enabled"]}),
                json!({"other":true}),
                json!([]),
                json!(true),
            ] {
                let mut raw = conversion::model();
                raw["api"] = json!(api);
                raw["compat"] = json!({"thinkingFormat":format,"chatTemplateArgs":args});
                let expected =
                    api == "openai-completions"
                        && format == "baseten"
                        && serde_json::from_value::<
                            melchior::mind::provider::compat::ChatTemplateArgs,
                        >(args.clone())
                        .is_ok();
                let record = compiler::convert(&raw).expect("convert");
                assert_eq!(
                    record["candidate"]["compat"].get("chat_template_args"),
                    expected.then_some(&args)
                );
                assert_eq!(
                    record["pending"]
                        .as_array()
                        .expect("pending")
                        .contains(&json!("compat.chatTemplateArgs")),
                    !expected
                );
                if api != "openai-completions" && format == "baseten" {
                    assert!(
                        record["pending"]
                            .as_array()
                            .expect("pending")
                            .contains(&json!("compat.thinkingFormat"))
                    );
                }
            }
        }
    }
}
