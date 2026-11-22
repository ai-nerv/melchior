use super::*;

const MODEL: &str = "stepfun-ai/step-5-preview";
const KEY: &str = "NVIDIA_API_KEY";
const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

fn fixture() -> (Scratch, TcpListener) {
    let dir = Scratch::new("melchior-presets", "nvidia");
    let (listener, url) = listener("nvidia");
    let mut declaration = shipped("nvidia");
    assert_eq!(
        declaration["base_url"],
        "https://integrate.api.nvidia.com/v1"
    );
    assert_eq!(declaration["api"], "openai-completions");
    assert_eq!(declaration["auth"], json!({"kind":"api-key","vars":[KEY]}));
    assert_eq!(declaration["discovery"], json!({"kind":"bundled"}));
    assert_eq!(
        declaration["catalog"],
        json!({"source":"pi-1.6.0","provider":"nvidia",
        "models":[format!("chat:{MODEL}")]})
    );
    assert_eq!(
        declaration["models"].as_array().expect("overrides").len(),
        1
    );
    assert_eq!(
        declaration["models"][0]["capabilities"]["thinking_levels"],
        json!([]),
        "{declaration}"
    );
    declaration["base_url"] = json!(url);
    declaration["models"][0]["base_url"] = json!(url);
    let encoded = serde_json::to_string(&declaration.to_string()).expect("Lua string");
    std::fs::write(dir.join("providers.lua"), format!(r#"
        for _, id in ipairs({{"openrouter", "ollama"}}) do
            melchior.provider(id, {{name=id,api="openai-completions",auth={{kind="none"}}}})
        end
        local nvidia = melchior.json.decode({encoded})
        nvidia.models[1].capabilities.thinking_levels = melchior.json.array(nvidia.models[1].capabilities.thinking_levels)
        melchior.provider("nvidia", nvidia)
    "#)).expect("private configuration");
    (dir, listener)
}

fn image() -> Value {
    json!({"type":"image","media_type":"image/png","data":PNG})
}

fn ask(images: usize) -> Ask {
    let mut ask = ask_for("nvidia", MODEL);
    ask.context.messages[0]
        .content
        .extend((0..images).map(|_| serde_json::from_value(image()).expect("image")));
    ask
}

fn request(headers: &str, body: &Value) {
    assert!(headers.starts_with("post /v1/chat/completions "));
    assert_eq!(headers.matches("authorization:").count(), 1);
    assert!(headers.contains("authorization: bearer private-sentinel\r\n"));
    assert_eq!(headers.matches("nvcf-poll-seconds:").count(), 1);
    assert!(headers.contains("nvcf-poll-seconds: 3600\r\n"));
    assert_eq!(body["model"], MODEL);
    assert_eq!(body["stream"], true);
    assert_eq!(body["stream_options"], json!({"include_usage":true}));
    assert_eq!(body["max_tokens"], 128);
    assert_eq!(
        body["messages"][0],
        json!({"role":"system","content":"Use the echo tool."})
    );
    for field in [
        "store",
        "max_completion_tokens",
        "reasoning_effort",
        "thinking",
        "enable_thinking",
        "prompt_cache_retention",
        "prompt_cache_key",
        "cache_control",
    ] {
        assert!(body.get(field).is_none(), "unsupported {field}: {body}");
    }
    assert!(body["tools"][0]["function"].get("strict").is_none());
    assert_eq!(
        body["tools"][0]["function"]["parameters"],
        ask(0).context.tools[0].parameters
    );
}

#[test]
fn listing_is_offline_and_preserves_reviewed_limits_without_claiming_free_service() {
    let (dir, listener) = fixture();
    for (key, value, ready) in [
        ("GROQ_API_KEY", "private-sentinel", false),
        (KEY, " \t\n", false),
        (KEY, "private-sentinel", true),
    ] {
        let output = command(&dir)
            .env(key, value)
            .args(["models", "--refresh", "--json"])
            .output()
            .expect("listing");
        assert!(output.status.success());
        assert!(!String::from_utf8_lossy(&output.stdout).contains("private-sentinel"));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("private-sentinel"));
        let reply: Value = serde_json::from_slice(&output.stdout).expect("cards");
        assert_eq!(reply["ok"], true, "{reply}");
        let cards: Vec<_> = reply["result"]
            .as_array()
            .expect("models")
            .iter()
            .filter(|card| card["provider"] == "nvidia")
            .collect();
        assert_eq!(
            cards.len(),
            1,
            "{reply}; stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        let card = cards[0];
        assert_eq!(card["id"], format!("nvidia/{MODEL}"));
        assert_eq!(card["ready"], ready);
        assert_eq!(card["setup"]["variables"], json!([[KEY]]));
        assert_eq!(
            card["setup"]["state"],
            if ready { "ready" } else { "credential_missing" }
        );
        assert_eq!(card["api"], "openai-completions");
        assert_eq!(card["context_window"], 1048576);
        assert_eq!(card["max_output"], 1048576);
        assert_eq!(card["reasons"], true);
        assert_eq!(card["capabilities"]["thinking_levels"], json!([]));
        assert_eq!(card["capabilities"]["input"], json!(["text", "image"]));
        assert_eq!(card["capabilities"]["strict_tools"], false);
        assert_eq!(
            card["capabilities"]["input_limits"],
            json!({"images":{
            "max_per_request":8,"resize":{"max_width":2000,"max_height":2000,
                "max_bytes":4718592,"jpeg_quality":80}}})
        );
        assert!(card.get("pricing").is_none(), "{card}");
        unasked(&listener);
    }
    let refused = events(&ask_request(&dir, &[], &ask(0)));
    assert_eq!(refused.len(), 1);
    assert_eq!(refused[0]["event"], "failed");
    assert_eq!(refused[0]["why"], "auth");
    unasked(&listener);
}

#[test]
fn reasoning_tool_arguments_images_and_unknown_cost_survive_the_native_tool_loop() {
    let (dir, listener) = fixture();
    let following = listener.try_clone().expect("follow-up");
    let fixture = response_server(listener, 200, concat!(
        "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"inspect image\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call-1\",\"function\":{\"name\":\"echo\",\"arguments\":\"{\\\"text\\\":\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"hello\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":20,\"prompt_tokens_details\":{\"cached_tokens\":40}}}\n\n",
        "data: [DONE]\n\n"
    ).into());
    let mut ask = ask(7);
    let first = events(&ask_request(&dir, &[(KEY, "private-sentinel")], &ask));
    assert!(!first.iter().any(|e| e["event"] == "failed"), "{first:?}");
    let (headers, body) = fixture.join().expect("tool request");
    request(&headers, &body);
    stopped(&first, "tool_use");
    assert!(
        first
            .iter()
            .any(|e| e["event"] == "thinking" && e["text"] == "inspect image")
    );
    let signatures: Vec<_> = first.iter().filter(|e| e["event"] == "signature").collect();
    assert_eq!(signatures.len(), 1);
    assert_eq!(signatures[0]["signature"], "reasoning_content");
    let calls: Vec<_> = first
        .iter()
        .filter(|e| e["event"] == "tool_call_start")
        .collect();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["id"], "call-1");
    assert_eq!(calls[0]["name"], "echo");
    let args: String = first
        .iter()
        .filter(|e| e["event"] == "tool_call_args")
        .map(|e| e["args"].as_str().expect("arguments"))
        .collect();
    let args: Value = serde_json::from_str(&args).expect("assembled arguments");
    assert_eq!(args, json!({"text":"hello"}));
    let usage = &first.iter().find(|e| e["event"] == "spent").expect("usage")["usage"];
    assert_eq!(usage["input"], 60);
    assert_eq!(usage["cache_read"], 40);
    assert_eq!(usage["output"], 20);
    assert_eq!(usage["cost_micros"], 0);
    assert!(usage.get("estimated_micros").is_none(), "{usage}");
    ask.context.messages.push(
        serde_json::from_value(json!({"role":"assistant","content":[
        {"type":"thinking","thinking":"inspect image","signature":"reasoning_content"},
        {"type":"tool_call","id":"call-1","name":"echo","arguments":args}]}))
        .expect("call"),
    );
    ask.context.messages.push(
        serde_json::from_value(json!({"role":"tool","content":[
        {"type":"tool_result","id":"call-1","name":"echo","content":"hello","is_error":false,
            "images":[{"media_type":"image/png","data":PNG}]}]}))
        .expect("result"),
    );
    let saved = ask.context.clone();
    let fixture = server(following);
    let second = events(&ask_request(&dir, &[(KEY, "private-sentinel")], &ask));
    let (headers, next) = fixture.join().expect("tool result request");
    request(&headers, &next);
    stopped(&second, "end_turn");
    assert!(
        second
            .iter()
            .any(|e| e["event"] == "text" && e["text"] == "ok")
    );
    assert_eq!(ask.context, saved);
    assert_eq!(next["messages"][1], body["messages"][1]);
    assert_eq!(
        next["messages"][1]["content"]
            .as_array()
            .expect("parts")
            .len(),
        8
    );
    let data_url = format!("data:image/png;base64,{PNG}");
    for part in next["messages"][1]["content"]
        .as_array()
        .expect("parts")
        .iter()
        .skip(1)
    {
        assert_eq!(
            part,
            &json!({"type":"image_url","image_url":{"url":data_url}})
        );
    }
    assert_eq!(next["messages"][2]["reasoning_content"], "inspect image");
    assert_eq!(next["messages"][2]["tool_calls"][0]["id"], "call-1");
    assert_eq!(
        next["messages"][3],
        json!({"role":"tool","tool_call_id":"call-1","content":"hello"})
    );
    assert_eq!(next["messages"][4]["role"], "user");
    assert_eq!(
        next["messages"][4]["content"][1]["image_url"]["url"],
        data_url
    );
}

#[test]
fn excess_images_and_required_strict_tools_refuse_before_authentication_or_dispatch() {
    let (dir, listener) = fixture();
    let mut strict = ask(0);
    strict.context.tools[0].strict = Some(melchior::mind::model::ToolStrictness::Require);
    let mut mixed = ask(8);
    mixed.context.messages.push(
        serde_json::from_value(json!({"role":"assistant","content":[
        {"type":"tool_call","id":"call-1","name":"echo","arguments":{"text":"hi"}}]}))
        .expect("call"),
    );
    mixed.context.messages.push(
        serde_json::from_value(json!({"role":"tool","content":[
        {"type":"tool_result","id":"call-1","name":"echo","content":"hi","is_error":false,
            "images":[{"media_type":"image/png","data":PNG}]}]}))
        .expect("image result"),
    );
    for (ask, message) in [(ask(9), "image"), (mixed, "image"), (strict, "strict")] {
        let output = events(&ask_request(&dir, &[], &ask));
        let failures: Vec<_> = output.iter().filter(|e| e["event"] == "failed").collect();
        assert_eq!(failures.len(), 1, "{output:?}");
        assert_eq!(failures[0]["why"], "invalid", "{output:?}");
        assert!(
            failures[0]["message"]
                .as_str()
                .expect("refusal")
                .contains(message),
            "{output:?}"
        );
        assert!(
            !output
                .iter()
                .any(|e| matches!(e["event"].as_str(), Some("stop" | "retrying")))
        );
        unasked(&listener);
    }
}

#[test]
fn provider_refusals_do_not_retry_or_publish_credentials() {
    for (status, why) in [(401, "auth"), (400, "invalid"), (422, "invalid")] {
        let (dir, listener) = fixture();
        let remaining = listener.try_clone().expect("retry observer");
        let fixture = response_server(
            listener,
            status,
            json!({"error":{"message":"fixture refusal"}}).to_string(),
        );
        let output = events(&ask_request(&dir, &[(KEY, "private-sentinel")], &ask(0)));
        let (headers, body) = fixture.join().expect("refused request");
        request(&headers, &body);
        let failures: Vec<_> = output.iter().filter(|e| e["event"] == "failed").collect();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0]["why"], why);
        assert!(
            !output
                .iter()
                .any(|e| matches!(e["event"].as_str(), Some("stop" | "retrying")))
        );
        unasked(&remaining);
    }
}

