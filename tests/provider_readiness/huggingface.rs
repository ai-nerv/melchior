use super::*;
use melchior::mind::model::{Content, Context, Message, ThinkingLevel, Tool, ToolStrictness};
use melchior::mind::wire::{Ask, Schema, Wants};
use std::collections::BTreeSet;

#[path = "huggingface/history.rs"]
mod history;
#[path = "huggingface/lifecycle.rs"]
mod lifecycle;

const KEY: &str = "HF_TOKEN";
const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

fn rows() -> Vec<Value> {
    serde_json::from_str::<Value>(include_str!(
        "../../catalog/reference/pi-1.6.0/data/huggingface.json"
    ))
    .expect("source")["openai-completions"]
        .as_object()
        .expect("models")
        .values()
        .cloned()
        .collect()
}

fn fixture() -> (Scratch, TcpListener) {
    let dir = Scratch::new("melchior-hf", "foundation");
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let url = format!("http://{}/v1", listener.local_addr().expect("address"));
    let rows = rows();
    assert_eq!(rows.len(), 87);
    let manifest: Value =
        serde_json::from_str(include_str!("../provider-parity/manifest.json")).expect("manifest");
    let pinned = manifest["providers"]
        .as_array()
        .expect("providers")
        .iter()
        .find(|row| row["id"] == "huggingface")
        .expect("provider");
    assert_eq!(pinned["auth"]["environment"], json!([KEY]));
    assert_eq!(
        pinned["endpoint"]["base_url"],
        "https://router.huggingface.co/v1"
    );
    let keys: BTreeSet<_> = rows
        .iter()
        .map(|row| format!("chat:{}", row["id"].as_str().expect("id")))
        .collect();
    let models: Vec<_> = rows
        .iter()
        .map(|row| {
            assert_eq!(row["provider"], "huggingface");
            assert_eq!(row["api"], "openai-completions");
            assert_eq!(row["baseUrl"], pinned["endpoint"]["base_url"]);
            json!({"id":row["id"],"base_url":url})
        })
        .collect();
    let mut provider = presets::shipped("huggingface");
    assert_eq!(provider["api"], "openai-completions");
    assert_eq!(provider["base_url"], pinned["endpoint"]["base_url"]);
    assert_eq!(provider["auth"], json!({"kind":"api-key","vars":[KEY]}));
    assert_eq!(provider["discovery"], json!({"kind":"bundled"}));
    assert_eq!(provider["catalog"]["source"], "pi-1.6.0");
    assert_eq!(provider["catalog"]["provider"], "huggingface");
    let selection = provider["catalog"]["models"].as_array().expect("selection");
    assert_eq!(selection.len(), 87);
    assert_eq!(
        selection
            .iter()
            .map(|key| key.as_str().expect("key").to_owned())
            .collect::<BTreeSet<_>>(),
        keys
    );
    assert!(provider.get("models").is_none());
    assert!(provider.get("compat").is_none());
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
        melchior.provider("huggingface",melchior.json.decode_value({encoded}))
    "#
        ),
    )
    .expect("fixture config");
    (dir, listener)
}

