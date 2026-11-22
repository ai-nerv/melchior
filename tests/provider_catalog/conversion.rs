use super::*;
use melchior::mind::model::{Cost, Usage};

pub(super) fn model() -> Value {
    json!({"id":"one","provider":"fixture","api":"openai-completions","type":"chat",
        "name":"One","baseUrl":"https://fixture.invalid/v1","reasoning":true,"input":["text"],
        "contextWindow":8192,"maxTokens":1024,
        "cost":{"input":3,"output":5,"cacheRead":0,"cacheWrite":0}})
}

#[test]
fn generated_candidates_are_deterministic_complete_and_not_activated() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let one = compiler::generate(root).expect("generation");
    let two = compiler::generate(root).expect("repeat generation");
    assert_eq!(one, two);
    assert_eq!(
        one,
        std::fs::read(root.join(compiler::OUTPUT)).expect("retained output")
    );
    let report: Value = serde_json::from_slice(&one).expect("report");
    assert_eq!(report["kind"], "model-candidates-not-runtime-catalog");
    assert_eq!(report["summary"]["providers"], 43);
    assert_eq!(
        report["summary"]["operations"],
        json!({"chat":1698,"classifier":81})
    );
    assert_eq!(report["summary"]["excluded_images"], 74);
    assert_eq!(report["summary"]["activated_providers"], 0);
    let providers = report["providers"].as_array().expect("providers");
    let mut total = 0;
    let mut tiers = 0;
    let mut headers = 0;
    for provider in providers {
        assert_eq!(provider["activation"], "not-reviewed");
        assert_eq!(
            provider["source"]["sha256"].as_str().expect("digest").len(),
            64
        );
        let mut identities = BTreeSet::new();
        for record in provider["models"].as_array().expect("records") {
            assert!(identities.insert(record["source_key"].as_str().expect("identity")));
            let operation = record["operation"].as_str().expect("operation");
            assert!(matches!(operation, "chat" | "classifier"));
            if operation == "classifier" {
                assert!(
                    record["pending"]
                        .as_array()
                        .expect("pending")
                        .contains(&json!("selection.operation"))
                );
            }
            if record["candidate"]["cost"].get("tiers").is_some() {
                tiers += 1;
            }
            if record["candidate"].get("headers").is_some() {
                headers += 1;
            }
            total += 1;
        }
    }
    assert_eq!((total, tiers, headers), (1779, 303, 48));
    let openai = providers
        .iter()
        .find(|provider| provider["id"] == "openai")
        .expect("OpenAI");
    let collision: BTreeSet<_> = openai["models"]
        .as_array()
        .expect("models")
        .iter()
        .filter(|record| record["id"] == "gpt-6-luna")
        .map(|record| record["source_key"].as_str().expect("identity"))
        .collect();
    assert_eq!(
        collision,
        BTreeSet::from(["chat:gpt-6-luna", "classifier:gpt-6-luna"])
    );
}

#[test]
fn conversion_preserves_protocol_routes_headers_thinking_and_compatibility() {
    let mut raw = model();
    raw["api"] = json!("openai-responses");
    raw["headers"] = json!({"X-Route":"selected"});
    raw["thinkingLevelMap"] = json!({"off":null,"low":"low","max":null});
    raw["compat"] =
        json!({"supportsStore":false,"supportsReasoningEffort":true,"maxTokensField":"max_tokens"});
    let record = compiler::convert(&raw).expect("converted");
    assert_eq!(record["pending"], json!([]));
    let candidate = &record["candidate"];
    assert_eq!(candidate["api"], "openai-responses");
    assert_eq!(candidate["base_url"], "https://fixture.invalid/v1");
    assert_eq!(candidate["headers"], raw["headers"]);
    assert_eq!(candidate["thinking"], raw["thinkingLevelMap"]);
    assert_eq!(candidate["compat"]["supports_store"], false);
    assert_eq!(candidate["compat"]["supports_reasoning_effort"], true);
    let model: melchior::mind::provider::model::Model =
        serde_json::from_value(candidate.clone()).expect("native model");
    assert_eq!(model.id, "one");
}

