use super::*;
use melchior::mind::model::{Context, Message, ThinkingLevel, Tool, ToolStrictness};
use melchior::mind::wire::{Ask, Schema, Wants};
use std::collections::BTreeSet;

const PLANS: [(&str, &str, &str, usize); 4] = [
    (
        "qwen-token-plan",
        "QWEN_TOKEN_PLAN_TEAM_API_KEY",
        "QWEN_TOKEN_PLAN_API_KEY",
        20,
    ),
    (
        "qwen-token-plan-cn",
        "QWEN_TOKEN_PLAN_TEAM_CN_API_KEY",
        "QWEN_TOKEN_PLAN_CN_API_KEY",
        20,
    ),
    (
        "qwen-token-plan-individual",
        "QWEN_TOKEN_PLAN_INDIVIDUAL_API_KEY",
        "QWEN_TOKEN_PLAN_API_KEY",
        13,
    ),
    (
        "qwen-token-plan-individual-cn",
        "QWEN_TOKEN_PLAN_INDIVIDUAL_CN_API_KEY",
        "QWEN_TOKEN_PLAN_CN_API_KEY",
        13,
    ),
];

#[path = "qwen_foundation/history.rs"]
mod history;

#[path = "qwen_foundation/lifecycle.rs"]
mod lifecycle;

#[path = "qwen_foundation/cancellation.rs"]
mod cancellation;

fn rows(id: &str) -> Vec<Value> {
    let source = match id {
        "qwen-token-plan" => {
            include_str!("../../catalog/reference/pi-1.6.0/data/qwen-token-plan.json")
        }
        "qwen-token-plan-cn" => {
            include_str!("../../catalog/reference/pi-1.6.0/data/qwen-token-plan-cn.json")
        }
        "qwen-token-plan-individual" => {
            include_str!("../../catalog/reference/pi-1.6.0/data/qwen-token-plan-individual.json")
        }
        "qwen-token-plan-individual-cn" => {
            include_str!("../../catalog/reference/pi-1.6.0/data/qwen-token-plan-individual-cn.json")
        }
        _ => panic!("unknown fixture plan"),
    };
    serde_json::from_str::<Value>(source).expect("source")["openai-completions"]
        .as_object()
        .expect("models")
        .values()
        .cloned()
        .collect()
}

fn fixture(id: &str, primary: &str, fallback: &str, count: usize) -> (Scratch, TcpListener) {
    let dir = Scratch::new("melchior-qwen", "foundation");
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let url = format!(
        "http://{}/compatible-mode/v1",
        listener.local_addr().expect("address")
    );
    let rows = rows(id);
    assert_eq!(rows.len(), count);
    let manifest: Value =
        serde_json::from_str(include_str!("../provider-parity/manifest.json")).expect("manifest");
    let pinned = manifest["providers"]
        .as_array()
        .expect("providers")
        .iter()
        .find(|row| row["id"] == id)
        .expect("provider");
    assert_eq!(pinned["auth"]["environment"], json!([primary, fallback]));
    for row in &rows {
        assert_eq!(row["provider"], id);
        assert_eq!(row["baseUrl"], pinned["endpoint"]["base_url"]);
    }
    let keys: BTreeSet<_> = rows
        .iter()
        .map(|row| format!("chat:{}", row["id"].as_str().expect("id")))
        .collect();
    let models: Vec<_> = rows
        .iter()
        .map(|row| json!({"id":row["id"],"base_url":url}))
        .collect();
    let mut provider = presets::shipped(id);
    assert_eq!(provider["api"], "openai-completions");
    assert_eq!(provider["base_url"], pinned["endpoint"]["base_url"]);
    assert_eq!(
        provider["auth"],
        json!({"kind":"api-key","vars":[primary,fallback]})
    );
    assert_eq!(
        provider["compat"],
        json!({"max_tokens_field":"max_completion_tokens"})
    );
    assert_eq!(provider["discovery"], json!({"kind":"bundled"}));
    assert_eq!(provider["catalog"]["source"], "pi-1.6.0");
    assert_eq!(provider["catalog"]["provider"], id);
    let selection = provider["catalog"]["models"]
        .as_array()
        .expect("selections");
    assert_eq!(selection.len(), count);
    assert_eq!(
        selection
            .iter()
            .map(|key| key.as_str().expect("key").to_owned())
            .collect::<BTreeSet<_>>(),
        keys
    );
    assert!(provider.get("models").is_none());
    provider["base_url"] = json!(url);
    provider["models"] = json!(models);
    let encoded = serde_json::to_string(&provider.to_string()).expect("Lua JSON");
    std::fs::write(
        dir.join("providers.lua"),
        format!(
            r#"
        for _, id in ipairs({{"openrouter","ollama"}}) do
            melchior.provider(id,{{name=id,api="openai-completions",auth={{kind="none"}}}})
        end
        melchior.provider({id:?},melchior.json.decode_value({encoded}))
    "#
        ),
    )
    .expect("config");
    (dir, listener)
}

