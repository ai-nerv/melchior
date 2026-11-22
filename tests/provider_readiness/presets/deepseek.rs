use super::*;
use melchior::mind::model::{Content, Message, ThinkingLevel};
use melchior::mind::wire::Schema;

const MODELS: [&str; 2] = ["deepseek-flash", "deepseek-v4-pro"];
const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

fn fixture() -> (Scratch, TcpListener) {
    let dir = Scratch::new("melchior-presets", "deepseek");
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let url = format!("http://{}", listener.local_addr().expect("address"));
    let mut declaration = shipped("deepseek");
    assert_eq!(declaration["base_url"], "https://api.deepseek.com");
    assert_eq!(declaration["api"], "openai-completions");
    assert_eq!(
        declaration["auth"],
        json!({"kind":"api-key","vars":["DEEPSEEK_API_KEY"]})
    );
    assert_eq!(declaration["discovery"], json!({"kind":"bundled"}));
    assert_eq!(
        declaration["catalog"],
        json!({"source":"pi-1.6.0","provider":"deepseek",
        "models":["chat:deepseek-flash","chat:deepseek-v4-pro"]})
    );
    assert_eq!(
        declaration["compat"],
        json!({"supports_reasoning_effort":true,
        "supports_json_schema":false,"chat_strict_dialect":"deepseek"})
    );
    assert!(declaration.get("models").is_none());
    declaration["base_url"] = json!(url);
    declaration["models"] = json!(
        MODELS
            .iter()
            .map(|id| json!({"id":id,"base_url":url}))
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
        melchior.provider("deepseek", melchior.json.decode_value({encoded}))
    "#
        ),
    )
    .expect("private overlay");
    (dir, listener)
}

fn ask(model: &str) -> Ask {
    ask_for("deepseek", model)
}

fn finish() -> String {
    concat!(
        "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"reason α\"},\"finish_reason\":null}],\"usage\":null}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"answer\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":7,\"prompt_cache_hit_tokens\":40}}\n\n",
        "data: [DONE]\n\n"
    ).into()
}

fn collected(said: &[Value]) -> Message {
    let text = |kind: &str| {
        said.iter()
            .filter(|e| e["event"] == kind)
            .filter_map(|e| e["text"].as_str())
            .collect::<String>()
    };
    let mut message = Message::assistant(text("text"));
    let signature = said
        .iter()
        .find(|e| e["event"] == "signature")
        .expect("reasoning signature");
    assert_eq!(signature["signature"], "reasoning_content");
    message.content.insert(
        0,
        Content::Thinking {
            thinking: text("thinking"),
            signature: Some("reasoning_content".into()),
        },
    );
    let calls: Vec<_> = said
        .iter()
        .filter(|e| e["event"] == "tool_call_start")
        .collect();
    if !calls.is_empty() {
        assert_eq!(calls.len(), 1);
        let arguments = said
            .iter()
            .filter(|e| e["event"] == "tool_call_args")
            .map(|e| e["args"].as_str().expect("argument fragment"))
            .collect::<String>();
        message.content.push(Content::ToolCall {
            id: calls[0]["id"].as_str().expect("id").into(),
            name: calls[0]["name"].as_str().expect("name").into(),
            arguments: serde_json::from_str(&arguments).expect("arguments"),
            thought_signature: None,
        });
    }
    message
}

fn request(model: &str, headers: &str, body: &Value) {
    assert!(headers.starts_with("post /beta/chat/completions "));
    assert!(headers.contains("authorization: bearer private-sentinel\r\n"));
    assert_eq!(headers.matches("authorization:").count(), 1);
    assert_eq!(body["model"], model);
    assert_eq!(body["max_tokens"], 128);
    assert!(body.get("max_completion_tokens").is_none());
    assert!(body.get("store").is_none());
    assert!(body.get("response_format").is_none());
    assert_eq!(body["tools"][0]["function"]["strict"], true);
    assert_eq!(
        body["tools"][0]["function"]["parameters"]["additionalProperties"],
        false
    );
}

