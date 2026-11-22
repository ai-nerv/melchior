use super::*;
use melchior::mind::model::{Content, Message, ThinkingLevel, ToolStrictness};
use melchior::mind::wire::Schema;

const REGIONS: [(&str, &str, &str); 4] = [
    ("xiaomi", "XIAOMI_API_KEY", "api"),
    (
        "xiaomi-token-plan-cn",
        "XIAOMI_TOKEN_PLAN_CN_API_KEY",
        "token-plan-cn",
    ),
    (
        "xiaomi-token-plan-ams",
        "XIAOMI_TOKEN_PLAN_AMS_API_KEY",
        "token-plan-ams",
    ),
    (
        "xiaomi-token-plan-sgp",
        "XIAOMI_TOKEN_PLAN_SGP_API_KEY",
        "token-plan-sgp",
    ),
];
const MODELS: [&str; 3] = [
    "mimo-v2.6-flash",
    "mimo-v2.6-pro",
    "mimo-v2.6-pro-ultraspeed",
];
const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

fn models(id: &str) -> &[&str] {
    if id == "xiaomi" {
        &MODELS
    } else {
        &MODELS[..2]
    }
}

fn fixture(id: &str, key: &str, host: &str) -> (Scratch, TcpListener) {
    let dir = Scratch::new("melchior-presets", id);
    let (listener, url) = listener(id);
    let mut declaration = shipped(id);
    assert_eq!(
        declaration["base_url"],
        format!("https://{host}.xiaomimimo.com/v1")
    );
    assert_eq!(declaration["api"], "openai-completions");
    assert_eq!(declaration["auth"], json!({"kind":"api-key","vars":[key]}));
    assert_eq!(declaration["discovery"], json!({"kind":"bundled"}));
    assert_eq!(
        declaration["catalog"],
        json!({"source":"pi-1.6.0","provider":id,
        "models":models(id).iter().map(|model| format!("chat:{model}")).collect::<Vec<_>>()})
    );
    assert_eq!(
        declaration["compat"],
        json!({"max_tokens_field":"max_completion_tokens","supports_json_schema":false})
    );
    declaration["base_url"] = json!(url);
    declaration["models"] = json!(
        models(id)
            .iter()
            .map(|model| json!({"id":model,"base_url":url}))
            .collect::<Vec<_>>()
    );
    let encoded = serde_json::to_string(&declaration.to_string()).expect("Lua string");
    std::fs::write(
        dir.join("providers.lua"),
        format!(
            r#"
        for _, id in ipairs({{"openrouter", "ollama"}}) do
            melchior.provider(id, {{name=id,api="openai-completions",auth={{kind="none"}}}})
        end
        melchior.provider({id:?}, melchior.json.decode_value({encoded}))
    "#
        ),
    )
    .expect("private overlay");
    (dir, listener)
}

fn ask(id: &str, model: &str) -> Ask {
    let mut ask = ask_for(id, model);
    ask.context.messages[0].content.push(Content::Image {
        media_type: "image/png".into(),
        data: PNG.into(),
    });
    ask
}

fn request(model: &str, headers: &str, body: &Value) {
    assert!(headers.starts_with("post /v1/chat/completions "));
    assert_eq!(headers.matches("authorization:").count(), 1);
    assert!(headers.contains("authorization: bearer private-sentinel\r\n"));
    assert!(!headers.contains("api-key:"));
    assert_eq!(body["model"], model);
    assert_eq!(body["stream"], true);
    assert_eq!(body["stream_options"], json!({"include_usage":true}));
    assert_eq!(
        body["messages"][0],
        json!({"role":"system","content":"Use the echo tool."})
    );
    assert_eq!(
        body["messages"][1]["content"][1],
        json!({"type":"image_url","image_url":{"url":format!("data:image/png;base64,{PNG}")}})
    );
    assert_eq!(body["tools"][0]["function"]["strict"], true);
    assert_eq!(
        body["tools"][0]["function"]["parameters"],
        ask_for("xiaomi", model).context.tools[0].parameters
    );
    for field in [
        "store",
        "max_tokens",
        "reasoning_effort",
        "response_format",
        "tool_choice",
        "prompt_cache_key",
        "prompt_cache_retention",
    ] {
        assert!(body.get(field).is_none(), "{field}: {body}");
    }
}

