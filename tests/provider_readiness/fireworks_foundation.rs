use super::*;
use melchior::mind::model::{Context, Message, ThinkingLevel, Tool, ToolStrictness};
use melchior::mind::wire::{Ask, Schema, Wants};
use std::collections::BTreeSet;

const KEY: &str = "FIREWORKS_API_KEY";

#[path = "fireworks_foundation/lifecycle.rs"]
mod lifecycle;

#[path = "fireworks_foundation/history.rs"]
mod history;

fn rows() -> Vec<Value> {
    let raw: Value = serde_json::from_str(include_str!(
        "../../catalog/reference/pi-1.6.0/data/fireworks.json"
    ))
    .expect("source");
    raw.as_object()
        .expect("protocols")
        .values()
        .flat_map(|models| models.as_object().expect("models").values().cloned())
        .collect()
}

fn anthropic(row: &Value) -> bool {
    row["api"] == "anthropic-messages"
}

fn fixture() -> (Scratch, TcpListener, Value) {
    let dir = Scratch::new("melchior-fireworks", "foundation");
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let url = format!(
        "http://{}/inference",
        listener.local_addr().expect("address")
    );
    let rows = rows();
    assert_eq!(rows.len(), 27);
    let models: Vec<_> = rows
        .iter()
        .map(|row| {
            json!({"id":row["id"],
        "base_url":if anthropic(row) { url.clone() } else { format!("{url}/v1") }})
        })
        .collect();
    let mut provider = presets::shipped("fireworks");
    assert_eq!(provider["api"], "anthropic-messages");
    assert_eq!(provider["base_url"], "https://api.fireworks.ai/inference");
    assert_eq!(provider["auth"], json!({"kind":"api-key","vars":[KEY]}));
    assert_eq!(provider["discovery"], json!({"kind":"bundled"}));
    assert_eq!(provider["catalog"]["source"], "pi-1.6.0");
    assert_eq!(provider["catalog"]["provider"], "fireworks");
    let selected: BTreeSet<_> = provider["catalog"]["models"]
        .as_array()
        .expect("selections")
        .iter()
        .map(|v| v.as_str().expect("key").to_owned())
        .collect();
    assert_eq!(
        selected,
        rows.iter()
            .map(|row| format!("chat:{}", row["id"].as_str().expect("ID")))
            .collect()
    );
    assert_eq!(
        provider["catalog"]["models"]
            .as_array()
            .expect("selections")
            .len(),
        27
    );
    assert!(provider.get("models").is_none());
    provider["base_url"] = json!(url);
    provider["models"] = json!(models);
    write(&dir, &provider);
    (dir, listener, provider)
}

fn write(dir: &Scratch, provider: &Value) {
    let encoded = serde_json::to_string(&provider.to_string()).expect("Lua string");
    std::fs::write(
        dir.join("providers.lua"),
        format!(
            r#"
        for _, id in ipairs({{"openrouter","ollama"}}) do
            melchior.provider(id,{{name=id,api="openai-completions",auth={{kind="none"}}}})
        end
        melchior.provider("fireworks",melchior.json.decode_value({encoded}))
    "#
        ),
    )
    .expect("fixture");
}

fn schema() -> Schema {
    Schema {
        name: "answer".into(),
        schema: json!({"type":"object",
        "properties":{"value":{"type":"integer","maximum":18446744073709551615u64}},
        "required":["value"],"additionalProperties":false,"examples":[{},[],null]}),
    }
}

fn request(row: &Value) -> Ask {
    Ask {
        model: format!("fireworks/{}", row["id"].as_str().expect("ID")),
        about: String::new(),
        wants: Wants {
            max_tokens: Some(128),
            session_id: Some("private-sentinel-session".into()),
            schema: Some(schema()),
            ..Default::default()
        },
        context: Context {
            system: Some("Return exact JSON".into()),
            messages: vec![Message::user("Use echo if needed")],
            tools: vec![Tool {
                name: "echo".into(),
                description: "Echo data".into(),
                strict: None,
                parameters: json!({"type":"object","properties":{"value":{"type":"string"}},"required":["value"],"additionalProperties":false}),
            }],
        },
    }
}

fn event(name: &str, value: Value) -> String {
    format!("event: {name}\ndata: {value}\n\n")
}