#[test]
fn disabled_long_retention_matches_qualified_protocols_without_enabling_long_ttl() {
    use melchior::mind::lua::{adapter::LuaAdapter, engine::Engine};
    use melchior::mind::model::{Context, Message};
    use melchior::mind::provider::api::{Adapter, Options};
    for api in [
        "openai-completions",
        "openai-responses",
        "anthropic-messages",
        "google-generative-ai",
    ] {
        for value in [
            json!(false),
            json!(true),
            json!(null),
            json!("false"),
            json!(0),
        ] {
            let mut raw = model();
            raw["api"] = json!(api);
            raw["compat"] = json!({"supportsLongCacheRetention":value});
            let record = compiler::convert(&raw).expect("cache compatibility");
            assert_eq!(
                record["pending"],
                if value == false && api != "google-generative-ai" {
                    json!([])
                } else {
                    json!(["compat.supportsLongCacheRetention"])
                }
            );
            assert_eq!(record["candidate"]["compat"], json!({}));
            if value == false && api != "google-generative-ai" {
                let mut engine = Engine::new();
                engine
                    .run(include_str!("../../config/apis.lua"), "apis.lua")
                    .expect("protocols");
                let adapter = LuaAdapter::new(engine, api).expect("adapter");
                let mut model: melchior::mind::provider::model::Model =
                    serde_json::from_value(record["candidate"].clone()).expect("model");
                if api == "anthropic-messages" {
                    model.compat = Some(melchior::mind::provider::compat::Compat {
                        prompt_cache: Some(
                            melchior::mind::provider::compat::PromptCache::AnthropicEphemeral,
                        ),
                        ..Default::default()
                    });
                }
                let body = adapter.request(
                    &model,
                    &Context {
                        messages: vec![Message::user("cache fixture")],
                        ..Default::default()
                    },
                    &Options::default(),
                );
                for field in [
                    "prompt_cache_retention",
                    "prompt_cache_key",
                    "cache_control",
                ] {
                    assert!(body.get(field).is_none(), "{api}: {body}");
                }
                assert!(!body.to_string().contains("\"ttl\""));
                if api == "anthropic-messages" {
                    assert_eq!(
                        body["messages"][0]["content"][0]["cache_control"],
                        json!({"type":"ephemeral"})
                    );
                }
            }
        }
    }
}

#[test]
fn xhigh_mapping_needs_adapter_support_and_null_stays_explicitly_unsupported() {
    let mut raw = model();
    raw["api"] = json!("openai-responses");
    raw["thinkingLevelMap"] = json!({"off":null,"minimal":null,"low":"low",
        "medium":null,"high":null,"xhigh":"xhigh","max":null});
    let record = compiler::convert(&raw).expect("effort model");
    assert_eq!(record["pending"], json!([]));
    assert_eq!(
        record["candidate"]["capabilities"]["thinking_levels"],
        json!(["low", "xhigh"])
    );
    raw["api"] = json!("anthropic-messages");
    let record = compiler::convert(&raw).expect("budget model");
    assert_eq!(record["pending"], json!(["thinkingLevelMap.xhigh:adapter"]));
    assert_eq!(
        record["candidate"]["capabilities"]["thinking_levels"],
        json!(["low"])
    );
    assert_eq!(record["candidate"]["thinking"]["xhigh"], "xhigh");
    raw["thinkingLevelMap"]["xhigh"] = Value::Null;
    let record = compiler::convert(&raw).expect("unsupported level");
    assert_eq!(record["pending"], json!([]));
    assert!(
        record["candidate"]["thinking"]
            .get("xhigh")
            .is_some_and(Value::is_null)
    );
}

#[test]
fn every_unmapped_field_is_visible_instead_of_silently_enabling_the_model() {
    let mut raw = model();
    raw["future"] = json!({"must":"keep"});
    raw["inputLimits"] = json!({"images":{"maxBytes":10}});
    raw["promptCache"] = json!("1h");
    raw["thinkingLevelMap"] = json!({"future-effort":null});
    raw["compat"] = json!({"supportsOpenAIGrammarTools":true,"thinkingFormat":"new-dialect"});
    raw["cost"]["futureRate"] = json!(2);
    let record = compiler::convert(&raw).expect("report unsupported metadata");
    let pending: BTreeSet<_> = record["pending"]
        .as_array()
        .expect("pending")
        .iter()
        .map(|value| value.as_str().expect("path"))
        .collect();
    assert_eq!(
        pending,
        BTreeSet::from([
            "future",
            "inputLimits.images.maxBytes",
            "promptCache",
            "thinkingLevelMap.future-effort",
            "compat.supportsOpenAIGrammarTools",
            "compat.thinkingFormat",
            "cost.futureRate"
        ])
    );
    raw["type"] = json!("image");
    assert!(compiler::convert(&raw).is_err());
    raw["type"] = json!("chat");
    raw["api"] = json!("unimplemented-protocol");
    assert!(
        compiler::convert(&raw).expect("pending API")["pending"]
            .as_array()
            .expect("pending")
            .contains(&json!("native_model_shape"))
    );
}

