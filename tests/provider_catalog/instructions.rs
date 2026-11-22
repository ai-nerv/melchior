use super::*;

#[test]
fn positional_instruction_mapping_is_boolean_and_requires_a_capable_adapter() {
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
            raw["compat"] = json!({"supportsMidConvoSystemMessages":value});
            let converted = compiler::convert(&raw).expect("convert");
            let supported = value.is_boolean() && (api == "openai-completions" || value == false);
            assert_eq!(
                converted["pending"].as_array().expect("pending").is_empty(),
                supported
            );
            assert_eq!(
                converted["candidate"]["compat"].get("supports_mid_convo_system_messages"),
                supported.then_some(&value)
            );
        }
    }
}

#[test]
fn pinned_chat_instruction_flags_are_preserved_without_clearing_other_protocol_gaps() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("catalog/reference/pi-1.6.0/data");
    let mut count = 0;
    let mut retained = 0;
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
            if row["type"] != "chat"
                || row["compat"]
                    .get("supportsMidConvoSystemMessages")
                    .is_none()
            {
                continue;
            }
            let before = row.clone();
            let converted = compiler::convert(row).expect("convert");
            let pending = converted["pending"].as_array().expect("pending");
            if row["api"] == "openai-completions" {
                assert_eq!(
                    converted["candidate"]["compat"]["supports_mid_convo_system_messages"],
                    row["compat"]["supportsMidConvoSystemMessages"]
                );
                assert!(!pending.contains(&json!("compat.supportsMidConvoSystemMessages")));
                count += 1;
            } else {
                assert!(pending.contains(&json!("compat.supportsMidConvoSystemMessages")));
                retained += 1;
            }
            if row["provider"] == "deepseek" {
                assert_eq!(row["id"], "deepseek-v4-pro");
                assert!(pending.is_empty());
                assert_eq!(converted["candidate"]["thinking"], row["thinkingLevelMap"]);
            }
            assert_eq!(row, &before);
        }
    }
    assert_eq!((count, retained), (42, 73));
}
