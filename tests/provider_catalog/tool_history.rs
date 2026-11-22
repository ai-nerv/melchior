use super::*;

#[test]
fn additions_mapping_is_boolean_and_only_chat_can_opt_into_native_positions() {
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
            raw["compat"] = json!({"supportsMidConvoToolAdditions":value});
            let converted = compiler::convert(&raw).expect("convert");
            let supported = value.is_boolean() && (api == "openai-completions" || value == false);
            assert_eq!(
                converted["pending"].as_array().expect("pending").is_empty(),
                supported
            );
            assert_eq!(
                converted["candidate"]["compat"].get("supports_mid_convo_tool_additions"),
                supported.then_some(&value)
            );
        }
    }
}

#[test]
fn all_eighteen_pinned_addition_flags_survive_without_hiding_other_gaps() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("catalog/reference/pi-1.6.0/data");
    let mut counts = BTreeMap::<String, usize>::new();
    for file in std::fs::read_dir(root).expect("inputs") {
        let source: Value =
            serde_json::from_slice(&std::fs::read(file.expect("file").path()).expect("read"))
                .expect("JSON");
        for row in source
            .as_object()
            .expect("apis")
            .values()
            .flat_map(|models| models.as_object().expect("models").values())
        {
            let Some(flag) = row["compat"].get("supportsMidConvoToolAdditions") else {
                continue;
            };
            let before = row.clone();
            let converted = compiler::convert(row).expect("convert");
            assert_eq!(row["api"], "openai-completions");
            assert_eq!(row["compat"]["supportsMidConvoSystemMessages"], true);
            assert_eq!(
                converted["candidate"]["compat"]["supports_mid_convo_tool_additions"],
                *flag
            );
            assert_eq!(
                converted["candidate"]["compat"]["supports_mid_convo_system_messages"],
                true
            );
            assert!(
                !converted["pending"]
                    .as_array()
                    .expect("pending")
                    .contains(&json!("compat.supportsMidConvoToolAdditions"))
            );
            let mut unknown = row.clone();
            unknown["compat"]["futureToolContract"] = json!(true);
            let future = compiler::convert(&unknown).expect("future");
            let mut expected = converted["pending"].as_array().expect("pending").clone();
            expected.push(json!("compat.futureToolContract"));
            expected.sort_by_key(Value::to_string);
            assert_eq!(future["pending"], json!(expected));
            assert_eq!(row, &before);
            *counts
                .entry(row["provider"].as_str().expect("provider").into())
                .or_default() += 1;
        }
    }
    assert_eq!(
        serde_json::to_value(counts).expect("counts"),
        json!({"fireworks":4,"kimi-coding":5,"moonshotai":3,"moonshotai-cn":3,"opencode":1,"opencode-go":2})
    );
}