#[test]
fn pinned_models_list_offline_with_independent_key_and_lossless_limits_and_controls() {
    let (dir, listener) = fixture();
    for vars in [
        vec![],
        vec![
            ("OPENAI_API_KEY", "private-sentinel"),
            ("MODEL_API_KEY", "private-sentinel"),
        ],
        vec![("DEEPSEEK_API_KEY", "private-sentinel")],
    ] {
        let output = command(&dir)
            .envs(vars.clone())
            .args(["models", "--all"])
            .output()
            .expect("models");
        assert!(output.status.success());
        assert!(!String::from_utf8_lossy(&output.stdout).contains("private-sentinel"));
        let reply: Value = serde_json::from_slice(&output.stdout).expect("cards");
        let cards: Vec<_> = reply["result"]
            .as_array()
            .expect("models")
            .iter()
            .filter(|m| m["provider"] == "deepseek")
            .collect();
        assert_eq!(cards.len(), 2);
        for id in MODELS {
            let card = cards
                .iter()
                .find(|m| m["id"] == format!("deepseek/{id}"))
                .expect("card");
            assert_eq!(
                card["ready"],
                vars.iter().any(|(key, _)| *key == "DEEPSEEK_API_KEY")
            );
            assert_eq!(card["setup"]["variables"], json!([["DEEPSEEK_API_KEY"]]));
            assert_eq!(card["context_window"], 1_000_000);
            assert_eq!(card["max_output"], 384_000);
            assert_eq!(card["capabilities"]["strict_tools"], true);
            assert_eq!(
                card["capabilities"]["thinking_levels"],
                if id == MODELS[0] {
                    json!(["off", "low", "high", "max"])
                } else {
                    json!(["off", "high", "max"])
                }
            );
            assert_eq!(
                card["capabilities"]["input"],
                if id == MODELS[0] {
                    json!(["text", "image"])
                } else {
                    json!(["text"])
                }
            );
            if id == MODELS[0] {
                assert_eq!(
                    card["capabilities"]["input_limits"],
                    json!({"images":{"resize":{"max_width":2000,"max_height":2000,"max_bytes":4718592,"jpeg_quality":80}}})
                );
            }
        }
        unasked(&listener);
    }
}

#[test]
fn all_efforts_keep_pinned_nulls_native_switches_image_input_and_terminal_accounting() {
    let (dir, listener) = fixture();
    for model in MODELS {
        for level in ThinkingLevel::ALL.into_iter().map(Some).chain([None]) {
            let mut ask = ask(model);
            ask.wants.thinking = level;
            if model == MODELS[0] {
                ask.context.messages[0].content.push(Content::Image {
                    media_type: "image/png".into(),
                    data: PNG.into(),
                });
            }
            let fixture = response_server(listener.try_clone().expect("listener"), 200, finish());
            let said = events(&ask_request(
                &dir,
                &[("DEEPSEEK_API_KEY", "private-sentinel")],
                &ask,
            ));
            stopped(&said, "end_turn");
            let (headers, body) = fixture.join().expect("fixture");
            request(model, &headers, &body);
            let mapped = match level {
                Some(ThinkingLevel::Off) => Some("off"),
                Some(ThinkingLevel::Low) if model == MODELS[0] => Some("low"),
                Some(ThinkingLevel::High) => Some("high"),
                Some(ThinkingLevel::Max) => Some("max"),
                _ => None,
            };
            if let Some(mapped) = mapped {
                assert_eq!(
                    body["thinking"]["type"],
                    if mapped == "off" {
                        "disabled"
                    } else {
                        "enabled"
                    }
                );
                if mapped != "off" {
                    assert_eq!(body["reasoning_effort"], mapped);
                } else {
                    assert!(body.get("reasoning_effort").is_none());
                }
            } else {
                assert!(body.get("thinking").is_none());
                assert!(body.get("reasoning_effort").is_none());
            }
            if model == MODELS[0] {
                assert_eq!(
                    body["messages"][1]["content"][1]["image_url"]["url"],
                    format!("data:image/png;base64,{PNG}")
                );
            }
            let spent = said.iter().rfind(|e| e["event"] == "spent").expect("usage");
            assert_eq!(spent["usage"]["input"], 60);
            assert_eq!(spent["usage"]["output"], 7);
            assert_eq!(spent["usage"]["cache_read"], 40);
            assert_eq!(
                spent["usage"]["estimated_micros"],
                if model == MODELS[0] { 27 } else { 109 }
            );
        }
    }
}