#[test]
fn retry_reuses_the_same_images_headers_and_limits_without_enabling_unsupported_options() {
    let (dir, listener) = fixture();
    let following = listener.try_clone().expect("retry listener");
    let remaining = listener.try_clone().expect("extra request observer");
    let fixture = std::thread::spawn(move || {
        let failed = response_server(
            listener,
            503,
            json!({"error":{"message":"temporarily unavailable"}}).to_string(),
        )
        .join()
        .expect("first attempt");
        let succeeded = response_server_with_timeout(following, 200,
            "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n".into(),
            std::time::Duration::from_secs(15)).join().expect("second attempt");
        [failed, succeeded]
    });
    let output = events(&ask_request_with_timeout(
        &dir,
        &[(KEY, "private-sentinel")],
        &ask(8),
        std::time::Duration::from_secs(25),
    ));
    let captured = fixture.join().expect("retry fixture");
    for (headers, body) in &captured {
        request(headers, body);
    }
    assert_eq!(captured[0].1, captured[1].1);
    assert_eq!(
        output.iter().filter(|e| e["event"] == "retrying").count(),
        1
    );
    assert!(!output.iter().any(|e| e["event"] == "failed"), "{output:?}");
    let stops: Vec<_> = output.iter().filter(|e| e["event"] == "stop").collect();
    assert_eq!(stops.len(), 1);
    assert_eq!(stops[0]["reason"], "end_turn");
    unasked(&remaining);
}
