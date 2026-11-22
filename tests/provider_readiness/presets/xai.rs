use super::*;

#[path = "xai/accounting.rs"]
mod accounting;
#[path = "xai/replay.rs"]
mod replay;

const KEY: &str = "XAI_API_KEY";
const MODELS: [&str; 4] = ["grok-4.3", "grok-4.5", "grok-4.6", "grok-4.7"];
const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

fn fixture() -> (Scratch, TcpListener) {
    let dir = Scratch::new("melchior-presets", "xai");
    let (listener, url) = listener("xai");
    let mut declaration = shipped("xai");
    assert_eq!(declaration["base_url"], "https://api.x.ai/v1");
    assert_eq!(declaration["api"], "openai-responses");
    assert_eq!(declaration["auth"], json!({"kind":"api-key","vars":[KEY]}));
    assert_eq!(declaration["discovery"], json!({"kind":"bundled"}));
    assert_eq!(
        declaration["catalog"],
        json!({"source":"pi-1.6.0","provider":"xai",
        "models":MODELS.map(|model| format!("chat:{model}"))})
    );
    declaration["base_url"] = json!(url);
    let overrides = declaration["models"].as_array_mut().expect("overrides");
    for model in MODELS {
        if let Some(found) = overrides.iter_mut().find(|item| item["id"] == model) {
            found["base_url"] = json!(url);
        } else {
            overrides.push(json!({"id":model,"base_url":url}));
        }
    }
    let encoded = serde_json::to_string(&declaration.to_string()).expect("Lua string");
    std::fs::write(
        dir.join("providers.lua"),
        format!(
            r#"
        for _, id in ipairs({{"openrouter", "ollama"}}) do
            melchior.provider(id, {{name=id,api="openai-completions",auth={{kind="none"}}}})
        end
        melchior.provider("xai", melchior.json.decode_value({encoded}))
    "#
        ),
    )
    .expect("private overlay");
    (dir, listener)
}

fn ask(model: &str) -> Ask {
    let mut ask = ask_for("xai", model);
    ask.context.messages[0].content.push(
        serde_json::from_value(json!({
        "type":"image","media_type":"image/png","data":PNG}))
        .expect("image"),
    );
    ask
}

fn request(model: &str, headers: &str, body: &Value) {
    assert!(headers.starts_with("post /v1/responses "));
    assert_eq!(headers.matches("authorization:").count(), 1);
    assert!(headers.contains("authorization: bearer private-sentinel\r\n"));
    assert_eq!(body["model"], model);
    assert_eq!(body["stream"], true);
    assert_eq!(body["store"], false);
    assert_eq!(body["include"], json!(["reasoning.encrypted_content"]));
    assert_eq!(body["max_output_tokens"], 128);
    assert_eq!(body["instructions"], "Use the echo tool.");
    assert_eq!(body["tools"][0]["name"], "echo");
    assert!(body["tools"][0].get("strict").is_none());
    assert_eq!(
        body["input"][0]["content"][1],
        json!({"type":"input_image","detail":"auto",
        "image_url":format!("data:image/png;base64,{PNG}")})
    );
    for field in [
        "stream_options",
        "reasoning_effort",
        "max_tokens",
        "prompt_cache_retention",
        "__scratch",
        "__budget",
    ] {
        assert!(body.get(field).is_none(), "{field}");
    }
}

