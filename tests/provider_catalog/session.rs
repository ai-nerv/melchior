use super::*;

#[test]
fn affinity_mapping_checks_types_dialects_and_the_actual_protocol() {
    for api in [
        "openai-completions",
        "anthropic-messages",
        "openai-responses",
    ] {
        for value in [
            json!(true),
            json!(false),
            json!(null),
            json!("true"),
            json!(1),
        ] {
            let mut raw = conversion::model();
            raw["api"] = json!(api);
            raw["compat"] = json!({"sendSessionAffinityHeaders":value});
            let record = compiler::convert(&raw).expect("convert");
            let supported = value.is_boolean() && (api != "openai-responses" || value == false);
            assert_eq!(
                record["pending"].as_array().expect("pending").is_empty(),
                supported
            );
            assert_eq!(
                record["candidate"]["compat"].get("send_session_affinity_headers"),
                supported.then_some(&value)
            );
        }
        for value in [
            json!("generic"),
            json!("openai"),
            json!("openrouter"),
            json!("unknown"),
            json!(null),
            json!(true),
        ] {
            let mut raw = conversion::model();
            raw["api"] = json!(api);
            raw["compat"] = json!({"sessionAffinityFormat":value});
            let record = compiler::convert(&raw).expect("convert");
            let supported = api != "openai-responses"
                && matches!(value.as_str(), Some("generic" | "openai" | "openrouter"));
            assert_eq!(
                record["pending"].as_array().expect("pending").is_empty(),
                supported
            );
            assert_eq!(
                record["candidate"]["compat"].get("session_affinity_format"),
                supported.then_some(&value)
            );
        }
    }
}

#[test]
fn all_463_chat_and_31_anthropic_flags_preserve_their_protocol_specific_dialects() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("catalog/reference/pi-1.6.0/data");
    let (mut chat, mut anthropic) = (0, 0);
    for file in std::fs::read_dir(root).expect("sources") {
        let file = file.expect("file");
        let raw: Value =
            serde_json::from_slice(&std::fs::read(file.path()).expect("source")).expect("JSON");
        for models in raw.as_object().expect("protocols").values() {
            for row in models.as_object().expect("models").values() {
                if row["type"] != "chat"
                    || row["compat"].get("sendSessionAffinityHeaders").is_none()
                {
                    continue;
                }
                let before = row.clone();
                let record = compiler::convert(row).expect("convert");
                if row["api"] == "openai-completions" {
                    assert_eq!(
                        record["candidate"]["compat"]["send_session_affinity_headers"],
                        true
                    );
                    assert!(
                        !record["pending"]
                            .as_array()
                            .expect("pending")
                            .contains(&json!("compat.sendSessionAffinityHeaders"))
                    );
                    if let Some(format) = row["compat"].get("sessionAffinityFormat") {
                        assert_eq!(
                            &record["candidate"]["compat"]["session_affinity_format"],
                            format
                        );
                    } else {
                        assert_eq!(
                            record["candidate"]["compat"]["session_affinity_format"],
                            if row["provider"] == "openrouter" {
                                "openrouter"
                            } else {
                                "openai"
                            }
                        );
                    }
                    chat += 1;
                } else {
                    assert_eq!(row["api"], "anthropic-messages");
                    assert_eq!(
                        record["candidate"]["compat"]["send_session_affinity_headers"],
                        true
                    );
                    assert!(
                        !record["pending"]
                            .as_array()
                            .expect("pending")
                            .contains(&json!("compat.sendSessionAffinityHeaders"))
                    );
                    assert_eq!(
                        record["candidate"]["compat"]["session_affinity_format"],
                        if row["provider"] == "openrouter" {
                            "openrouter"
                        } else {
                            "generic"
                        }
                    );
                    anthropic += 1;
                }
                assert_eq!(row, &before);
            }
        }
    }
    assert_eq!((chat, anthropic), (463, 31));
}

#[test]
fn pinned_detected_dialects_are_explicit_and_never_replace_explicit_or_invalid_formats() {
    for api in ["openai-completions", "anthropic-messages"] {
        for (provider, url, mut expected) in [
            ("openrouter", "https://fixture.invalid/v1", "openrouter"),
            ("fixture", "https://openrouter.ai/api/v1", "openrouter"),
            ("fireworks", "https://fixture.invalid/v1", "openai"),
        ] {
            if api == "anthropic-messages" && expected == "openai" {
                expected = "generic";
            }
            for format in [
                None,
                Some("generic"),
                Some("openai"),
                Some("openrouter"),
                Some("unknown"),
            ] {
                let mut raw = conversion::model();
                raw["api"] = json!(api);
                raw["provider"] = json!(provider);
                raw["baseUrl"] = json!(url);
                raw["compat"] = json!({"sendSessionAffinityHeaders":true});
                if let Some(format) = format {
                    raw["compat"]["sessionAffinityFormat"] = json!(format);
                }
                let record = compiler::convert(&raw).expect("convert");
                if format == Some("unknown") {
                    assert!(
                        record["candidate"]["compat"]
                            .get("session_affinity_format")
                            .is_none()
                    );
                    assert!(
                        record["pending"]
                            .as_array()
                            .expect("pending")
                            .contains(&json!("compat.sessionAffinityFormat"))
                    );
                } else {
                    assert_eq!(
                        record["candidate"]["compat"]["session_affinity_format"],
                        format.unwrap_or(expected)
                    );
                }
            }
        }
    }
}