fn request(id: &str, row: &Value) -> Ask {
    let schema = json!({"type":"object","properties":{"value":{"type":"integer","maximum":18446744073709551615u64}},
        "required":["value"],"additionalProperties":false,"examples":[{},[],null]});
    Ask {
        model: format!("{id}/{}", row["id"].as_str().expect("id")),
        about: String::new(),
        wants: Wants {
            max_tokens: Some(128),
            schema: Some(Schema {
                name: "answer".into(),
                schema: schema.clone(),
            }),
            ..Default::default()
        },
        context: Context {
            system: Some("Return exact JSON".into()),
            messages: vec![Message::user("Return seven")],
            tools: vec![Tool {
                name: "echo".into(),
                description: "Echo a value".into(),
                parameters: schema,
                strict: Some(ToolStrictness::Require),
            }],
        },
    }
}

fn finished() -> String {
    concat!("data: {\"choices\":[{\"delta\":{\"content\":\"{\\\"value\\\":7}\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":7,\"prompt_tokens_details\":{\"cached_tokens\":40}}}\n\n",
        "data: [DONE]\n\n").into()
}

#[test]
fn all_66_shipped_records_list_offline_with_plan_region_keys_and_unknown_prices() {
    for (id, primary, fallback, count) in PLANS {
        let (dir, listener) = fixture(id, primary, fallback, count);
        let other = if id.ends_with("-cn") {
            "QWEN_TOKEN_PLAN_API_KEY"
        } else {
            "QWEN_TOKEN_PLAN_CN_API_KEY"
        };
        for (vars, ready) in [
            (vec![], false),
            (vec![(primary, "private-sentinel")], true),
            (vec![(fallback, "private-sentinel")], true),
            (
                vec![(primary, " \t\n"), (fallback, "private-sentinel")],
                true,
            ),
            (vec![(primary, " \t\n")], false),
            (vec![(other, "private-sentinel")], false),
        ] {
            let output = command(&dir)
                .envs(vars)
                .args(["models", "--refresh", "--json"])
                .output()
                .expect("models");
            assert!(output.status.success());
            for bytes in [&output.stdout, &output.stderr] {
                assert!(!String::from_utf8_lossy(bytes).contains("private-sentinel"));
            }
            let reply: Value = serde_json::from_slice(&output.stdout).expect("cards");
            assert_eq!(reply["ok"], true);
            let cards: Vec<_> = reply["result"]
                .as_array()
                .expect("models")
                .iter()
                .filter(|row| row["provider"] == id)
                .collect();
            assert_eq!(cards.len(), count);
            for row in rows(id) {
                let card = cards
                    .iter()
                    .find(|card| card["id"] == request(id, &row).model)
                    .expect("card");
                assert_eq!(card["ready"], ready);
                assert_eq!(card["setup"]["variables"], json!([[primary, fallback]]));
                assert_eq!(card["context_window"], row["contextWindow"]);
                assert_eq!(card["max_output"], row["maxTokens"]);
                assert_eq!(card["capabilities"]["input"], row["input"]);
                assert_eq!(card["capabilities"]["strict_tools"], true);
                assert!(card.get("pricing").is_none(), "{card}");
                if let Some(map) = row.get("thinkingLevelMap") {
                    let levels: Vec<_> = ThinkingLevel::ALL
                        .into_iter()
                        .filter(|level| match map.get(level.as_str()) {
                            Some(value) => value.is_string(),
                            None => *level != ThinkingLevel::XHigh,
                        })
                        .map(ThinkingLevel::as_str)
                        .collect();
                    assert_eq!(card["capabilities"]["thinking_levels"], json!(levels));
                }
            }
            presets::unasked(&listener);
        }
    }
}

