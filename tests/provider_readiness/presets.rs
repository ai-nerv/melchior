use super::*;
use melchior::mind::lua::engine::Engine;
use melchior::mind::wire::Ask;

#[path = "presets/nvidia.rs"]
mod nvidia;

#[path = "presets/xai.rs"]
mod xai;

#[path = "presets/meta.rs"]
mod meta;

#[path = "presets/minimax.rs"]
mod minimax;

#[path = "presets/xiaomi.rs"]
mod xiaomi;

#[path = "presets/deepseek.rs"]
mod deepseek;

#[path = "presets/zai.rs"]
mod zai;

#[path = "presets/moonshot.rs"]
mod moonshot;

#[path = "presets/baseten.rs"]
mod baseten;

const MODELS: &[(&str, &str, &str)] = &[
    ("groq", "openai/gpt-oss-120b", "GROQ_API_KEY"),
    ("groq", "openai/gpt-oss-20b", "GROQ_API_KEY"),
    ("cerebras", "gpt-oss-120b", "CEREBRAS_API_KEY"),
];

pub(super) fn shipped(id: &str) -> Value {
    let mut engine = Engine::new();
    engine
        .run(include_str!("../../config/providers.lua"), "providers.lua")
        .expect("shipped providers");
    engine.harvest();
    engine
        .config()
        .all("provider")
        .into_iter()
        .find(|(name, _)| *name == id)
        .expect("shipped provider")
        .1
        .clone()
}

fn local(dir: &Scratch, id: &str, url: &str) {
    let mut declaration = shipped(id);
    let expected = if id == "groq" {
        "https://api.groq.com/openai/v1"
    } else {
        "https://api.cerebras.ai/v1"
    };
    assert_eq!(declaration["base_url"], expected);
    assert_eq!(declaration["discovery"], json!({"kind":"bundled"}));
    let keys: Vec<_> = MODELS
        .iter()
        .filter(|(provider, _, _)| *provider == id)
        .map(|(_, model, _)| format!("chat:{model}"))
        .collect();
    assert_eq!(declaration["catalog"]["models"], json!(keys));
    declaration["base_url"] = json!(url);
    declaration["models"] = Value::Array(
        keys.iter()
            .map(|key| {
                json!({
                    "id":key.strip_prefix("chat:").expect("chat identity"),"base_url":url
                })
            })
            .collect(),
    );
    let encoded = serde_json::to_string(&declaration.to_string()).expect("Lua string");
    std::fs::write(
        dir.join("providers.lua"),
        format!(
            r#"
        for _, id in ipairs({{"openrouter", "ollama"}}) do
            melchior.provider(id, {{name=id,api="openai-completions",auth={{kind="none"}}}})
        end
        melchior.provider({id:?}, melchior.json.decode({encoded}))
    "#
        ),
    )
    .expect("private provider overlay");
}

fn listener(id: &str) -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let prefix = if id == "groq" { "/openai/v1" } else { "/v1" };
    let url = format!("http://{}{prefix}", listener.local_addr().expect("address"));
    (listener, url)
}

pub(super) fn unasked(listener: &TcpListener) {
    assert_eq!(
        listener.accept().expect_err("no network request").kind(),
        std::io::ErrorKind::WouldBlock
    );
}

fn ask_for(id: &str, model: &str) -> Ask {
    serde_json::from_value(json!({"model":format!("{id}/{model}"),"about":"preset-fixture",
        "context":{"system":"Use the echo tool.","messages":[{"role":"user","content":[{"type":"text","text":"hello"}]}],
            "tools":[{"name":"echo","description":"Echo text","strict":"prefer",
                "parameters":{"type":"object","properties":{"text":{"type":"string"}},"required":["text"],"additionalProperties":false}}]},
        "wants":{"thinking":"high","max_tokens":128}})).expect("ask")
}

pub(super) fn events(output: &std::process::Output) -> Vec<Value> {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).expect("event"))
        .collect()
}

pub(super) fn stopped(events: &[Value], reason: &str) {
    assert!(
        !events
            .iter()
            .any(|event| matches!(event["event"].as_str(), Some("failed" | "retrying"))),
        "{events:?}"
    );
    let stops: Vec<_> = events
        .iter()
        .filter(|event| event["event"] == "stop")
        .collect();
    assert_eq!(stops.len(), 1);
    assert_eq!(stops[0]["reason"], reason);
}