#[test]
fn tool_and_later_user_history_replay_all_reasoning_and_preserve_pro_instruction_positions() {
    let (dir, listener) = fixture();
    for model in MODELS {
        let mut ask = ask(model);
        let fixture = response_server(listener.try_clone().expect("listener"), 200, concat!(
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"tool reason\"},\"finish_reason\":null}],\"usage\":null}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call\",\"function\":{\"name\":\"echo\",\"arguments\":\"{\\\"text\\\":\"}}]},\"finish_reason\":null}],\"usage\":null}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"α\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":7}}\n\n",
            "data: [DONE]\n\n"
        ).into());
        let said = events(&ask_request(
            &dir,
            &[("DEEPSEEK_API_KEY", "private-sentinel")],
            &ask,
        ));
        stopped(&said, "tool_use");
        let (headers, body) = fixture.join().expect("first request");
        request(model, &headers, &body);
        ask.context.messages.push(collected(&said));
        ask.context.messages.push(
            serde_json::from_value(json!({"role":"tool","content":[
                {"type":"tool_result","id":"call","name":"echo","content":"result","is_error":false,
                    "images":[{"media_type":"image/png","data":PNG}]}
            ]}))
            .expect("tool result"),
        );
        let mut previous = None;
        for later in [false, true] {
            if later {
                ask.context.messages.extend([
                    previous.take().expect("previous assistant"),
                    Message::system("updated instruction"),
                    Message::user("later"),
                ]);
            }
            let saved = ask.context.clone();
            let fixture = response_server(listener.try_clone().expect("listener"), 200, finish());
            let said = events(&ask_request(
                &dir,
                &[("DEEPSEEK_API_KEY", "private-sentinel")],
                &ask,
            ));
            stopped(&said, "end_turn");
            previous = Some(collected(&said));
            let (headers, body) = fixture.join().expect("fixture");
            request(model, &headers, &body);
            let messages = body["messages"].as_array().expect("messages");
            let assistants: Vec<_> = messages
                .iter()
                .filter(|m| m["role"] == "assistant")
                .collect();
            assert_eq!(assistants[0]["reasoning_content"], "tool reason");
            assert_eq!(
                assistants[0]["tool_calls"][0]["function"]["arguments"],
                r#"{"text":"α"}"#
            );
            if later {
                assert_eq!(assistants[1]["reasoning_content"], "reason α");
                assert_eq!(messages.last().expect("last")["content"], "later");
                if model == MODELS[1] {
                    assert_eq!(
                        messages[messages.len() - 2],
                        json!({"role":"system","content":"updated instruction"})
                    );
                } else {
                    assert_eq!(
                        messages[0]["content"],
                        "Use the echo tool.\n\nupdated instruction"
                    );
                }
            }
            if model == MODELS[0] {
                assert!(
                    body.to_string()
                        .contains(&format!("data:image/png;base64,{PNG}"))
                );
            } else {
                assert!(!body.to_string().contains(PNG));
                assert!(body.to_string().contains("Image omitted"));
            }
            assert_eq!(ask.context, saved);
        }
    }
}

#[test]
fn unsupported_answer_schema_refuses_before_missing_credentials_or_inference() {
    let (dir, listener) = fixture();
    for model in MODELS {
        let mut ask = ask(model);
        ask.wants.schema = Some(Schema {
            name: "answer".into(),
            schema: json!({"type":"object"}),
        });
        let said = events(&ask_request(&dir, &[], &ask));
        assert_eq!(said.len(), 1);
        assert_eq!(said[0]["event"], "failed");
        assert_eq!(said[0]["why"], "invalid");
        unasked(&listener);
    }
}
