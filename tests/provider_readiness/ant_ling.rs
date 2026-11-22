use super::*;
use melchior::mind::model::{Content, Context, Message, ThinkingLevel, Tool, ToolStrictness};
use melchior::mind::wire::{Ask, Schema, Wants};
use std::collections::BTreeSet;

#[path = "ant_ling/history.rs"]
mod history;
#[path = "ant_ling/lifecycle.rs"]
mod lifecycle;

const KEY: &str = "ANT_LING_API_KEY";
const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

fn rows() -> Vec<Value> {
    serde_json::from_str::<Value>(include_str!(
        "../../catalog/reference/pi-1.6.0/data/ant-ling.json"
    ))
    .expect("source")["openai-completions"]
        .as_object()
        .expect("models")
        .values()
        .cloned()
        .collect()
}

fn fixture() -> (Scratch, TcpListener) {
    let dir = Scratch::new("melchior-ant", "native");
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let url = format!("http://{}/v1", listener.local_addr().expect("address"));
    let rows = rows();
    assert_eq!(rows.len(), 3);
    let mut provider = presets::shipped("ant-ling");
    assert_eq!(provider["api"], "openai-completions");
    assert_eq!(provider["base_url"], "https://api.ant-ling.com/v1");
    assert_eq!(provider["auth"], json!({"kind":"api-key","vars":[KEY]}));
    assert_eq!(provider["discovery"], json!({"kind":"bundled"}));
    assert_eq!(provider["catalog"]["source"], "pi-1.6.0");
    assert_eq!(provider["catalog"]["provider"], "ant-ling");
    assert!(provider.get("compat").is_none());
    assert!(provider.get("models").is_none());
    let selection = provider["catalog"]["models"].as_array().expect("selection");
    assert_eq!(selection.len(), 3);
    assert_eq!(
        selection
            .iter()
            .map(|key| key.as_str().expect("key").to_owned())
            .collect::<BTreeSet<_>>(),
        rows.iter()
            .map(|row| format!("chat:{}", row["id"].as_str().expect("id")))
            .collect()
    );
    provider["base_url"] = json!(url);
    provider["models"] = json!(
        rows.iter()
            .map(|row| {
                assert_eq!(row["baseUrl"], "https://api.ant-ling.com/v1");
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
        melchior.provider("ant-ling",melchior.json.decode_value({encoded}))
    "#
        ),
    )
    .expect("fixture config");
    (dir, listener)
}

fn request(row: &Value) -> Ask {
    let schema = json!({"type":"object","properties":{"value":{"type":"integer","maximum":18446744073709551615u64}},
        "required":["value"],"additionalProperties":false,"examples":[{},[],null]});
    let mut user = Message::user("Return seven");
    user.content.push(Content::Image {
        data: PNG.into(),
        media_type: "image/png".into(),
    });
    Ask {
        model: format!("ant-ling/{}", row["id"].as_str().expect("id")),
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
            messages: vec![user],
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
    let effort = level
        .filter(|_| row["reasoning"] == true)
        .and_then(|level| row["thinkingLevelMap"][level.as_str()].as_str());
    assert_eq!(
        body.get("reasoning"),
        effort.map(|effort| json!({"effort":effort})).as_ref()
    );
    for field in [
        "reasoning_effort",
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
    assert_eq!(body["max_tokens"], 128);
    assert!(body.get("max_completion_tokens").is_none());
    assert!(body.get("store").is_none());
    assert_eq!(body["stream"], true);
    assert_eq!(body["stream_options"]["include_usage"], true);
    assert_eq!(body["messages"][0]["role"], "system");
    assert!(!body.to_string().contains(PNG));
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
fn all_models_list_offline_with_isolated_credentials_exact_prices_and_mapped_efforts() {
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
            .filter(|row| row["provider"] == "ant-ling")
            .collect();
        assert_eq!(cards.len(), 3);
        for row in rows() {
            let card = cards
                .iter()
                .find(|card| card["id"] == request(&row).model)
                .expect("card");
            assert_eq!(card["ready"], ready);
            assert_eq!(card["setup"]["variables"], json!([[KEY]]));
            assert_eq!(card["context_window"], row["contextWindow"]);
            assert_eq!(card["max_output"], row["maxTokens"]);
            assert_eq!(card["capabilities"]["input"], json!(["text"]));
            assert_eq!(card["capabilities"]["strict_tools"], true);
            assert_eq!(
                card["capabilities"]["thinking_levels"],
                if row["reasoning"] == true {
                    json!(["high", "xhigh"])
                } else {
                    json!([])
                }
            );
            assert_eq!(
                card["pricing"],
                json!({
                    "input":row["cost"]["input"].as_f64().expect("input").to_string(),
                    "output":row["cost"]["output"].as_f64().expect("output").to_string(),
                })
            );
        }
        presets::unasked(&listener);
    }
}

#[test]
fn every_effort_preserves_exact_schemas_image_fallback_caps_and_priced_usage() {
    let (dir, listener) = fixture();
    let mut requests = 0;
    for row in rows() {
        for level in std::iter::once(None).chain(ThinkingLevel::ALL.into_iter().map(Some)) {
            let mut ask = request(&row);
            ask.wants.thinking = level;
            let (said, body) = round(&dir, &listener, &row, &ask, finished());
            presets::stopped(&said, "end_turn");
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
            assert_eq!(
                usage["estimated_micros"],
                if row["id"] == "Ling-2.6-flash" { 1 } else { 8 }
            );
            requests += 1;
        }
    }
    assert_eq!(requests, 24);
}

#[test]
fn missing_keys_invalid_sessions_schemas_and_models_refuse_before_http() {
    let (dir, listener) = fixture();
    let mut refused = 0;
    for row in rows() {
        for variant in 0..3 {
            let mut ask = request(&row);
            if variant == 1 {
                ask.wants.session_id = Some("invalid\nheader".into());
            }
            if variant == 2 {
                ask.context.tools[0].parameters =
                    json!({"type":"object","additionalProperties":true});
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
    ask.model = "ant-ling/not-a-model".into();
    let said = presets::events(&ask_request(&dir, &[], &ask));
    assert_eq!(said.len(), 1);
    assert_eq!(said[0]["event"], "failed");
    presets::unasked(&listener);
    assert_eq!(refused, 9);
}

#[test]
fn every_model_cancels_after_partial_usage_with_transport_eof_and_no_success() {
    let (dir, listener) = fixture();
    let mut requests = 0;
    for row in rows() {
        let (headers, body) =
            super::cancellation::after_usage(&dir, &listener, KEY, &request(&row));
        wire(&row, &headers, &body);
        requests += 1;
    }
    assert_eq!(requests, 3);
}
