use super::*;
use melchior::mind::lua::{adapter::LuaAdapter, engine::Engine};
use melchior::mind::model::{Context, Message};
use melchior::mind::provider::{
    api::{Adapter, Options},
    model::Model,
};

#[test]
fn required_reasoning_mapping_is_boolean_and_chat_completions_only() {
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
            json!("true"),
            json!(1),
            json!({}),
        ] {
            let mut raw = conversion::model();
            raw["api"] = json!(api);
            raw["compat"] = json!({"requiresReasoningContentOnAssistantMessages":value});
            let converted = compiler::convert(&raw).expect("convert");
            let supported = api == "openai-completions" && value.is_boolean();
            assert_eq!(
                converted["pending"].as_array().expect("pending").is_empty(),
                supported
            );
            if supported {
                assert_eq!(
                    converted["candidate"]["compat"]["requires_reasoning_content_on_assistant_messages"],
                    value
                );
                let model: Model =
                    serde_json::from_value(converted["candidate"].clone()).expect("model");
                let mut engine = Engine::new();
                engine
                    .run(include_str!("../../config/apis.lua"), "apis.lua")
                    .expect("adapters");
                let wire = LuaAdapter::new(engine, api).expect("adapter");
                let body = wire.request(
                    &model,
                    &Context {
                        messages: vec![Message::assistant("answer")],
                        ..Context::default()
                    },
                    &Options::default(),
                );
                assert_eq!(
                    body["messages"][0].get("reasoning_content").is_some(),
                    value == true
                );
            } else {
                assert!(
                    converted["candidate"]["compat"]
                        .get("requires_reasoning_content_on_assistant_messages")
                        .is_none()
                );
            }
        }
    }
}

#[test]
fn all_nine_pinned_xiaomi_records_convert_without_mutating_source_or_claiming_free_plans() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut total = 0;
    for id in [
        "xiaomi",
        "xiaomi-token-plan-cn",
        "xiaomi-token-plan-ams",
        "xiaomi-token-plan-sgp",
    ] {
        let raw: Value = serde_json::from_slice(
            &std::fs::read(root.join(format!("catalog/reference/pi-1.6.0/data/{id}.json")))
                .expect("source"),
        )
        .expect("JSON");
        for row in raw["openai-completions"]
            .as_object()
            .expect("models")
            .values()
        {
            let saved = row.clone();
            let converted = compiler::convert(row).expect("convert");
            assert_eq!(converted["pending"], json!([]));
            assert_eq!(converted["candidate"]["thinking"], row["thinkingLevelMap"]);
            assert_eq!(
                converted["candidate"]["capabilities"]["thinking_levels"],
                json!(["off", "high"])
            );
            assert_eq!(
                converted["candidate"]["compat"]["requires_reasoning_content_on_assistant_messages"],
                true
            );
            if id != "xiaomi" {
                assert!(converted["candidate"].get("pricing").is_none());
                let cost: melchior::mind::model::Cost =
                    serde_json::from_value(converted["candidate"]["cost"].clone()).expect("cost");
                assert_eq!(cost, melchior::mind::model::Cost::default());
                assert_eq!(
                    converted["candidate"]["cost"]["presence"],
                    json!({
                        "version":1,"input":false,"output":false,"cache_read":false,"cache_write":false
                    })
                );
            }
            assert_eq!(row, &saved);
            total += 1;
        }
    }
    assert_eq!(total, 9);
}