#[test]
fn all_four_pinned_models_list_offline_with_native_limits_prices_and_only_the_xai_key() {
    let (dir, listener) = fixture();
    for (key, value, ready) in [
        ("OPENAI_API_KEY", "private-sentinel", false),
        (KEY, " \t\n", false),
        (KEY, "private-sentinel", true),
    ] {
        let output = command(&dir)
            .env(key, value)
            .args(["models", "--refresh", "--json"])
            .output()
            .expect("cards");
        assert!(output.status.success());
        assert!(!String::from_utf8_lossy(&output.stdout).contains("private-sentinel"));
        let reply: Value = serde_json::from_slice(&output.stdout).expect("reply");
        assert_eq!(reply["ok"], true, "{reply}");
        let cards: Vec<_> = reply["result"]
            .as_array()
            .expect("models")
            .iter()
            .filter(|card| card["provider"] == "xai")
            .collect();
        assert_eq!(cards.len(), 4);
        for model in MODELS {
            let card = cards
                .iter()
                .find(|card| card["id"] == format!("xai/{model}"))
                .expect("model");
            assert_eq!(card["ready"], ready);
            assert_eq!(card["api"], "openai-responses");
            assert_eq!(card["setup"]["variables"], json!([[KEY]]));
            assert_eq!(
                card["setup"]["state"],
                if ready { "ready" } else { "credential_missing" }
            );
            assert_eq!(
                card["context_window"],
                if model == "grok-4.3" { 1000000 } else { 500000 }
            );
            assert_eq!(
                card["max_output"],
                if model == "grok-4.3" { 30000 } else { 500000 }
            );
            assert_eq!(card["capabilities"]["input"], json!(["text", "image"]));
            assert_eq!(card["capabilities"]["strict_tools"], false);
            assert_eq!(
                card["capabilities"]["thinking_levels"],
                if model == "grok-4.3" {
                    json!(["off", "low", "medium", "high", "xhigh"])
                } else {
                    json!(["low", "medium", "high", "xhigh"])
                }
            );
            assert_eq!(
                card["capabilities"]["input_limits"],
                json!({"images":{"resize":{
                "max_width":2000,"max_height":2000,"max_bytes":4718592,"jpeg_quality":80}}})
            );
            assert_eq!(
                card["pricing"],
                if model == "grok-4.3" {
                    json!({"input":"1.25","output":"2.5"})
                } else {
                    json!({"input":"2","output":"6"})
                }
            );
        }
        unasked(&listener);
    }
    let refused = events(&ask_request(&dir, &[], &ask(MODELS[0])));
    assert_eq!(refused[0]["event"], "failed");
    assert_eq!(refused[0]["why"], "auth");
    unasked(&listener);
}

fn completed(output: Value, usage: Value) -> String {
    format!(
        "data: {}\n\n",
        json!({"type":"response.completed","response":{"output":output,"usage":usage}})
    )
}

#[test]
fn each_pinned_model_dispatches_images_and_reports_native_token_and_tick_usage() {
    for model in MODELS {
        let (dir, listener) = fixture();
        let fixture = response_server(
            listener,
            200,
            completed(
                json!([{"type":"message","id":"msg",
            "role":"assistant","content":[{"type":"output_text","text":"ok","annotations":[]}]}]),
                json!({"input_tokens":90,"output_tokens":12,"input_tokens_details":{"cached_tokens":30},
                "cost_in_usd_ticks":37756000}),
            ),
        );
        let result = events(&ask_request(
            &dir,
            &[(KEY, "private-sentinel")],
            &ask(model),
        ));
        let (headers, body) = fixture.join().expect("request");
        request(model, &headers, &body);
        assert_eq!(body["reasoning"], json!({"effort":"high","summary":"auto"}));
        stopped(&result, "end_turn");
        assert_eq!(
            result
                .iter()
                .filter(|event| event["event"] == "text")
                .map(|event| event["text"].as_str().expect("text"))
                .collect::<String>(),
            "ok"
        );
        let spent: Vec<_> = result
            .iter()
            .filter(|event| event["event"] == "spent")
            .collect();
        assert_eq!(spent.len(), 1);
        let usage = &spent[0]["usage"];
        assert_eq!(usage["input"], 60);
        assert_eq!(usage["cache_read"], 30);
        assert_eq!(usage["output"], 12);
        assert_eq!(usage["cost_micros"], 3776);
        assert!(
            !serde_json::to_string(&result)
                .expect("wire")
                .contains("private-sentinel")
        );
    }
}