#[test]
fn all_66_models_route_all_efforts_through_native_schema_tools_and_cache_accounting() {
    let mut requests = 0;
    for (id, primary, fallback, count) in PLANS {
        let (dir, listener) = fixture(id, primary, fallback, count);
        for row in rows(id) {
            for level in std::iter::once(None).chain(ThinkingLevel::ALL.into_iter().map(Some)) {
                let mut ask = request(id, &row);
                ask.wants.thinking = level;
                let before = serde_json::to_value(&ask).expect("snapshot");
                let server =
                    response_server(listener.try_clone().expect("listener"), 200, finished());
                let said =
                    presets::events(&ask_request(&dir, &[(primary, "private-sentinel")], &ask));
                let (headers, body) = server.join().expect("HTTP fixture");
                requests += 1;
                presets::stopped(&said, "end_turn");
                assert!(headers.starts_with("post /compatible-mode/v1/chat/completions "));
                assert!(headers.contains("authorization: bearer private-sentinel\r\n"));
                assert_eq!(headers.matches("authorization:").count(), 1);
                assert_eq!(body["model"], row["id"]);
                assert_eq!(body["max_completion_tokens"], 128);
                assert!(body.get("max_tokens").is_none());
                assert_eq!(body["stream"], true);
                assert_eq!(body["stream_options"]["include_usage"], true);
                let alias =
                    level.and_then(|level| match row["thinkingLevelMap"].get(level.as_str()) {
                        Some(Value::String(value)) => Some(value.as_str()),
                        Some(Value::Null) => None,
                        None if level != ThinkingLevel::XHigh => Some(level.as_str()),
                        _ => None,
                    });
                assert_eq!(
                    body.get("enable_thinking").and_then(Value::as_bool),
                    alias.map(|value| value != "off")
                );
                assert_eq!(
                    body.get("reasoning_effort").and_then(Value::as_str),
                    alias
                        .filter(|value| *value != "off")
                        .filter(|_| row["compat"]["supportsReasoningEffort"] == true)
                );
                assert!(body.get("thinking_budget").is_none());
                assert_eq!(body["tools"][0]["function"]["strict"], true);
                assert_eq!(
                    body["response_format"]["json_schema"]["schema"],
                    ask.wants.schema.as_ref().expect("schema").schema
                );
                assert_eq!(
                    body["tools"][0]["function"]["parameters"],
                    ask.context.tools[0].parameters
                );
                let usage = &said
                    .iter()
                    .rfind(|event| event["event"] == "spent")
                    .expect("usage")["usage"];
                assert_eq!(
                    (
                        usage["input"].as_u64(),
                        usage["output"].as_u64(),
                        usage["cache_read"].as_u64()
                    ),
                    (Some(60), Some(7), Some(40))
                );
                assert_eq!(serde_json::to_value(&ask).expect("unchanged"), before);
            }
        }
    }
    assert_eq!(requests, 528);
}

#[test]
fn plan_alias_precedence_and_missing_credentials_are_enforced_before_dispatch() {
    let mut refusals = 0;
    for (id, primary, fallback, count) in PLANS {
        let (dir, listener) = fixture(id, primary, fallback, count);
        let rows = rows(id);
        for row in &rows {
            let said = presets::events(&ask_request(&dir, &[], &request(id, row)));
            assert_eq!(said.len(), 1);
            assert_eq!(said[0]["event"], "failed");
            assert_eq!(said[0]["why"], "auth");
            presets::unasked(&listener);
            refusals += 1;
        }
        for (vars, expected) in [
            (
                vec![
                    (primary, "private-sentinel-primary"),
                    (fallback, "private-sentinel-fallback"),
                ],
                "primary",
            ),
            (vec![(fallback, "private-sentinel-fallback")], "fallback"),
            (
                vec![(primary, " \t\n"), (fallback, "private-sentinel-fallback")],
                "fallback",
            ),
        ] {
            let server = response_server(listener.try_clone().expect("listener"), 200, finished());
            let said = presets::events(&ask_request(&dir, &vars, &request(id, &rows[0])));
            presets::stopped(&said, "end_turn");
            let (headers, _) = server.join().expect("request");
            assert!(headers.contains(&format!(
                "authorization: bearer private-sentinel-{expected}\r\n"
            )));
            assert_eq!(headers.matches("authorization:").count(), 1);
        }
    }
    assert_eq!(refusals, 66);
}
