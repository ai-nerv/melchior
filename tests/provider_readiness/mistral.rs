use super::*;
use melchior::mind::{
    model::{Message, ThinkingLevel},
    wire::Ask,
};
use std::collections::BTreeSet;

const KEY: &str = "MISTRAL_API_KEY";
const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
const DONE: &str = "data: {\"choices\":[{\"delta\":{\"content\":[{\"type\":\"text\",\"text\":\"ok\"}]},\"finish_reason\":\"stop\"}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":7,\"num_cached_tokens\":40}}\n\ndata: [DONE]\n\n";

fn rows() -> Vec<Value> {
    serde_json::from_str::<Value>(include_str!(
        "../../catalog/reference/pi-1.6.0/data/mistral.json"
    ))
    .expect("source")["mistral-conversations"]
        .as_object()
        .expect("models")
        .values()
        .cloned()
        .collect()
}

fn fixture() -> (Scratch, TcpListener) {
    let dir = Scratch::new("melchior-mistral", "native");
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let url = format!("http://{}", listener.local_addr().expect("address"));
    let mut provider = presets::shipped("mistral");
    assert_eq!(provider["api"], "mistral-conversations");
    assert_eq!(provider["base_url"], "https://api.mistral.ai");
    assert_eq!(provider["auth"], json!({"kind":"api-key","vars":[KEY]}));
    assert_eq!(provider["discovery"], json!({"kind":"bundled"}));
    let rows = rows();
    assert_eq!(rows.len(), 34);
    assert_eq!(
        provider["catalog"]["models"]
            .as_array()
            .expect("selected")
            .iter()
            .map(|value| value.as_str().expect("key").to_owned())
            .collect::<BTreeSet<_>>(),
        rows.iter()
            .map(|row| format!("chat:{}", row["id"].as_str().expect("id")))
            .collect()
    );
    provider["base_url"] = json!(url);
    provider["models"] = json!(
        rows.iter()
            .map(|row| json!({"id":row["id"],"base_url":url}))
            .collect::<Vec<_>>()
    );
    let encoded = serde_json::to_string(&provider.to_string()).expect("Lua string");
    std::fs::write(
        dir.join("providers.lua"),
        format!(
            r#"
        for _, id in ipairs({{"openrouter","ollama"}}) do
            melchior.provider(id,{{name=id,api="openai-completions",auth={{kind="none"}}}})
        end
        melchior.provider("mistral",melchior.json.decode_value({encoded}))
    "#
        ),
    )
    .expect("fixture");
    (dir, listener)
}

fn ask(row: &Value) -> Ask {
    serde_json::from_value(json!({"model":format!("mistral/{}",row["id"].as_str().expect("id")),
        "about":"mistral-fixture","wants":{"max_tokens":128,"session_id":"private-session"},
        "context":{"system":"Use the existing echo tool.","messages":[{"role":"user","content":[{"type":"text","text":"hello"},{"type":"image","media_type":"image/png","data":PNG}]}],
            "tools":[{"name":"echo","description":"Echo","strict":"prefer","parameters":{"type":"object","properties":{"value":{"type":"integer"}},"required":["value"],"additionalProperties":false}}]}})).expect("ask")
}

fn round(dir: &Scratch, listener: &TcpListener, ask: &Ask, response: &str) -> (Vec<Value>, Value) {
    let server = response_server(
        listener.try_clone().expect("listener"),
        200,
        response.into(),
    );
    let events = presets::events(&ask_request(dir, &[(KEY, "private-sentinel")], ask));
    let (headers, body) = server.join().expect("server");
    assert!(
        headers.starts_with("post /v1/chat/completions "),
        "{headers}"
    );
    assert!(headers.contains("authorization: bearer private-sentinel\r\n"));
    let session = body["prompt_cache_key"].as_str().expect("opaque session");
    assert_eq!(session.len(), 64);
    assert!(headers.contains(&format!("x-affinity: {session}\r\n")));
    assert!(!body.to_string().contains("private-session"));
    assert_eq!(body["max_tokens"], 128);
    for field in ["stream_options", "store", "__budget", "__session_id"] {
        assert!(body.get(field).is_none());
    }
    presets::unasked(listener);
    (events, body)
}

#[test]
fn all_34_models_list_offline_and_only_mistral_credentials_enable_them() {
    let (dir, listener) = fixture();
    for (vars, ready) in [
        (vec![], false),
        (vec![(KEY, "private-sentinel")], true),
        (vec![("OPENAI_API_KEY", "private-sentinel")], false),
    ] {
        let output = command(&dir)
            .envs(vars)
            .args(["models", "--refresh", "--json"])
            .output()
            .expect("models");
        assert!(output.status.success());
        assert!(!String::from_utf8_lossy(&output.stdout).contains("private-sentinel"));
        let reply: Value = serde_json::from_slice(&output.stdout).expect("JSON");
        let cards: Vec<_> = reply["result"]
            .as_array()
            .expect("cards")
            .iter()
            .filter(|card| card["provider"] == "mistral")
            .collect();
        assert_eq!(cards.len(), 34);
        for row in rows() {
            let card = cards
                .iter()
                .find(|card| card["id"] == ask(&row).model)
                .expect("model");
            assert_eq!(card["ready"], ready);
            assert_eq!(card["api"], "mistral-conversations");
            assert_eq!(card["capabilities"]["strict_tools"], false);
            assert_eq!(card["context_window"], row["contextWindow"]);
            assert_eq!(card["max_output"], row["maxTokens"]);
            assert_eq!(card["capabilities"]["input"], row["input"]);
            assert_eq!(card["setup"]["variables"], json!([[KEY]]));
        }
        presets::unasked(&listener);
    }
}