fn request(id: &str, model: &str, headers: &str, body: &Value) {
    let path = if id == "groq" {
        "/openai/v1/chat/completions"
    } else {
        "/v1/chat/completions"
    };
    assert!(headers.starts_with(&format!("post {path} ")));
    assert_eq!(headers.matches("authorization:").count(), 1);
    assert!(headers.contains("authorization: bearer private-sentinel\r\n"));
    assert_eq!(body["model"], model);
    assert_eq!(body["stream"], true);
    assert_eq!(body["stream_options"]["include_usage"], true);
    assert_eq!(body["reasoning_effort"], "high");
    assert_eq!(body["max_completion_tokens"], 128);
    assert!(body.get("max_tokens").is_none());
    assert_eq!(body["messages"][0]["role"], "system");
    assert!(
        body["messages"]
            .as_array()
            .expect("messages")
            .iter()
            .all(|message| message.get("name").is_none())
    );
    assert_eq!(
        body.get("store"),
        if id == "groq" {
            Some(&Value::Bool(false))
        } else {
            None
        }
    );
    assert_eq!(
        body["tools"][0]["function"].get("strict"),
        if id == "groq" {
            Some(&Value::Bool(true))
        } else {
            None
        }
    );
}

#[test]
fn bundled_presets_expose_selected_metadata_and_only_their_own_api_key_enables_them() {
    for &(id, model, variable) in MODELS {
        let dir = Scratch::new("melchior-presets", "listing");
        let (listener, url) = listener(id);
        local(&dir, id, &url);
        for (key, value, ready) in [
            ("UNRELATED_KEY", "private-sentinel", false),
            (variable, " \t\n", false),
            (variable, "private-sentinel", true),
        ] {
            let output = command(&dir)
                .env(key, value)
                .args(["models", "--refresh", "--json"])
                .output()
                .expect("listing");
            assert!(output.status.success());
            assert!(!String::from_utf8_lossy(&output.stdout).contains("private-sentinel"));
            assert!(!String::from_utf8_lossy(&output.stderr).contains("private-sentinel"));
            let reply: Value = serde_json::from_slice(&output.stdout).expect("model cards");
            assert_eq!(reply["ok"], true);
            let card = reply["result"]
                .as_array()
                .expect("cards")
                .iter()
                .find(|card| card["id"] == format!("{id}/{model}"))
                .expect("selected card");
            assert_eq!(card["ready"], ready);
            assert_eq!(card["api"], "openai-completions");
            assert_eq!(card["context_window"], 131072);
            assert_eq!(card["max_output"], if id == "groq" { 65536 } else { 40960 });
            assert_eq!(card["reasons"], true);
            assert_eq!(
                card["capabilities"]["thinking_levels"],
                json!(["low", "medium", "high"])
            );
            assert_eq!(card["capabilities"]["input"], json!(["text"]));
            assert_eq!(
                card["setup"]["state"],
                if ready { "ready" } else { "credential_missing" }
            );
            assert_eq!(card["setup"]["variables"], json!([[variable]]));
            unasked(&listener);
        }
        let refused = events(&ask_request(&dir, &[], &ask_for(id, model)));
        assert_eq!(refused.len(), 1);
        assert_eq!(refused[0]["event"], "failed");
        assert_eq!(refused[0]["why"], "auth");
        unasked(&listener);
    }
}

