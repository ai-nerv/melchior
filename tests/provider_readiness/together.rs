use super::*;
use melchior::mind::model::{Context, Message, ThinkingLevel, Tool, ToolStrictness};
use melchior::mind::wire::{Ask, Schema, Wants};
use std::collections::BTreeSet;

#[path = "together/history.rs"]
mod history;
#[path = "together/lifecycle.rs"]
mod lifecycle;

const KEY: &str = "TOGETHER_API_KEY";

fn rows() -> Vec<Value> {
    serde_json::from_str::<Value>(include_str!(
        "../../catalog/reference/pi-1.6.0/data/together.json"
    ))
    .expect("source")["openai-completions"]
        .as_object()
        .expect("models")
        .values()
        .cloned()
        .collect()
}

fn fixture() -> (Scratch, TcpListener) {
    let dir = Scratch::new("melchior-together", "native");
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let url = format!("http://{}/v1", listener.local_addr().expect("address"));
    let rows = rows();
    assert_eq!(rows.len(), 16);
    let mut provider = presets::shipped("together");
    assert_eq!(provider["api"], "openai-completions");
    assert_eq!(provider["base_url"], "https://api.together.ai/v1");
    assert_eq!(provider["auth"], json!({"kind":"api-key","vars":[KEY]}));
    assert_eq!(provider["discovery"], json!({"kind":"bundled"}));
    assert_eq!(provider["catalog"]["source"], "pi-1.6.0");
    assert_eq!(provider["catalog"]["provider"], "together");
    assert!(provider.get("compat").is_none());
    assert!(provider.get("models").is_none());
    let selection = provider["catalog"]["models"].as_array().expect("selection");
    assert_eq!(selection.len(), 16);
    assert_eq!(
        selection
            .iter()
            .map(|v| v.as_str().expect("key").to_owned())
            .collect::<BTreeSet<_>>(),
        rows.iter()
            .map(|row| format!("chat:{}", row["id"].as_str().expect("id")))
            .collect()
    );
    provider["base_url"] = json!(url);
    provider["models"] = json!(
        rows.iter()
            .map(|row| {
                assert_eq!(row["baseUrl"], "https://api.together.ai/v1");
                json!({"id":row["id"],"base_url":url})
            })
            .collect::<Vec<_>>()
    );
    let encoded = serde_json::to_string(&provider.to_string()).expect("Lua JSON");
    std::fs::write(
        dir.join("providers.lua"),
        format!(
            r#"
        for _, id in ipairs({{"openrouter","ollama"}}) do
            melchior.provider(id,{{name=id,api="openai-completions",auth={{kind="none"}}}})
        end
        melchior.provider("together",melchior.json.decode_value({encoded}))
    "#
        ),
    )
    .expect("fixture config");
    (dir, listener)
}

fn request(row: &Value) -> Ask {
    let schema = json!({"type":"object","properties":{"value":{"type":"integer","maximum":18446744073709551615u64}},
        "required":["value"],"additionalProperties":false,"examples":[{},[],null]});
    Ask {
        model: format!("together/{}", row["id"].as_str().expect("id")),
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
                strict: Some(ToolStrictness::Prefer),
            }],
        },
    }
}

fn finished() -> String {
    concat!("data: {\"choices\":[{\"delta\":{\"content\":\"{\\\"value\\\":7}\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":7,\"prompt_tokens_details\":{\"cached_tokens\":40}}}\n\n",
        "data: [DONE]\n\n").into()
}

fn controls(row: &Value, level: Option<ThinkingLevel>, body: &Value) {
    let alias = level.and_then(|level| match row["thinkingLevelMap"].get(level.as_str()) {
        Some(Value::String(value)) => Some(value.as_str()),
        Some(Value::Null) => None,
        None if level != ThinkingLevel::XHigh => Some(level.as_str()),
        _ => None,
    });
    let toggle = (row["compat"]["thinkingFormat"] == "together")
        .then_some(alias)
        .flatten()
        .map(|value| json!({"enabled":value != "off"}));
    assert_eq!(
        body.get("reasoning"),
        toggle.as_ref(),
        "{} {level:?}: {body}",
        row["id"]
    );
    let effort = alias
        .filter(|value| *value != "off")
        .filter(|_| row["compat"]["supportsReasoningEffort"] == true);
    assert_eq!(body.get("reasoning_effort").and_then(Value::as_str), effort);
    assert!(body.get("thinking").is_none());
    assert!(body.get("enable_thinking").is_none());
}

fn wire(row: &Value, headers: &str, body: &Value) {
    assert!(headers.starts_with("post /v1/chat/completions "));
    assert!(headers.contains("authorization: bearer private-sentinel\r\n"));
    assert_eq!(headers.matches("authorization:").count(), 1);
    assert_eq!(body["model"], row["id"]);
    assert_eq!(body["max_tokens"], 128);
    assert!(body.get("max_completion_tokens").is_none());
    assert_eq!(body["stream"], true);
    assert_eq!(body["stream_options"]["include_usage"], true);
    assert!(body.get("store").is_none());
}