#[test]
fn all_models_and_efforts_use_native_controls_scoped_caching_and_paid_usage() {
    let (dir, listener) = fixture();
    let mut requests = 0;
    for row in rows() {
        for level in std::iter::once(None).chain(ThinkingLevel::ALL.map(Some)) {
            let mut ask = ask(&row);
            ask.wants.thinking = level;
            let before = serde_json::to_value(&ask).expect("before");
            let (events, body) = round(&dir, &listener, &ask, DONE);
            presets::stopped(&events, "end_turn");
            let map = row["thinkingLevelMap"].as_object();
            let expected = map.and_then(|map| match level {
                None => map.get("off").and_then(Value::as_str),
                Some(level) => {
                    let at = ThinkingLevel::ALL
                        .iter()
                        .position(|candidate| *candidate == level)
                        .expect("level");
                    ThinkingLevel::ALL
                        .iter()
                        .skip(at)
                        .chain(ThinkingLevel::ALL.iter().take(at).rev())
                        .find_map(|level| map.get(level.as_str()).and_then(Value::as_str))
                }
            });
            assert_eq!(
                body.get("reasoning_effort").and_then(Value::as_str),
                expected,
                "{} {level:?}",
                row["id"]
            );
            assert!(body["tools"][0]["function"].get("strict").is_none());
            assert_eq!(
                body.to_string().contains(PNG),
                row["input"]
                    .as_array()
                    .expect("input")
                    .contains(&json!("image"))
            );
            let usage = &events
                .iter()
                .rfind(|event| event["event"] == "spent")
                .expect("spent")["usage"];
            assert_eq!(
                (
                    usage["input"].as_u64(),
                    usage["output"].as_u64(),
                    usage["cache_read"].as_u64()
                ),
                (Some(60), Some(7), Some(40))
            );
            assert_eq!(serde_json::to_value(&ask).expect("after"), before);
            requests += 1;
        }
    }
    assert_eq!(requests, 272);
}

#[test]
fn native_call_and_result_history_replays_without_executing_tools_in_melchior() {
    let (dir, listener) = fixture();
    let mut requests = 0;
    for row in rows() {
        let mut ask = ask(&row);
        let script = "data: {\"choices\":[{\"delta\":{\"content\":[{\"type\":\"thinking\",\"thinking\":[{\"type\":\"text\",\"text\":\"check\"}]}],\"tool_calls\":[{\"index\":0,\"id\":\"abcdefghi\",\"function\":{\"name\":\"echo\",\"arguments\":\"{\\\"value\\\":7}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n";
        let (events, _) = round(&dir, &listener, &ask, script);
        presets::stopped(&events, "tool_use");
        assert!(
            events
                .iter()
                .any(|event| event["event"] == "tool_call_start"
                    && event["id"] == "abcdefghi"
                    && event["name"] == "echo")
        );
        assert!(events.iter().any(|event| event["event"] == "tool_call_args" && event["args"] == "{\"value\":7}"));
        for message in [
            json!({"role":"assistant","content":[{"type":"thinking","thinking":"check"},
            {"type":"tool_call","id":"abcdefghi","name":"echo","arguments":{"value":7}}]}),
            json!({"role":"tool","content":[{"type":"tool_result","id":"abcdefghi","name":"echo","content":"7","is_error":false}]}),
        ] {
            ask.context
                .messages
                .push(serde_json::from_value(message).expect("history"));
        }
        ask.context.messages.push(Message::user("continue"));
        let (events, body) = round(&dir, &listener, &ask, DONE);
        presets::stopped(&events, "end_turn");
        assert_eq!(body["messages"][2]["content"][0]["type"], "thinking");
        assert_eq!(body["messages"][2]["tool_calls"][0]["id"], "abcdefghi");
        assert_eq!(body["messages"][3]["tool_call_id"], "abcdefghi");
        assert_eq!(body["messages"][3]["content"][0]["text"], "7");
        requests += 2;
    }
    assert_eq!(requests, 68);
}

#[test]
fn missing_auth_strict_refusal_malformed_stream_and_cancellation_stay_in_existing_transport() {
    let (dir, listener) = fixture();
    let mut ask = ask(&rows()[0]);
    let events = presets::events(&ask_request(&dir, &[], &ask));
    assert_eq!(events.last().expect("failed")["why"], "auth");
    presets::unasked(&listener);
    ask.context.tools[0].strict = Some(melchior::mind::model::ToolStrictness::Require);
    let events = presets::events(&ask_request(&dir, &[(KEY, "private-sentinel")], &ask));
    assert_eq!(events.last().expect("failed")["why"], "invalid");
    presets::unasked(&listener);
    ask.context.tools[0].strict = None;
    let (events, _) = round(&dir, &listener, &ask, "data: not json\n\ndata: [DONE]\n\n");
    presets::stopped(&events, "error");
    let (headers, body) = cancellation::after_usage(&dir, &listener, KEY, &ask);
    assert!(headers.starts_with("post /v1/chat/completions "));
    assert_eq!(body["model"], rows()[0]["id"]);
}
