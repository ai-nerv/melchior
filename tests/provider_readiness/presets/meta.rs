use super::*;
use melchior::mind::model::{Content, ThinkingLevel, ToolStrictness};

#[path = "meta/replay.rs"]
mod replay;

const KEY: &str = "META_API_KEY";
const MODELS: [(&str, bool, &str, &str, u64); 7] = [
    ("muse-ember-1.0", true, "0.1", "0.5", 12),
    ("muse-spark-1.1", false, "1.25", "4.25", 131),
    ("muse-spark-1.2", false, "1.25", "4.25", 131),
    ("muse-spark-1.2-contributor", false, "0.1", "0.2", 8),
    ("muse-spark-1.3", true, "1.25", "4.25", 131),
    ("muse-spark-1.3-contributor", false, "0.1", "0.2", 8),
    ("muse-wonder-1.0", true, "4", "20", 486),
];
const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

fn fixture() -> (Scratch, TcpListener) {
    let dir = Scratch::new("melchior-presets", "meta");
    let (listener, url) = listener("meta");
    let mut declaration = shipped("meta");
    assert_eq!(declaration["base_url"], "https://api.meta.ai/v1");
    assert_eq!(declaration["api"], "openai-responses");
    assert_eq!(declaration["auth"], json!({"kind":"api-key","vars":[KEY]}));
    assert_eq!(declaration["discovery"], json!({"kind":"bundled"}));
    assert_eq!(
        declaration["catalog"],
        json!({"source":"pi-1.6.0","provider":"meta",
        "models":MODELS.map(|(model,..)| format!("chat:{model}"))})
    );
    declaration["base_url"] = json!(url);
    let overrides = declaration["models"].as_array_mut().expect("overrides");
    for (model, ..) in MODELS {
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
        melchior.provider("meta", melchior.json.decode_value({encoded}))
    "#
        ),
    )
    .expect("private overlay");
    (dir, listener)
}

fn ask(model: &str) -> Ask {
    let mut ask = ask_for("meta", model);
    ask.context.messages[0].content.push(Content::Image {
        media_type: "image/png".into(),
        data: PNG.into(),
    });
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
        "previous_response_id",
        "truncation",
        "__scratch",
        "__budget",
    ] {
        assert!(body.get(field).is_none(), "{field}");
    }
    assert!(!body.to_string().contains("melchior.responses.v1:"));
}

fn completed(output: Value, usage: Value) -> String {
    format!(
        "data: {}\n\n",
        json!({"type":"response.completed","response":{"output":output,"usage":usage}})
    )
}

fn usage() -> Value {
    json!({"input_tokens":90,"output_tokens":12,"input_tokens_details":{"cached_tokens":30}})
}

fn spent(events: &[Value]) -> &Value {
    &events
        .iter()
        .rev()
        .find(|event| event["event"] == "spent")
        .expect("usage")["usage"]
}

#[test]
fn all_seven_models_list_offline_with_explicit_contributor_labels_and_provider_specific_keys() {
    let (dir, listener) = fixture();
    for (key, value, ready) in [
        ("OPENAI_API_KEY", "private-sentinel", false),
        ("MODEL_API_KEY", "private-sentinel", false),
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
        assert!(!String::from_utf8_lossy(&output.stderr).contains("private-sentinel"));
        let reply: Value = serde_json::from_slice(&output.stdout).expect("reply");
        assert_eq!(reply["ok"], true, "{reply}");
        let cards: Vec<_> = reply["result"]
            .as_array()
            .expect("models")
            .iter()
            .filter(|card| card["provider"] == "meta")
            .collect();
        assert_eq!(cards.len(), 7);
        for (model, max, input, output, _) in MODELS {
            let card = cards
                .iter()
                .find(|card| card["id"] == format!("meta/{model}"))
                .expect("card");
            assert_eq!(card["ready"], ready);
            assert_eq!(card["setup"]["variables"], json!([[KEY]]));
            assert_eq!(
                card["setup"]["state"],
                if ready { "ready" } else { "credential_missing" }
            );
            assert_eq!(card["api"], "openai-responses");
            assert_eq!(card["context_window"], 1048576);
            assert_eq!(
                card["max_output"],
                if model == "muse-ember-1.0" {
                    524288
                } else {
                    131072
                }
            );
            assert_eq!(card["pricing"], json!({"input":input,"output":output}));
            assert_eq!(card["capabilities"]["input"], json!(["text", "image"]));
            assert_eq!(card["capabilities"]["strict_tools"], false);
            assert_eq!(
                card["capabilities"]["thinking_levels"],
                if max {
                    json!(["minimal", "low", "medium", "high", "xhigh", "max"])
                } else {
                    json!(["minimal", "low", "medium", "high", "xhigh"])
                }
            );
            assert_eq!(
                card["capabilities"]["input_limits"],
                json!({"images":{"resize":{
                "max_width":2000,"max_height":2000,"max_bytes":4718592,"jpeg_quality":80}}})
            );
            assert_eq!(
                card["display_name"]
                    .as_str()
                    .expect("name")
                    .contains("training-enabled"),
                model.ends_with("-contributor")
            );
            assert_eq!(card["name"], model);
        }
        unasked(&listener);
    }
    for (model, ..) in MODELS {
        let refused = events(&ask_request(&dir, &[], &ask(model)));
        assert_eq!(refused.len(), 1, "{refused:?}");
        assert_eq!(refused[0]["why"], "auth");
        unasked(&listener);
    }
}

