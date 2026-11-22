use super::*;
use melchior::mind::model::{Content, Context, Message, Role, Tool};
use melchior::mind::wire::{Ask, Wants};

fn values() -> [Value; 2] {
    [json!({"value":[null,[],{}]}), json!({"value":u64::MAX})]
}

fn tools(provided: bool) -> String {
    let parts: Vec<_> = values()
        .into_iter()
        .enumerate()
        .map(|(index, arguments)| {
            let mut part = json!({"functionCall":{"name":"echo","args":arguments}});
            if provided {
                part["functionCall"]["id"] = json!(format!("provider-{index}"));
            }
            part
        })
        .collect();
    format!(
        "data: {}\n\n",
        json!({"candidates":[{"content":{"parts":parts},"finishReason":"STOP"}]})
    )
}

fn collect(events: &[Value], ask: &mut Ask) -> Vec<String> {
    presets::stopped(events, "tool_use");
    let mut calls: Vec<(String, String)> = Vec::new();
    for event in events {
        match event["event"].as_str() {
            Some("tool_call_start") => {
                assert_eq!(event["name"], "echo");
                calls.push((event["id"].as_str().expect("id").into(), String::new()));
            }
            Some("tool_call_args") => calls
                .last_mut()
                .expect("call")
                .1
                .push_str(event["args"].as_str().expect("arguments")),
            _ => {}
        }
    }
    assert_eq!(calls.len(), 2);
    assert_ne!(calls[0].0, calls[1].0);
    let mut assistant = Message::assistant("");
    for ((id, arguments), expected) in calls.iter().zip(values()) {
        let arguments: Value = serde_json::from_str(arguments).expect("JSON");
        assert_eq!(arguments, expected);
        assistant.content.push(Content::ToolCall {
            id: id.clone(),
            name: "echo".into(),
            arguments,
            thought_signature: None,
        });
    }
    ask.context.messages.push(assistant);
    for (id, _) in &calls {
        ask.context.messages.push(Message {
            role: Role::Tool,
            content: vec![Content::ToolResult {
                id: id.clone(),
                name: "echo".into(),
                content: format!("result-{id}"),
                is_error: false,
                images: Vec::new(),
            }],
            ..Message::user("")
        });
    }
    calls.into_iter().map(|(id, _)| id).collect()
}

fn paired(body: &Value, ids: &[String], modern: bool) {
    assert!(body.get("__scratch").is_none());
    assert!(body.get("__budget").is_none());
    let parts: Vec<_> = body["contents"]
        .as_array()
        .expect("contents")
        .iter()
        .flat_map(|message| message["parts"].as_array().expect("parts"))
        .collect();
    let calls: Vec<_> = parts
        .iter()
        .filter_map(|part| part.get("functionCall"))
        .collect();
    let results: Vec<_> = parts
        .iter()
        .filter_map(|part| part.get("functionResponse"))
        .collect();
    assert_eq!(calls.len(), ids.len());
    assert_eq!(results.len(), ids.len());
    for (index, id) in ids.iter().enumerate() {
        assert_eq!(calls[index]["name"], "echo");
        assert_eq!(calls[index]["args"], values()[index % 2]);
        assert_eq!(results[index]["name"], "echo");
        assert_eq!(results[index]["response"]["output"], format!("result-{id}"));
        if modern {
            assert_eq!(calls[index]["id"], *id);
            assert_eq!(results[index]["id"], *id);
        } else {
            assert!(calls[index].get("id").is_none());
            assert!(results[index].get("id").is_none());
        }
    }
}

#[test]
fn repeated_tools_keep_distinct_ids_and_pair_results_across_fresh_native_processes() {
    let mut requests = 0;
    for api in ["google-generative-ai", "google-vertex"] {
        for model in ["gemini-2.5-pro", "gemini-3-flash-preview"] {
            for provided in [false, true] {
                let dir = Scratch::new("melchior-ready", "google-calls");
                let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
                let url = format!("http://{}/v1", listener.local_addr().expect("address"));
                config(&dir, &url, r#"{kind="api-key",vars={"FIXTURE_KEY"}}"#, api);
                let path = dir.join("providers.lua");
                let source = std::fs::read_to_string(&path).expect("fixture config");
                std::fs::write(path, source.replace("id=\"m\"", &format!("id={model:?}")))
                    .expect("model");
                let mut ask = Ask {
                    model: format!("fixture/{model}"),
                    about: String::new(),
                    wants: Wants::default(),
                    context: Context {
                        messages: vec![Message::user("Use echo twice")],
                        tools: vec![Tool {
                            name: "echo".into(),
                            description: "Echo".into(),
                            parameters: json!({"type":"object"}),
                            strict: None,
                        }],
                        ..Context::default()
                    },
                };
                let mut ids = Vec::new();
                for step in 0..3 {
                    let script = if step < 2 {
                        tools(step == 0 && provided)
                    } else {
                        format!(
                            "data: {}\n\n",
                            json!({"candidates":[{"content":{"parts":[{"text":"done"}]},"finishReason":"STOP"}]})
                        )
                    };
                    let server = response_server(listener.try_clone().expect("clone"), 200, script);
                    let context = ask.context.clone();
                    let events = presets::events(&ask_request(
                        &dir,
                        &[("FIXTURE_KEY", "private-sentinel")],
                        &ask,
                    ));
                    let (headers, body) = server.join().expect("server");
                    requests += 1;
                    assert_eq!(ask.context, context);
                    assert!(headers.contains(
                        &format!("/models/{model}:streamGenerateContent?alt=sse").to_lowercase()
                    ));
                    assert!(headers.contains(if api == "google-generative-ai" {
                        "x-goog-api-key: private-sentinel\r\n"
                    } else {
                        "authorization: bearer private-sentinel\r\n"
                    }));
                    paired(&body, &ids, model.starts_with("gemini-3"));
                    if step < 2 {
                        let next = collect(&events, &mut ask);
                        assert!(next.iter().all(|id| !ids.contains(id)));
                        if step == 0 && provided {
                            assert_eq!(next, ["provider-0", "provider-1"]);
                        }
                        ids.extend(next);
                    } else {
                        presets::stopped(&events, "end_turn");
                    }
                }
                presets::unasked(&listener);
            }
        }
    }
    assert_eq!(requests, 24);
}