fn finished() -> String {
    concat!(
        "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"verified\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"answer\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":20,\"completion_tokens_details\":{\"reasoning_tokens\":12},\"prompt_tokens_details\":{\"cached_tokens\":40}}}\n\n",
        "data: [DONE]\n\n"
    ).into()
}

fn usage(id: &str, model: &str, events: &[Value]) {
    let spent = events
        .iter()
        .find(|event| event["event"] == "spent")
        .expect("usage");
    assert_eq!(spent["usage"]["input"], 60);
    assert_eq!(spent["usage"]["cache_read"], 40);
    assert_eq!(spent["usage"]["output"], 20);
    assert_eq!(spent["usage"]["cost_micros"], 0);
    if id == "xiaomi" {
        assert_eq!(
            spent["usage"]["estimated_micros"],
            match model {
                "mimo-v2.6-flash" => 14,
                "mimo-v2.6-pro" => 44,
                _ => 436,
            }
        );
    } else {
        assert!(
            spent["usage"].get("estimated_micros").is_none(),
            "subscription cost is not free: {spent}"
        );
    }
}

#[test]
fn nine_models_list_offline_with_separate_region_keys_and_honest_plan_pricing() {
    for (id, key, host) in REGIONS {
        let (dir, listener) = fixture(id, key, host);
        for (candidate, value) in REGIONS
            .into_iter()
            .map(|(_, key, _)| (key, "private-sentinel"))
            .chain([(key, " \t\n")])
        {
            let ready = candidate == key && !value.trim().is_empty();
            let output = command(&dir)
                .env(candidate, value)
                .args(["models", "--refresh", "--json"])
                .output()
                .expect("listing");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(!String::from_utf8_lossy(&output.stdout).contains("private-sentinel"));
            assert!(!String::from_utf8_lossy(&output.stderr).contains("private-sentinel"));
            let reply: Value = serde_json::from_slice(&output.stdout).expect("cards");
            assert_eq!(reply["ok"], true);
            let cards: Vec<_> = reply["result"]
                .as_array()
                .expect("models")
                .iter()
                .filter(|card| card["provider"] == id)
                .collect();
            assert_eq!(cards.len(), models(id).len());
            for model in models(id) {
                let card = cards
                    .iter()
                    .find(|card| card["id"] == format!("{id}/{model}"))
                    .expect("model");
                assert_eq!(card["ready"], ready);
                assert_eq!(card["setup"]["variables"], json!([[key]]));
                assert_eq!(
                    card["setup"]["state"],
                    if ready { "ready" } else { "credential_missing" }
                );
                assert_eq!(card["api"], "openai-completions");
                assert_eq!(card["context_window"], 1048576);
                assert_eq!(card["max_output"], 131072);
                assert_eq!(card["reasons"], true);
                assert_eq!(card["capabilities"]["input"], json!(["text", "image"]));
                assert_eq!(
                    card["capabilities"]["thinking_levels"],
                    json!(["off", "high"])
                );
                assert_eq!(card["capabilities"]["strict_tools"], true);
                assert_eq!(
                    card["capabilities"]["input_limits"],
                    json!({"images":{"resize":{
                    "max_width":2000,"max_height":2000,"max_bytes":4718592,"jpeg_quality":80}}})
                );
                if id != "xiaomi" {
                    assert!(card.get("pricing").is_none(), "{card}");
                }
            }
            unasked(&listener);
        }
        for model in models(id) {
            let refused = events(&ask_request(&dir, &[], &ask(id, model)));
            assert_eq!(refused.len(), 1);
            assert_eq!(refused[0]["why"], "auth");
            unasked(&listener);
        }
    }
}

#[test]
fn all_neutral_efforts_use_only_supported_binary_controls_and_preserve_cap_accounting() {
    for (id, key, host) in REGIONS {
        let (dir, listener) = fixture(id, key, host);
        for model in models(id) {
            for level in ThinkingLevel::ALL.into_iter().map(Some).chain([None]) {
                let fixture =
                    response_server(listener.try_clone().expect("listener"), 200, finished());
                let mut ask = ask(id, model);
                ask.wants.thinking = level;
                let said = events(&ask_request(&dir, &[(key, "private-sentinel")], &ask));
                stopped(&said, "end_turn");
                let (headers, body) = fixture.join().expect("request");
                request(model, &headers, &body);
                assert_eq!(body["max_completion_tokens"], 128);
                match level {
                    Some(ThinkingLevel::Off) => {
                        assert_eq!(body["thinking"], json!({"type":"disabled"}))
                    }
                    Some(ThinkingLevel::High) => {
                        assert_eq!(body["thinking"], json!({"type":"enabled"}))
                    }
                    _ => assert!(body.get("thinking").is_none(), "{level:?}: {body}"),
                }
                usage(id, model, &said);
            }
        }
    }
}

