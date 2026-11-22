use super::*;
use melchior::mind::model::{Content, Message, ThinkingLevel, ToolStrictness};
use melchior::mind::wire::Schema;

const REGIONS: [(&str, &str, &str); 2] = [
    ("zai", "ZAI_API_KEY", "api.z.ai"),
    ("zai-coding-cn", "ZAI_CODING_CN_API_KEY", "open.bigmodel.cn"),
];
const GLOBAL: &[&str] = &[
    "glm-4.7",
    "glm-5-turbo",
    "glm-5.2",
    "glm-5.2-highspeed",
    "glm-5.3",
    "glm-5.3-flash",
    "glm-5.3-highspeed",
];
const CHINA: &[&str] = &["glm-4.6v", "glm-5.3", "glm-5.3-flash", "glm-5.3-highspeed"];
const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

fn models(id: &str) -> &'static [&'static str] {
    if id == "zai" { GLOBAL } else { CHINA }
}

fn binary(model: &str) -> bool {
    matches!(model, "glm-4.7" | "glm-5-turbo" | "glm-4.6v")
}

fn vision(model: &str) -> bool {
    matches!(model, "glm-4.6v" | "glm-5.3-flash")
}

fn fixture(id: &str, key: &str, host: &str) -> (Scratch, TcpListener) {
    let dir = Scratch::new("melchior-presets", id);
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let url = format!(
        "http://{}/api/coding/paas/v4",
        listener.local_addr().expect("address")
    );
    let mut declaration = shipped(id);
    assert_eq!(
        declaration["base_url"],
        format!("https://{host}/api/coding/paas/v4")
    );
    assert_eq!(declaration["auth"], json!({"kind":"api-key","vars":[key]}));
    assert_eq!(declaration["api"], "openai-completions");
    assert_eq!(declaration["discovery"], json!({"kind":"bundled"}));
    assert_eq!(
        declaration["compat"],
        json!({"supports_json_schema":false,
        "requires_tool_result_name":id == "zai"})
    );
    assert_eq!(
        declaration["catalog"],
        json!({"source":"pi-1.6.0","provider":id,
        "models":models(id).iter().map(|model| format!("chat:{model}")).collect::<Vec<_>>()})
    );
    declaration["base_url"] = json!(url);
    let overrides = declaration["models"].as_array_mut().expect("overrides");
    assert_eq!(overrides.len(), models(id).len());
    for model in overrides {
        assert_eq!(
            model["compat"],
            json!({"supports_strict_mode":false,
            "supports_store":false,"supports_reasoning_effort":!binary(model["id"].as_str().expect("id")),
            "max_tokens_field":"max_tokens","thinking_format":"zai","zai_tool_stream":true})
        );
        model["base_url"] = json!(url);
    }
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

fn finished() -> String {
    concat!(
        "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"reason α\"},\"finish_reason\":null}],\"usage\":null}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"answer\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":7,\"prompt_tokens_details\":{\"cached_tokens\":40}}}\n\n",
        "data: [DONE]\n\n"
    ).into()
}

fn request(id: &str, model: &str, headers: &str, body: &Value, tools: bool) {
    assert!(headers.starts_with("post /api/coding/paas/v4/chat/completions "));
    assert!(headers.contains("authorization: bearer private-sentinel\r\n"));
    assert_eq!(headers.matches("authorization:").count(), 1);
    assert_eq!(body["model"], model);
    assert_eq!(body["max_tokens"], 128);
    assert_eq!(body["stream"], true);
    assert_eq!(body["stream_options"], json!({"include_usage":true}));
    for field in [
        "max_completion_tokens",
        "store",
        "response_format",
        "tool_choice",
    ] {
        assert!(body.get(field).is_none(), "{field}: {body}");
    }
    assert_eq!(body.get("tool_stream"), tools.then_some(&json!(true)));
    if tools {
        let original = ask_for(id, model);
        assert!(body["tools"][0]["function"].get("strict").is_none());
        assert_eq!(
            body["tools"][0]["function"]["parameters"],
            original.context.tools[0].parameters
        );
    } else {
        assert!(body.get("tools").is_none());
    }
}

fn usage(model: &str, said: &[Value]) {
    let usage = &said.iter().rfind(|e| e["event"] == "spent").expect("usage")["usage"];
    assert_eq!(usage["input"], 60);
    assert_eq!(usage["cache_read"], 40);
    assert_eq!(usage["output"], 7);
    assert_eq!(usage["cost_micros"], 0);
    if model.ends_with("highspeed") {
        assert!(
            usage.get("estimated_micros").is_none(),
            "unknown is not free: {usage}"
        );
    } else {
        let expected = match model {
            "glm-4.6v" => 26,
            "glm-4.7" => 56,
            "glm-5-turbo" => 110,
            "glm-5.3-flash" => 14,
            _ => 125,
        };
        assert_eq!(usage["estimated_micros"], expected, "{model}");
    }
}

#[test]
fn eleven_models_list_offline_with_region_isolation_and_honest_capabilities() {
    let mut total = 0;
    for (id, key, host) in REGIONS {
        let (dir, listener) = fixture(id, key, host);
        for vars in [
            vec![],
            vec![(REGIONS[0].1, "private-sentinel")],
            vec![(REGIONS[1].1, "private-sentinel")],
            vec![(key, " \t\n")],
            vec![
                ("OPENAI_API_KEY", "private-sentinel"),
                ("MODEL_API_KEY", "private-sentinel"),
            ],
        ] {
            let output = command(&dir)
                .envs(vars.clone())
                .args(["models", "--refresh", "--json"])
                .output()
                .expect("models");
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
                assert_eq!(card["ready"], vars.contains(&(key, "private-sentinel")));
                assert_eq!(card["setup"]["variables"], json!([[key]]));
                assert_eq!(card["capabilities"]["strict_tools"], false);
                assert_eq!(
                    card["capabilities"]["thinking_levels"],
                    if binary(model) {
                        json!(["off", "high"])
                    } else if model.starts_with("glm-5.2") {
                        json!(["off", "high", "max"])
                    } else {
                        json!(["low", "high", "max"])
                    }
                );
                assert_eq!(
                    card["capabilities"]["input"],
                    if vision(model) {
                        json!(["text", "image"])
                    } else {
                        json!(["text"])
                    }
                );
                assert_eq!(
                    card["context_window"],
                    match *model {
                        "glm-4.6v" => 128_000,
                        "glm-4.7" => 204_800,
                        "glm-5-turbo" => 200_000,
                        _ => 1_000_000,
                    }
                );
                assert_eq!(
                    card["max_output"],
                    if *model == "glm-4.6v" {
                        32_768
                    } else {
                        131_072
                    }
                );
                assert_eq!(card["reasons"], true);
                if vision(model) {
                    assert_eq!(
                        card["capabilities"]["input_limits"],
                        json!({"images":{"resize":{
                        "max_width":2000,"max_height":2000,"max_bytes":4718592,"jpeg_quality":80}}})
                    );
                }
                if model.ends_with("highspeed") {
                    assert!(card.get("pricing").is_none());
                }
            }
            unasked(&listener);
        }
        for model in models(id) {
            let said = events(&ask_request(&dir, &[], &ask_for(id, model)));
            assert_eq!(said.len(), 1);
            assert_eq!(said[0]["why"], "auth");
            unasked(&listener);
            total += 1;
        }
    }
    assert_eq!(total, 11);
}