fn request(row: &Value) -> Ask {
    let schema = json!({"type":"object","properties":{"value":{"type":"integer","maximum":18446744073709551615u64}},
        "required":["value"],"additionalProperties":false,"examples":[{},[],null]});
    let mut message = Message::user("Return seven");
    message.content.push(Content::Image {
        data: PNG.into(),
        media_type: "image/png".into(),
    });
    Ask {
        model: format!("huggingface/{}", row["id"].as_str().expect("id")),
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
            messages: vec![message],
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

fn controls(row: &Value, level: Option<ThinkingLevel>, body: &Value) {
    let alias = level
        .filter(|_| row["reasoning"] == true)
        .and_then(|level| match row["thinkingLevelMap"].get(level.as_str()) {
            Some(Value::String(value)) => Some(value.as_str()),
            Some(Value::Null) => None,
            None if level != ThinkingLevel::XHigh => Some(level.as_str()),
            _ => None,
        })
        .filter(|value| *value != "off");
    assert_eq!(
        body.get("reasoning_effort").and_then(Value::as_str),
        alias,
        "{} {level:?}: {body}",
        row["id"]
    );
    for field in [
        "reasoning",
        "thinking",
        "enable_thinking",
        "chat_template_kwargs",
    ] {
        assert!(body.get(field).is_none());
    }
}

fn wire(row: &Value, headers: &str, body: &Value) {
    assert!(headers.starts_with("post /v1/chat/completions "));
    assert!(headers.contains("authorization: bearer private-sentinel\r\n"));
    assert_eq!(headers.matches("authorization:").count(), 1);
    assert_eq!(body["model"], row["id"]);
    assert_eq!(body["max_completion_tokens"], 128);
    assert!(body.get("max_tokens").is_none());
    assert_eq!(body["store"], false);
    assert_eq!(body["stream"], true);
    assert_eq!(body["stream_options"]["include_usage"], true);
    assert_eq!(body["messages"][0]["role"], "system");
    let vision = row["input"]
        .as_array()
        .expect("input")
        .contains(&json!("image"));
    assert_eq!(body.to_string().contains(PNG), vision);
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
fn all_shipped_models_list_offline_with_isolated_token_prices_and_nullable_efforts() {
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
            .filter(|row| row["provider"] == "huggingface")
            .collect();
        assert_eq!(cards.len(), 87);
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
            assert_eq!(card["capabilities"]["strict_tools"], true);
            assert_eq!(
                card.get("pricing").is_some(),
                row["cost"]["input"].as_f64() != Some(0.0)
            );
            if let Some(map) = row.get("thinkingLevelMap") {
                let levels: Vec<_> = ThinkingLevel::ALL
                    .into_iter()
                    .filter(|level| match map.get(level.as_str()) {
                        Some(value) => value.is_string(),
                        None => *level != ThinkingLevel::XHigh,
                    })
                    .filter(|level| *level == ThinkingLevel::Off || row["reasoning"] == true)
                    .map(ThinkingLevel::as_str)
                    .collect();
                assert_eq!(card["capabilities"]["thinking_levels"], json!(levels));
            }
        }
        presets::unasked(&listener);
    }
}

#[test]
fn all_shipped_models_dispatch_every_effort_with_exact_schemas_images_and_usage() {
    let (dir, listener) = fixture();
    let mut requests = 0;
    for row in rows() {
        for level in std::iter::once(None).chain(ThinkingLevel::ALL.into_iter().map(Some)) {
            let mut ask = request(&row);
            ask.wants.thinking = level;
            let before = serde_json::to_value(&ask).expect("snapshot");
            let server = response_server(listener.try_clone().expect("listener"), 200, finished());
            let said = presets::events(&ask_request(&dir, &[(KEY, "private-sentinel")], &ask));
            let (headers, body) = server.join().expect("request");
            presets::stopped(&said, "end_turn");
            wire(&row, &headers, &body);
            controls(&row, level, &body);
            assert_eq!(body["tools"][0]["function"]["strict"], true);
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
            let input = price["input"].as_f64().expect("input");
            let cache = price["cacheRead"].as_f64().expect("cache");
            let cache = if cache > 0.0 { cache } else { input };
            let dollars = 60.0 / 1_000_000.0 * input
                + 7.0 / 1_000_000.0 * price["output"].as_f64().expect("output")
                + 40.0 / 1_000_000.0 * cache;
            let rounded: Value =
                serde_json::from_str(&format!("{:.0}", (dollars * 1_000_000.0).round()))
                    .expect("rounded estimate");
            let expected = if price
                .as_object()
                .expect("prices")
                .values()
                .all(|rate| rate.as_f64() == Some(0.0))
            {
                Value::Null
            } else {
                rounded
            };
            assert_eq!(usage["estimated_micros"], expected, "{}", row["id"]);
            assert_eq!(serde_json::to_value(&ask).expect("unchanged"), before);
            requests += 1;
        }
    }
    assert_eq!(requests, 696);
}

#[test]
fn missing_tokens_invalid_sessions_and_schemas_refuse_before_http_for_all_models() {
    let (dir, listener) = fixture();
    let mut refused = 0;
    for row in rows() {
        for variant in 0..3 {
            let mut ask = request(&row);
            if variant == 1 {
                ask.wants.session_id = Some("bad\nheader".into());
            }
            if variant == 2 {
                ask.context.tools[0].parameters = json!({"type":"object","properties":{"value":{"type":"integer"}},"additionalProperties":true});
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
    ask.model = "huggingface/not-a-model".into();
    let said = presets::events(&ask_request(&dir, &[], &ask));
    assert_eq!(said.len(), 1);
    assert_eq!(said[0]["event"], "failed");
    presets::unasked(&listener);
    assert_eq!(refused, 261);
}

#[test]
fn every_shipped_model_cancels_after_partial_usage_with_transport_eof_and_no_success() {
    let (dir, listener) = fixture();
    let mut requests = 0;
    for row in rows() {
        let (headers, body) =
            super::cancellation::after_usage(&dir, &listener, KEY, &request(&row));
        wire(&row, &headers, &body);
        requests += 1;
    }
    assert_eq!(requests, 87);
}