#[test]
fn native_tool_and_later_user_turns_preserve_reasoning_and_fill_empty_assistant_fields() {
    for (id, key, host) in REGIONS {
        let (dir, listener) = fixture(id, key, host);
        for model in models(id) {
            let mut ask = ask(id, model);
            ask.context
                .messages
                .push(Message::assistant("old answer without reasoning"));
            ask.context.messages.push(Message::user("use the tool now"));
            let fixture = response_server(listener.try_clone().expect("listener"), 200, concat!(
                "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"inspect \"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"image α\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call-1\",\"function\":{\"name\":\"echo\",\"arguments\":\"{\\\"text\\\":\"}}]}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"hello\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
                "data: [DONE]\n\n"
            ).into());
            let first = events(&ask_request(&dir, &[(key, "private-sentinel")], &ask));
            stopped(&first, "tool_use");
            let (headers, body) = fixture.join().expect("tool call");
            request(model, &headers, &body);
            assert_eq!(body["messages"][2]["reasoning_content"], "");
            let reasoning: String = first
                .iter()
                .filter(|e| e["event"] == "thinking")
                .map(|e| e["text"].as_str().expect("thinking"))
                .collect();
            let signatures: Vec<_> = first.iter().filter(|e| e["event"] == "signature").collect();
            assert_eq!(signatures.len(), 1);
            assert_eq!(signatures[0]["signature"], "reasoning_content");
            assert_eq!(reasoning, "inspect image α");
            let arguments: String = first
                .iter()
                .filter(|e| e["event"] == "tool_call_args")
                .map(|e| e["args"].as_str().expect("arguments"))
                .collect();
            let arguments: Value = serde_json::from_str(&arguments).expect("JSON");
            assert_eq!(arguments, json!({"text":"hello"}));
            ask.context.messages.push(
                serde_json::from_value(json!({"role":"assistant","content":[
                    {"type":"thinking","thinking":reasoning,"signature":signatures[0]["signature"]},
                    {"type":"tool_call","id":"call-1","name":"echo","arguments":arguments}
                ]}))
                .expect("assistant"),
            );
            ask.context.messages.push(serde_json::from_value(json!({"role":"tool","content":[
                {"type":"tool_result","id":"call-1","name":"echo","content":"hello","is_error":false,
                 "images":[{"media_type":"image/png","data":PNG}]}
            ]})).expect("tool result"));
            for later_turn in [false, true] {
                let before = serde_json::to_value(&ask.context).expect("context");
                let fixture =
                    response_server(listener.try_clone().expect("listener"), 200, finished());
                let said = events(&ask_request(&dir, &[(key, "private-sentinel")], &ask));
                stopped(&said, "end_turn");
                usage(id, model, &said);
                let (headers, body) = fixture.join().expect("followup");
                request(model, &headers, &body);
                assert_eq!(body["messages"][4]["reasoning_content"], reasoning);
                assert_eq!(
                    body["messages"][4]["tool_calls"][0]["function"]["arguments"],
                    "{\"text\":\"hello\"}"
                );
                assert_eq!(
                    body["messages"][5],
                    json!({"role":"tool","tool_call_id":"call-1","content":"hello"})
                );
                assert_eq!(
                    body["messages"][6]["content"][1]["image_url"]["url"],
                    format!("data:image/png;base64,{PNG}")
                );
                if later_turn {
                    assert_eq!(body["messages"][7]["reasoning_content"], "verified");
                }
                assert_eq!(
                    serde_json::to_value(&ask.context).expect("unchanged"),
                    before
                );
                ask.context.messages.push(
                    serde_json::from_value(json!({"role":"assistant","content":[
                        {"type":"thinking","thinking":"verified","signature":"reasoning_content"},
                        {"type":"text","text":"answer"}
                    ]}))
                    .expect("answer"),
                );
                ask.context.messages.push(Message::user("new user turn"));
            }
        }
    }
}