#[test]
fn bundled_presets_stream_reasoning_tools_usage_and_replay_the_tool_result() {
    for &(id, model, variable) in MODELS {
        let dir = Scratch::new("melchior-presets", "tool-round");
        let (listener, url) = listener(id);
        local(&dir, id, &url);
        let following = listener.try_clone().expect("second request listener");
        let server = response_server(listener, 200, concat!(
            "data: {\"choices\":[{\"delta\":{\"reasoning\":\"check tool\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call-1\",\"function\":{\"name\":\"echo\",\"arguments\":\"{\\\"text\\\":\"}}]},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"hello\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":20,\"prompt_tokens_details\":{\"cached_tokens\":40}}}\n\n",
            "data: [DONE]\n\n"
        ).into());
        let mut ask = ask_for(id, model);
        let first = events(&ask_request(&dir, &[(variable, "private-sentinel")], &ask));
        let (headers, body) = server.join().expect("HTTP fixture");
        request(id, model, &headers, &body);
        stopped(&first, "tool_use");
        assert!(
            first
                .iter()
                .any(|event| event["event"] == "thinking" && event["text"] == "check tool")
        );
        let signatures: Vec<_> = first
            .iter()
            .filter(|event| event["event"] == "signature")
            .collect();
        assert_eq!(signatures.len(), 1);
        assert_eq!(signatures[0]["signature"], "reasoning");
        let starts: Vec<_> = first
            .iter()
            .filter(|event| event["event"] == "tool_call_start")
            .collect();
        assert_eq!(starts.len(), 1);
        assert_eq!(starts[0]["id"], "call-1");
        assert_eq!(starts[0]["name"], "echo");
        let arguments: String = first
            .iter()
            .filter(|event| event["event"] == "tool_call_args")
            .map(|event| event["args"].as_str().expect("arguments"))
            .collect();
        let arguments: Value = serde_json::from_str(&arguments).expect("assembled arguments");
        assert_eq!(arguments, json!({"text":"hello"}));
        let usage = &first
            .iter()
            .rev()
            .find(|event| event["event"] == "spent")
            .expect("usage")["usage"];
        assert_eq!(usage["input"], 60);
        assert_eq!(usage["cache_read"], 40);
        assert_eq!(usage["output"], 20);
        assert_eq!(usage["cost_micros"], 0);
        let estimated = if id == "cerebras" {
            50
        } else if model.ends_with("/gpt-oss-20b") {
            12
        } else {
            24
        };
        assert_eq!(usage["estimated_micros"], estimated);
        ask.context.messages.push(
            serde_json::from_value(json!({"role":"assistant","content":[
            {"type":"thinking","thinking":"check tool","signature":signatures[0]["signature"]},
            {"type":"tool_call","id":"call-1","name":"echo","arguments":arguments}]}))
            .expect("assistant history"),
        );
        ask.context.messages.push(serde_json::from_value(json!({"role":"tool","content":[
            {"type":"tool_result","id":"call-1","name":"echo","content":"hello","is_error":false}]})).expect("tool history"));
        let server = super::server(following);
        let second = events(&ask_request(&dir, &[(variable, "private-sentinel")], &ask));
        let (headers, body) = server.join().expect("second HTTP fixture");
        request(id, model, &headers, &body);
        stopped(&second, "end_turn");
        assert!(
            second
                .iter()
                .any(|event| event["event"] == "text" && event["text"] == "ok")
        );
        assert_eq!(body["messages"][2]["tool_calls"][0]["id"], "call-1");
        assert_eq!(body["messages"][2]["reasoning"], "check tool");
        assert!(body["messages"][2].get("reasoning_content").is_none());
        assert_eq!(body["messages"][3]["role"], "tool");
        assert_eq!(body["messages"][3]["tool_call_id"], "call-1");
        assert_eq!(body["messages"][3]["content"], "hello");
    }
}

#[test]
fn bundled_presets_classify_auth_and_invalid_errors_without_retrying_or_leaking_keys() {
    for &(id, model, variable) in MODELS {
        for (status, why) in [(401, "auth"), (400, "invalid")] {
            let dir = Scratch::new("melchior-presets", "errors");
            let (listener, url) = listener(id);
            local(&dir, id, &url);
            let remaining = listener.try_clone().expect("extra request observer");
            let server = response_server(
                listener,
                status,
                json!({"error":{"message":"fixture refusal"}}).to_string(),
            );
            let events = events(&ask_request(
                &dir,
                &[(variable, "private-sentinel")],
                &ask_for(id, model),
            ));
            let (headers, body) = server.join().expect("HTTP error fixture");
            request(id, model, &headers, &body);
            let failures: Vec<_> = events
                .iter()
                .filter(|event| event["event"] == "failed")
                .collect();
            assert_eq!(failures.len(), 1);
            assert_eq!(failures[0]["why"], why);
            assert!(
                !events
                    .iter()
                    .any(|event| matches!(event["event"].as_str(), Some("stop" | "retrying")))
            );
            unasked(&remaining);
        }
    }
}