#[test]
fn snapshot_zero_rates_remain_unknown_and_tiers_use_actual_cached_context() {
    let mut raw = model();
    raw["cost"]["tiers"] =
        json!([{"inputTokensAbove":100,"input":6,"output":10,"cacheRead":0,"cacheWrite":0}]);
    let record = compiler::convert(&raw).expect("tier conversion");
    let cost: Cost =
        serde_json::from_value(record["candidate"]["cost"].clone()).expect("native cost");
    assert_eq!(record["pending"], json!([]));
    assert!(cost.presence.input && cost.presence.output);
    assert!(!cost.presence.cache_read && !cost.presence.cache_write);
    assert_eq!(cost.tiers.len(), 1);
    assert_eq!(cost.tiers[0].cache_read, None);
    let usage = Usage {
        input: 1,
        cache_read: 100,
        output: 1,
        ..Default::default()
    };
    assert!((usage.price(&cost) - 616.0 / 1_000_000.0).abs() < 1e-12);
    raw["cost"] = json!({"input":0,"output":0,"cacheRead":0,"cacheWrite":0});
    for free in [false, true] {
        raw["free"] = json!(free);
        let result = compiler::convert(&raw).expect("zero prices");
        let cost: Cost =
            serde_json::from_value(result["candidate"]["cost"].clone()).expect("native cost");
        assert_eq!(cost.presence.input, free);
        assert_eq!(cost.presence.output, free);
        assert_eq!(cost.presence.cache_read, free);
        assert_eq!(cost.presence.cache_write, free);
    }
    raw["cost"]["input"] = json!(3);
    assert!(
        compiler::convert(&raw).is_err(),
        "contradictory free pricing"
    );
    raw["free"] = json!(false);
    raw["cost"]["tiers"] = json!([{"inputTokensAbove":100,"input":0}]);
    let result = compiler::convert(&raw).expect("unrepresentable unknown tier override");
    assert_eq!(
        result["pending"],
        json!(["cost.tiers[0].input:unknown-override"])
    );
}

#[test]
fn malformed_schedules_and_thinking_maps_fail_generation() {
    for (field, value) in [
        ("cost", json!({"input":-1})),
        ("cost", json!({"input":"3"})),
        (
            "cost",
            json!({"tiers":[{"inputTokensAbove":1},{"inputTokensAbove":1}]}),
        ),
        ("cost", json!({"tiers":[{"inputTokensAbove":1.5}]})),
        (
            "cost",
            json!({"tiers":(0..65).map(|n|json!({"inputTokensAbove":n})).collect::<Vec<_>>()}),
        ),
        ("thinkingLevelMap", json!({"low":false})),
        ("free", json!("yes")),
    ] {
        let mut raw = model();
        raw[field] = value;
        assert!(compiler::convert(&raw).is_err(), "{field}");
    }
}

#[test]
fn source_reader_refuses_unpinned_files_counts_and_indirect_inputs() {
    let dir = melchior::scratch::Scratch::new("melchior-catalog", "inputs");
    let raw =
        serde_json::to_vec(&json!({"openai-completions":{"chat:one":model()}})).expect("JSON");
    let digest = format!("{:x}", Sha256::digest(&raw));
    let manifest = json!({"reference":{"model_catalog":{"records":{"chat":1}}},
        "providers":[{"id":"fixture","models":{"catalog":{
            "path":"package/dist/providers/data/fixture.json","sha256":digest,
            "records":{"chat":1},"apis":["openai-completions"]}}}]});
    let path = dir.join("fixture.json");
    std::fs::write(&path, &raw).expect("source");
    assert_eq!(
        compiler::input::read(&dir, &manifest).expect("read").len(),
        1
    );
    for pointer in [
        "/reference/model_catalog/records/chat",
        "/providers/0/models/catalog/records/chat",
    ] {
        let mut bad = manifest.clone();
        *bad.pointer_mut(pointer).expect("count") = json!(2);
        assert!(compiler::input::read(&dir, &bad).is_err());
    }
    let mut bad = manifest.clone();
    bad["providers"][0]["id"] = json!("../fixture");
    assert!(compiler::input::read(&dir, &bad).is_err());
    std::fs::write(dir.join("extra.json"), b"{}").expect("extra");
    assert!(compiler::input::read(&dir, &manifest).is_err());
    std::fs::remove_file(dir.join("extra.json")).expect("remove");
    std::fs::remove_file(&path).expect("remove");
    assert!(compiler::input::read(&dir, &manifest).is_err());
    let target = dir.join("other");
    std::fs::write(&target, &raw).expect("target");
    std::os::unix::fs::symlink(&target, &path).expect("symlink");
    assert!(compiler::input::read(&dir, &manifest).is_err());
}