#[test]
fn unsupported_answer_schema_and_oversized_images_refuse_before_credentials() {
    for (id, key, host) in REGIONS {
        let (dir, listener) = fixture(id, key, host);
        for model in models(id) {
            for schema in [false, true] {
                let mut ask = ask(id, model);
                if schema {
                    ask.wants.schema = Some(Schema {
                        name: "answer".into(),
                        schema: json!({"type":"object"}),
                    });
                } else {
                    ask.context.messages[0].content.push(Content::Image {
                        media_type: "image/png".into(),
                        data: "A".repeat(4718596),
                    });
                }
                let said = events(&ask_request(&dir, &[], &ask));
                assert_eq!(said.len(), 1);
                assert_eq!(said[0]["why"], "invalid", "{said:?}");
                if schema {
                    assert!(
                        said[0]
                            .to_string()
                            .contains("JSON-schema structured answers")
                    );
                }
                unasked(&listener);
            }
        }
    }
}

#[test]
fn output_caps_and_required_strict_tools_remain_supported_without_structured_answers() {
    for (id, key, host) in REGIONS {
        let (dir, listener) = fixture(id, key, host);
        for model in models(id) {
            for cap in [None, Some(131073), Some(1)] {
                let mut ask = ask(id, model);
                ask.wants.max_tokens = cap;
                ask.context.tools[0].strict = Some(ToolStrictness::Require);
                let fixture =
                    response_server(listener.try_clone().expect("listener"), 200, finished());
                stopped(
                    &events(&ask_request(&dir, &[(key, "private-sentinel")], &ask)),
                    "end_turn",
                );
                let (headers, body) = fixture.join().expect("request");
                request(model, &headers, &body);
                assert_eq!(
                    body["max_completion_tokens"],
                    cap.unwrap_or(32000).min(131072)
                );
            }
        }
    }
}

#[test]
fn invalid_or_unauthorized_requests_do_not_retry_or_expose_credentials() {
    for (id, key, host) in REGIONS {
        let (dir, listener) = fixture(id, key, host);
        for (status, why) in [(401, "auth"), (400, "invalid")] {
            let fixture = response_server(
                listener.try_clone().expect("listener"),
                status,
                json!({"error":{"message":"fixture refusal"}}).to_string(),
            );
            let output = ask_request(&dir, &[(key, "private-sentinel")], &ask(id, MODELS[0]));
            let said = events(&output);
            assert!(!String::from_utf8_lossy(&output.stdout).contains("private-sentinel"));
            assert!(!String::from_utf8_lossy(&output.stderr).contains("private-sentinel"));
            assert_eq!(
                said.iter()
                    .filter(|e| e["event"] == "failed" && e["why"] == why)
                    .count(),
                1,
                "{said:?}"
            );
            assert!(
                !said
                    .iter()
                    .any(|e| matches!(e["event"].as_str(), Some("retrying" | "stop")))
            );
            let (headers, body) = fixture.join().expect("request");
            request(MODELS[0], &headers, &body);
            unasked(&listener);
        }
    }
}

#[test]
fn transient_retry_preserves_region_auth_reasoning_and_image_bytes() {
    for (id, key, host) in REGIONS {
        let (dir, listener) = fixture(id, key, host);
        let fixture = std::thread::spawn(move || {
            let first = response_server(
                listener.try_clone().expect("listener"),
                503,
                "{\"error\":{\"message\":\"retry\"}}".into(),
            )
            .join()
            .expect("first");
            let second = response_server_with_timeout(
                listener,
                200,
                finished(),
                std::time::Duration::from_secs(15),
            )
            .join()
            .expect("retry");
            (first, second)
        });
        let said = events(&ask_request_with_timeout(
            &dir,
            &[(key, "private-sentinel")],
            &ask(id, MODELS[0]),
            std::time::Duration::from_secs(30),
        ));
        assert_eq!(
            said.iter().filter(|e| e["event"] == "retrying").count(),
            1,
            "{said:?}"
        );
        assert_eq!(said.iter().filter(|e| e["event"] == "stop").count(), 1);
        assert!(!said.iter().any(|e| e["event"] == "failed"));
        let (first, second) = fixture.join().expect("fixture");
        request(MODELS[0], &first.0, &first.1);
        request(MODELS[0], &second.0, &second.1);
        assert_eq!(first.1, second.1);
        usage(id, MODELS[0], &said);
    }
}