fn finished(row: &Value) -> String {
    if !anthropic(row) {
        return concat!("data: {\"choices\":[{\"delta\":{\"content\":\"{\\\"value\\\":7}\"},\"finish_reason\":\"stop\"}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":7,\"prompt_tokens_details\":{\"cached_tokens\":40}}}\n\n",
            "data: [DONE]\n\n").into();
    }
    [event("message_start", json!({"message":{"role":"assistant","content":[],"usage":{
        "input_tokens":0,"output_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}})),
        event("content_block_start", json!({"index":0,"content_block":{"type":"text","text":""}})),
        event("content_block_delta", json!({"index":0,"delta":{"type":"text_delta","text":"{\"value\":7}"}})),
        event("content_block_stop", json!({"index":0})),
        event("message_delta", json!({"delta":{"stop_reason":"end_turn"},"usage":{
            "input_tokens":60,"output_tokens":7,"cache_read_input_tokens":40,"cache_creation_input_tokens":0}})),
        event("message_stop", json!({}))].concat()
}

fn events(output: &std::process::Output) -> Vec<Value> {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).expect("frame"))
        .collect()
}

fn unasked(listener: &TcpListener) {
    assert_eq!(
        listener.accept().expect_err("no request").kind(),
        std::io::ErrorKind::WouldBlock
    );
}

fn affinity(headers: &str, row: &Value) -> String {
    assert!(!headers.contains("private-sentinel-session"));
    let value = headers
        .lines()
        .find_map(|line| line.strip_prefix("x-session-affinity: "))
        .expect("affinity");
    assert_eq!(value.len(), 64);
    assert!(value.bytes().all(|b| b.is_ascii_hexdigit()));
    for name in ["session_id", "x-client-request-id"] {
        let prefix = format!("{name}: ");
        assert_eq!(
            headers.lines().find_map(|line| line.strip_prefix(&prefix)),
            (!anthropic(row)).then_some(value)
        );
    }
    assert!(!headers.contains("x-session-id:"));
    value.into()
}

#[test]
fn all_27_shipped_records_list_offline_with_exact_capabilities_and_key_readiness() {
    let (dir, listener, _) = fixture();
    for vars in [
        vec![],
        vec![(KEY, "private-sentinel")],
        vec![(KEY, " \t\n")],
        vec![("OPENAI_API_KEY", "private-sentinel")],
    ] {
        let output = command(&dir)
            .envs(vars.clone())
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
            .filter(|row| row["provider"] == "fireworks")
            .collect();
        assert_eq!(cards.len(), 27);
        for row in rows() {
            let card = cards
                .iter()
                .find(|card| card["id"] == request(&row).model)
                .expect("card");
            assert_eq!(card["ready"], vars.contains(&(KEY, "private-sentinel")));
            assert_eq!(card["setup"]["variables"], json!([[KEY]]));
            assert_eq!(card["context_window"], row["contextWindow"]);
            assert_eq!(card["max_output"], row["maxTokens"]);
            assert_eq!(card["capabilities"]["input"], row["input"]);
            assert_eq!(card["capabilities"]["strict_tools"], !anthropic(&row));
            assert_eq!(card["capabilities"]["session_identity"], true);
            let levels: Vec<_> = ThinkingLevel::ALL
                .into_iter()
                .filter(|level| row["thinkingLevelMap"][level.as_str()].is_string())
                .map(ThinkingLevel::as_str)
                .collect();
            assert_eq!(card["capabilities"]["thinking_levels"], json!(levels));
        }
        unasked(&listener);
    }
}

#[test]
fn all_27_shipped_models_route_all_efforts_with_native_schema_tools_affinity_and_final_usage() {
    let (dir, listener, _) = fixture();
    let mut model_keys = BTreeSet::new();
    for row in rows() {
        let mut identity = None;
        for level in std::iter::once(None).chain(ThinkingLevel::ALL.into_iter().map(Some)) {
            let mut ask = request(&row);
            ask.wants.thinking = level;
            let before = serde_json::to_value(&ask).expect("snapshot");
            let server =
                response_server(listener.try_clone().expect("listener"), 200, finished(&row));
            let said = events(&ask_request(&dir, &[(KEY, "private-sentinel")], &ask));
            let (headers, body) = server.join().expect("request");
            presets::stopped(&said, "end_turn");
            let key = affinity(&headers, &row);
            assert_eq!(identity.get_or_insert_with(|| key.clone()), &key);
            assert!(headers.starts_with(if anthropic(&row) {
                "post /inference/v1/messages "
            } else {
                "post /inference/v1/chat/completions "
            }));
            assert!(headers.contains(if anthropic(&row) {
                "x-api-key: private-sentinel\r\n"
            } else {
                "authorization: bearer private-sentinel\r\n"
            }));
            assert_eq!(body["model"], row["id"]);
            assert_eq!(body["max_tokens"], 128);
            assert_eq!(body["stream"], true);
            let effort = level.and_then(|level| row["thinkingLevelMap"][level.as_str()].as_str());
            if anthropic(&row) {
                assert_eq!(
                    body["output_config"]["format"],
                    json!({"type":"json_schema","schema":schema().schema})
                );
                assert!(body.get("tool_choice").is_none());
                assert_eq!(body["tools"][0]["name"], "echo");
                for field in ["cache_control", "eager_input_streaming", "strict"] {
                    assert!(body["tools"][0].get(field).is_none());
                }
                assert_eq!(
                    body["system"][0]["cache_control"],
                    json!({"type":"ephemeral"})
                );
                assert_eq!(
                    body["messages"][0]["content"][0]["cache_control"],
                    json!({"type":"ephemeral"})
                );
                let disabled = level == Some(ThinkingLevel::Off) && effort.is_some();
                assert_eq!(
                    body.get("thinking"),
                    effort
                        .map(|_| json!({"type":if disabled { "disabled" } else { "adaptive" }}))
                        .as_ref()
                );
                assert_eq!(
                    body["output_config"].get("effort"),
                    effort.filter(|_| !disabled).map(|e| json!(e)).as_ref()
                );
            } else {
                assert_eq!(
                    body["response_format"]["json_schema"]["schema"],
                    schema().schema
                );
                assert_eq!(body["tools"][0]["function"]["name"], "echo");
                assert_eq!(
                    body.get("reasoning_effort"),
                    effort.map(|e| json!(e)).as_ref()
                );
                assert!(body.get("output_config").is_none());
            }
            assert!(!body.to_string().contains("\"ttl\""));
            assert!(!body.to_string().contains("private-sentinel"));
            assert!(body.get("prompt_cache_key").is_none());
            let text: String = said
                .iter()
                .filter(|e| e["event"] == "text")
                .map(|e| e["text"].as_str().expect("text"))
                .collect();
            assert_eq!(text, "{\"value\":7}");
            assert!(!said.iter().any(|e| e["event"] == "tool_call_start"));
            let usage = &said.iter().rfind(|e| e["event"] == "spent").expect("usage")["usage"];
            assert_eq!(
                (
                    usage["input"].as_u64(),
                    usage["output"].as_u64(),
                    usage["cache_read"].as_u64()
                ),
                (Some(60), Some(7), Some(40))
            );
            let cost = &row["cost"];
            let estimate = cost["input"].as_f64().expect("input") * 60.0
                + cost["output"].as_f64().expect("output") * 7.0
                + cost["cacheRead"].as_f64().expect("cache") * 40.0;
            let expected: Value =
                serde_json::from_str(&format!("{estimate:.0}")).expect("estimate");
            assert_eq!(usage["estimated_micros"], expected);
            assert_eq!(serde_json::to_value(ask).expect("unchanged"), before);
        }
        assert!(model_keys.insert(identity.expect("scope")));
    }
    assert_eq!(model_keys.len(), 27);
    unasked(&listener);
}

#[test]
fn unsupported_strict_tools_invalid_sessions_and_bad_credentials_refuse_before_network() {
    let (dir, listener, _) = fixture();
    for row in rows() {
        let mut ask = request(&row);
        for vars in [
            vec![],
            vec![(KEY, " \t\n")],
            vec![("OPENAI_API_KEY", "private-sentinel")],
        ] {
            let said = events(&ask_request(&dir, &vars, &ask));
            assert_eq!(said.len(), 1);
            assert_eq!(said[0]["why"], "auth");
            unasked(&listener);
        }
        ask.wants.session_id = Some("invalid\r\n".into());
        let said = events(&ask_request(&dir, &[], &ask));
        assert_eq!(said.len(), 1);
        assert_eq!(said[0]["why"], "invalid");
        unasked(&listener);
        if anthropic(&row) {
            ask.wants.session_id = None;
            ask.context.tools[0].strict = Some(ToolStrictness::Require);
            let said = events(&ask_request(&dir, &[], &ask));
            assert_eq!(said.len(), 1);
            assert_eq!(said[0]["why"], "invalid");
            unasked(&listener);
        }
    }
    let mut unknown = request(&rows()[0]);
    unknown.model = "fireworks/not-a-model".into();
    let said = events(&ask_request(&dir, &[(KEY, "private-sentinel")], &unknown));
    assert_eq!(said.len(), 1);
    assert_eq!(said[0]["event"], "failed");
    unasked(&listener);
}