#[test]
fn all_efforts_keep_native_aliases_nulls_binary_switches_images_and_final_usage() {
    for (id, key, host) in REGIONS {
        let (dir, listener) = fixture(id, key, host);
        for model in models(id) {
            for level in ThinkingLevel::ALL.into_iter().map(Some).chain([None]) {
                let mut ask = ask_for(id, model);
                ask.wants.thinking = level;
                if level.is_none() {
                    ask.context.tools.clear();
                }
                ask.context.messages[0].content.push(Content::Image {
                    media_type: "image/png".into(),
                    data: PNG.into(),
                });
                let saved = ask.context.clone();
                let server =
                    response_server(listener.try_clone().expect("listener"), 200, finished());
                let said = events(&ask_request(&dir, &[(key, "private-sentinel")], &ask));
                stopped(&said, "end_turn");
                let (headers, body) = server.join().expect("request");
                request(id, model, &headers, &body, level.is_some());
                if binary(model) && level == Some(ThinkingLevel::Off) {
                    assert_eq!(body["thinking"], json!({"type":"disabled"}));
                } else {
                    assert_eq!(
                        body["thinking"],
                        json!({"type":"enabled","clear_thinking":false})
                    );
                }
                let effort = match level {
                    Some(ThinkingLevel::Off) if model.starts_with("glm-5.2") => Some("none"),
                    Some(ThinkingLevel::Low) if model.starts_with("glm-5.3") => Some("low"),
                    Some(ThinkingLevel::High) if !binary(model) => Some("high"),
                    Some(ThinkingLevel::Max) if !binary(model) => Some("max"),
                    _ => None,
                };
                assert_eq!(
                    body.get("reasoning_effort"),
                    effort.map(|s| json!(s)).as_ref()
                );
                if vision(model) {
                    assert_eq!(
                        body["messages"][1]["content"][1]["image_url"]["url"],
                        format!("data:image/png;base64,{PNG}")
                    );
                } else {
                    assert!(!body.to_string().contains(PNG));
                    assert!(
                        body["messages"][1]["content"]
                            .as_str()
                            .expect("text")
                            .contains("Image omitted")
                    );
                }
                usage(model, &said);
                assert_eq!(ask.context, saved);
            }
        }
    }
}