#[test]
fn native_effort_selection_retains_unsupported_off_and_model_specific_max() {
    let (dir, listener) = fixture();
    for (model, max, ..) in MODELS {
        for level in [
            None,
            Some(ThinkingLevel::Off),
            Some(ThinkingLevel::Minimal),
            Some(ThinkingLevel::Low),
            Some(ThinkingLevel::Medium),
            Some(ThinkingLevel::High),
            Some(ThinkingLevel::XHigh),
            Some(ThinkingLevel::Max),
        ] {
            let mut ask = ask(model);
            ask.wants.thinking = level;
            let worker = response_server(
                listener.try_clone().expect("clone"),
                200,
                completed(json!([]), json!({})),
            );
            let output = events(&ask_request(&dir, &[(KEY, "private-sentinel")], &ask));
            let (headers, body) = worker.join().expect("request");
            request(model, &headers, &body);
            stopped(&output, "end_turn");
            match level {
                None | Some(ThinkingLevel::Off) => assert!(body.get("reasoning").is_none()),
                Some(ThinkingLevel::Max) if !max => assert!(body.get("reasoning").is_none()),
                Some(level) => assert_eq!(
                    body["reasoning"],
                    json!({"effort":level.as_str(),"summary":"auto"})
                ),
            }
        }
    }
}

#[test]
fn model_prices_cache_rates_and_ember_long_context_tiers_reach_native_accounting() {
    let (dir, listener) = fixture();
    for (model, _, _, _, estimate) in MODELS {
        let worker = response_server(
            listener.try_clone().expect("clone"),
            200,
            completed(json!([]), usage()),
        );
        let output = events(&ask_request(
            &dir,
            &[(KEY, "private-sentinel")],
            &ask(model),
        ));
        let (headers, body) = worker.join().expect("priced request");
        request(model, &headers, &body);
        stopped(&output, "end_turn");
        assert_eq!(spent(&output)["input"], 60);
        assert_eq!(spent(&output)["cache_read"], 30);
        assert_eq!(spent(&output)["output"], 12);
        assert_eq!(spent(&output)["cost_micros"], 0);
        assert_eq!(spent(&output)["estimated_micros"], estimate, "{model}");
    }
    for (total, estimate) in [(200000, 10406), (200001, 31218)] {
        let worker = response_server(
            listener.try_clone().expect("clone"),
            200,
            completed(
                json!([]),
                json!({"input_tokens":total,"output_tokens":12,"input_tokens_details":{"cached_tokens":100000}}),
            ),
        );
        let output = events(&ask_request(
            &dir,
            &[(KEY, "private-sentinel")],
            &ask("muse-ember-1.0"),
        ));
        let (headers, body) = worker.join().expect("tier request");
        request("muse-ember-1.0", &headers, &body);
        stopped(&output, "end_turn");
        assert_eq!(spent(&output)["estimated_micros"], estimate);
    }
}

#[test]
fn required_strict_tools_and_oversized_images_refuse_before_credentials_or_dispatch() {
    let (dir, listener) = fixture();
    for (model, ..) in MODELS {
        let mut strict = ask(model);
        strict.context.tools[0].strict = Some(ToolStrictness::Require);
        let mut large = ask(model);
        large.context.messages[0].content[1] = Content::Image {
            media_type: "image/png".into(),
            data: "A".repeat(6_291_460),
        };
        for ask in [strict, large] {
            let output = events(&ask_request(&dir, &[], &ask));
            assert_eq!(output.len(), 1, "{output:?}");
            assert_eq!(output[0]["why"], "invalid");
            unasked(&listener);
        }
    }
}
