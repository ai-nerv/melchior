use super::*;
use melchior::mind::lua::{adapter::LuaAdapter, engine::Engine};
use melchior::mind::model::{Context, Message, ThinkingLevel};
use melchior::mind::provider::{
    api::{Adapter, Options},
    model::Model,
};

#[test]
fn adaptive_and_unsigned_thinking_mappings_are_protocol_typed_and_exercised() {
    for api in [
        "anthropic-messages",
        "openai-completions",
        "openai-responses",
    ] {
        for value in [
            json!(true),
            json!(false),
            json!(null),
            json!("true"),
            json!(0),
        ] {
            let mut raw = conversion::model();
            raw["api"] = json!(api);
            raw["thinkingLevelMap"] = json!({"off":null,"minimal":null,"low":"low",
                "medium":"medium","high":"high","xhigh":"xhigh","max":"max"});
            raw["compat"] = json!({"forceAdaptiveThinking":value,"allowEmptySignature":value});
            let record = compiler::convert(&raw).expect("convert");
            let supported = api == "anthropic-messages" && value.is_boolean();
            let pending = record["pending"].as_array().expect("pending");
            for field in ["forceAdaptiveThinking", "allowEmptySignature"] {
                assert_eq!(
                    pending.contains(&json!(format!("compat.{field}"))),
                    !supported
                );
            }
            if !supported {
                continue;
            }
            let model: Model = serde_json::from_value(record["candidate"].clone()).expect("model");
            assert_eq!(
                record["candidate"]["compat"]["anthropic_thinking"],
                if value == true {
                    "adaptive_effort"
                } else {
                    "budget"
                }
            );
            assert_eq!(
                record["candidate"]["compat"]["allow_empty_signature"],
                value
            );
            assert_eq!(
                pending.contains(&json!("thinkingLevelMap.xhigh:adapter")),
                value == false
            );
            let mut engine = Engine::new();
            engine
                .run(include_str!("../../config/apis.lua"), "apis.lua")
                .expect("adapters");
            let adapter = LuaAdapter::new(engine, api).expect("adapter");
            for level in ThinkingLevel::ALL {
                let mut options = Options::default();
                options.thinking = Some(level);
                let body = adapter
                    .prepare(
                        &model,
                        &Context {
                            messages: vec![Message::user("hi")],
                            ..Default::default()
                        },
                        &options,
                    )
                    .expect("request")
                    .0;
                let supported = !matches!(level, ThinkingLevel::Off | ThinkingLevel::Minimal)
                    && (level != ThinkingLevel::XHigh || value == true);
                assert_eq!(
                    body.get("thinking").is_some(),
                    supported,
                    "{value}: {level:?}"
                );
                if supported && value == true {
                    assert_eq!(body["thinking"], json!({"type":"adaptive"}));
                    assert_eq!(body["output_config"], json!({"effort":level.as_str()}));
                } else {
                    assert!(body.get("output_config").is_none());
                }
            }
        }
    }
}

#[test]
fn both_pinned_minimax_flash_records_have_lossless_effort_candidates() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for id in ["minimax", "minimax-cn"] {
        let raw: Value = serde_json::from_slice(
            &std::fs::read(root.join(format!("catalog/reference/pi-1.6.0/data/{id}.json")))
                .expect("source"),
        )
        .expect("json");
        let row = &raw["anthropic-messages"]["chat:MiniMax-M3.1-Flash"];
        let record = compiler::convert(row).expect("convert");
        assert_eq!(record["pending"], json!([]));
        assert_eq!(record["candidate"]["thinking"], row["thinkingLevelMap"]);
        assert_eq!(
            record["candidate"]["capabilities"]["thinking_levels"],
            json!(["low", "medium", "high", "xhigh", "max"])
        );
        assert!(
            record["candidate"]["compat"]
                .get("anthropic_replay")
                .is_none()
        );
    }
}