fn collected(said: &[Value]) -> Message {
    let text = |kind: &str| {
        said.iter()
            .filter(|e| e["event"] == kind)
            .filter_map(|e| e["text"].as_str())
            .collect::<String>()
    };
    assert!(
        said.iter()
            .any(|e| e["event"] == "signature" && e["signature"] == "reasoning_content")
    );
    let mut message = Message::assistant(text("text"));
    message.content.insert(
        0,
        Content::Thinking {
            thinking: text("thinking"),
            signature: Some("reasoning_content".into()),
        },
    );
    let mut calls: Vec<(String, String)> = Vec::new();
    for event in said {
        match event["event"].as_str() {
            Some("tool_call_start") => {
                assert_eq!(event["name"], "echo");
                calls.push((event["id"].as_str().expect("id").into(), String::new()));
            }
            Some("tool_call_args") => calls
                .last_mut()
                .expect("call")
                .1
                .push_str(event["args"].as_str().expect("args")),
            _ => {}
        }
    }
    for (id, args) in calls {
        message.content.push(Content::ToolCall {
            id,
            name: "echo".into(),
            arguments: serde_json::from_str(&args).expect("JSON"),
            thought_signature: None,
        });
    }
    message
}

fn tool_stream() -> String {
    let rows = [
        json!({"choices":[{"delta":{"reasoning_content":"first α\n"}}],"usage":null}),
        json!({"choices":[{"delta":{"reasoning_content":"second 🦀","tool_calls":[
            {"index":0,"id":"a","function":{"name":"echo","arguments":"{\"text\":"}},
            {"index":1,"id":"b","function":{"name":"echo","arguments":"{\"text\":"}}]}}]}),
        json!({"choices":[{"delta":{"tool_calls":[
            {"index":1,"function":{"arguments":"\"β\"}"}},
            {"index":0,"function":{"arguments":"\"α\"}"}}]},"finish_reason":"tool_calls"}]}),
    ];
    rows.into_iter()
        .map(|row| format!("data: {row}\n\n"))
        .collect::<String>()
        + "data: [DONE]\n\n"
}