fn round(
    dir: &Scratch,
    listener: &TcpListener,
    row: &Value,
    ask: &Ask,
    reply: String,
) -> (Vec<Value>, Value) {
    let before = serde_json::to_value(ask).expect("snapshot");
    let server = response_server(listener.try_clone().expect("listener"), 200, reply);
    let said = presets::events(&ask_request(dir, &[(KEY, "private-sentinel")], ask));
    let (headers, body) = server.join().expect("request");
    wire(row, &headers, &body);
    controls(row, ask.wants.thinking, &body);
    assert_eq!(serde_json::to_value(ask).expect("unchanged"), before);
    (said, body)
}

#[test]
fn all_shipped_records_list_offline_with_exact_models_controls_and_isolated_credentials() {
    let (dir, listener) = fixture();
    for (vars, ready) in [
        (vec![], false),
        (vec![(KEY, "private-sentinel")], true),
        (vec![(KEY, " \t\n")], false),
        (vec![("OPENAI_API_KEY", "private-sentinel")], false),
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
            .filter(|row| row["provider"] == "together")
            .collect();
        assert_eq!(cards.len(), 16);
        for row in rows() {
            let card = cards
                .iter()
                .find(|card| card["id"] == request(&row).model)
                .expect("card");
            assert_eq!(card["ready"], ready);
            assert_eq!(card["setup"]["variables"], json!([[KEY]]));
            assert_eq!(card["context_window"], row["contextWindow"]);
            assert_eq!(card["max_output"], row["maxTokens"]);
            assert_eq!(card["capabilities"]["input"], row["input"]);
            assert_eq!(card["capabilities"]["strict_tools"], false);
            let levels: Vec<_> = ThinkingLevel::ALL
                .into_iter()
                .filter(|level| match row["thinkingLevelMap"].get(level.as_str()) {
                    Some(value) => value.is_string(),
                    None => *level != ThinkingLevel::XHigh,
                })
                .map(ThinkingLevel::as_str)
                .collect();
            assert_eq!(card["capabilities"]["thinking_levels"], json!(levels));
        }
        presets::unasked(&listener);
    }
}

#[test]
fn every_default_and_effort_preserves_json_caps_strict_fallback_and_priced_usage() {
    let (dir, listener) = fixture();
    let mut requests = 0;
    for row in rows() {
        for level in std::iter::once(None).chain(ThinkingLevel::ALL.into_iter().map(Some)) {
            let mut ask = request(&row);
            ask.wants.thinking = level;
            let (said, body) = round(&dir, &listener, &row, &ask, finished());
            requests += 1;
            presets::stopped(&said, "end_turn");
            assert!(body["tools"][0]["function"].get("strict").is_none());
            assert_eq!(
                body["tools"][0]["function"]["parameters"],
                ask.context.tools[0].parameters
            );
            assert_eq!(
                body["response_format"]["json_schema"]["schema"],
                ask.wants.schema.as_ref().expect("schema").schema
            );
            let usage = &said
                .iter()
                .rfind(|event| event["event"] == "spent")
                .expect("usage")["usage"];
            assert_eq!(usage["input"], 60);
            assert_eq!(usage["output"], 7);
            assert_eq!(usage["cache_read"], 40);
            assert_eq!(usage["cost_micros"], 0);
            let price = &row["cost"];
            let cache = price["cacheRead"].as_f64().expect("cache");
            let cache = if cache > 0.0 {
                cache
            } else {
                price["input"].as_f64().expect("input")
            };
            let estimate = price["input"].as_f64().expect("input") * 60.0
                + price["output"].as_f64().expect("output") * 7.0
                + cache * 40.0;
            let expected: Value = serde_json::from_str(&format!("{:.0}", estimate.round()))
                .expect("rounded estimate");
            assert_eq!(usage["estimated_micros"], expected);
        }
    }
    assert_eq!(requests, 128);
}

#[test]
fn missing_keys_required_strictness_invalid_sessions_and_unknown_models_refuse_before_http() {
    let (dir, listener) = fixture();
    let mut refused = 0;
    for row in rows() {
        let mut ask = request(&row);
        for variant in 0..3 {
            if variant == 1 {
                ask.context.tools[0].strict = Some(ToolStrictness::Require);
            }
            if variant == 2 {
                ask.context.tools[0].strict = Some(ToolStrictness::Prefer);
                ask.wants.session_id = Some("invalid\nheader".into());
            }
            let said = presets::events(&ask_request(&dir, &[], &ask));
            assert_eq!(said.len(), 1);
            assert_eq!(said[0]["event"], "failed");
            assert_eq!(
                said[0]["why"],
                if variant == 0 { "auth" } else { "invalid" }
            );
            presets::unasked(&listener);
            refused += 1;
        }
    }
    let mut ask = request(&rows()[0]);
    ask.model = "together/not-a-model".into();
    let said = presets::events(&ask_request(&dir, &[], &ask));
    assert_eq!(said.len(), 1);
    assert_eq!(said[0]["event"], "failed");
    presets::unasked(&listener);
    assert_eq!(refused, 48);
}