#[test]
fn parallel_tools_replay_exact_reasoning_results_and_images_across_later_turns() {
    for (id, key, host) in REGIONS {
        let (dir, listener) = fixture(id, key, host);
        for model in models(id) {
            let mut ask = ask_for(id, model);
            let server =
                response_server(listener.try_clone().expect("listener"), 200, tool_stream());
            let said = events(&ask_request(&dir, &[(key, "private-sentinel")], &ask));
            stopped(&said, "tool_use");
            let (headers, body) = server.join().expect("tools request");
            request(id, model, &headers, &body, true);
            let message = collected(&said);
            let calls: Vec<_> = message
                .content
                .iter()
                .filter_map(|content| match content {
                    Content::ToolCall { id, arguments, .. } => {
                        Some((id.clone(), arguments.clone()))
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(
                calls,
                vec![
                    ("a".into(), json!({"text":"α"})),
                    ("b".into(), json!({"text":"β"}))
                ]
            );
            ask.context.messages.push(message);
            for (call, _) in &calls {
                ask.context.messages.push(serde_json::from_value(json!({"role":"tool","content":[{
                    "type":"tool_result","id":call,"name":"echo","content":format!("result-{call}"),"is_error":false,
                    "images":[{"media_type":"image/png","data":PNG}]}]})).expect("tool result"));
            }
            for later in [false, true] {
                let before = ask.context.clone();
                let server =
                    response_server(listener.try_clone().expect("listener"), 200, finished());
                let said = events(&ask_request(&dir, &[(key, "private-sentinel")], &ask));
                stopped(&said, "end_turn");
                let (headers, body) = server.join().expect("followup");
                request(id, model, &headers, &body, true);
                assert_eq!(
                    body["thinking"],
                    json!({"type":"enabled","clear_thinking":false})
                );
                let messages = body["messages"].as_array().expect("messages");
                assert_eq!(messages[2]["reasoning_content"], "first α\nsecond 🦀");
                for (index, (call, args)) in calls.iter().enumerate() {
                    let tool = &messages[2]["tool_calls"][index];
                    assert_eq!(tool["id"], *call);
                    assert_eq!(
                        serde_json::from_str::<Value>(
                            tool["function"]["arguments"].as_str().expect("args")
                        )
                        .expect("JSON"),
                        *args
                    );
                    assert_eq!(messages[index + 3]["tool_call_id"], *call);
                    let result = if vision(model) {
                        format!("result-{call}")
                    } else {
                        format!(
                            "result-{call}\n[Image omitted: the current model does not support images. Switch to a vision-capable model to view it.]"
                        )
                    };
                    assert_eq!(messages[index + 3]["content"], result);
                    assert_eq!(
                        messages[index + 3].get("name"),
                        (id == "zai").then_some(&json!("echo"))
                    );
                }
                assert_eq!(body.to_string().contains(PNG), vision(model));
                if later {
                    assert_eq!(
                        messages[messages.len() - 2]["reasoning_content"],
                        "reason α"
                    );
                    assert_eq!(messages.last().expect("last")["content"], "later");
                }
                assert_eq!(ask.context, before);
                ask.context.messages.push(collected(&said));
                ask.context.messages.push(Message::user("later"));
            }
        }
    }
}

#[test]
fn unverified_strict_guarantees_and_json_schema_refuse_before_credentials() {
    for (id, key, host) in REGIONS {
        let (dir, listener) = fixture(id, key, host);
        for model in models(id) {
            for strict in [false, true] {
                let mut ask = ask_for(id, model);
                if strict {
                    ask.context.tools[0].strict = Some(ToolStrictness::Require);
                } else {
                    ask.wants.schema = Some(Schema {
                        name: "answer".into(),
                        schema: json!({"type":"object"}),
                    });
                }
                let said = events(&ask_request(&dir, &[], &ask));
                assert_eq!(said.len(), 1);
                assert_eq!(said[0]["event"], "failed");
                assert_eq!(said[0]["why"], "invalid");
                unasked(&listener);
            }
        }
    }
}

#[test]
fn auth_errors_and_truncated_tool_arguments_never_become_successful_turns() {
    for (id, key, host) in REGIONS {
        let (dir, listener) = fixture(id, key, host);
        for (status, response, expected) in [
            (401, r#"{"error":{"code":"1000","message":"Authentication failed"}}"#.into(), "auth"),
            (200, "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"a\",\"function\":{\"name\":\"echo\",\"arguments\":\"{\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\ndata: [DONE]\n\n".into(), "protocol"),
        ] {
            let ask = ask_for(id, "glm-5.3");
            let server = response_server(listener.try_clone().expect("listener"), status, response);
            let said = events(&ask_request(&dir, &[(key,"private-sentinel")], &ask));
            let (headers, body) = server.join().expect("request");
            request(id, "glm-5.3", &headers, &body, true);
            if status == 200 {
                stopped(&said, "error");
            } else {
                assert!(!said.iter().any(|e| e["event"] == "stop" || e["event"] == "retrying"), "{said:?}");
                let failed: Vec<_> = said.iter().filter(|e| e["event"] == "failed").collect();
                assert_eq!(failed.len(), 1);
                assert_eq!(failed[0]["why"], expected);
            }
            unasked(&listener);
        }
    }
}

#[test]
fn regional_retry_preserves_controls_headers_and_the_callers_request() {
    for (id, key, host) in REGIONS {
        let (dir, listener) = fixture(id, key, host);
        let ask = ask_for(id, "glm-5.3");
        let before = serde_json::to_value(&ask).expect("ask");
        let server = std::thread::spawn(move || {
            let first = response_server(
                listener.try_clone().expect("listener"),
                503,
                r#"{"error":{"code":"1303","message":"Service overloaded"}}"#.into(),
            )
            .join()
            .expect("first request");
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
            &ask,
            std::time::Duration::from_secs(30),
        ));
        assert_eq!(said.iter().filter(|e| e["event"] == "retrying").count(), 1);
        assert!(!said.iter().any(|e| e["event"] == "failed"), "{said:?}");
        let stops: Vec<_> = said.iter().filter(|e| e["event"] == "stop").collect();
        assert_eq!(stops.len(), 1);
        assert_eq!(stops[0]["reason"], "end_turn");
        let (first, second) = server.join().expect("fixture");
        request(id, "glm-5.3", &first.0, &first.1, true);
        request(id, "glm-5.3", &second.0, &second.1, true);
        assert_eq!(first.1, second.1);
        assert_eq!(
            second.1["thinking"],
            json!({"type":"enabled","clear_thinking":false})
        );
        assert_eq!(second.1["reasoning_effort"], "high");
        assert_eq!(serde_json::to_value(&ask).expect("unchanged"), before);
        usage("glm-5.3", &said);
    }
}
