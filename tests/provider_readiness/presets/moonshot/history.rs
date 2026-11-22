use super::*;

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
fn parallel_tools_and_images_replay_complete_reasoning_across_later_user_turns() {
    let mut total = 0;
    for (id, key, url) in REGIONS {
        let (dir, listener) = fixture(id, key, url);
        for model in models(id) {
            let mut ask = ask_for(id, model);
            let server =
                response_server(listener.try_clone().expect("listener"), 200, tool_stream());
            let said = events(&ask_request(&dir, &[(key, "private-sentinel")], &ask));
            stopped(&said, "tool_use");
            let (headers, body) = server.join().expect("request");
            request(id, model, &headers, &body);
            controls(model, ask.wants.thinking, &body);
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
                    "images":[{"media_type":"image/png","data":PNG}]}]})).expect("result"));
            }
            for later in [false, true] {
                let before = ask.context.clone();
                let server =
                    response_server(listener.try_clone().expect("listener"), 200, finished());
                let said = events(&ask_request(&dir, &[(key, "private-sentinel")], &ask));
                stopped(&said, "end_turn");
                let (headers, body) = server.join().expect("followup");
                request(id, model, &headers, &body);
                controls(model, ask.wants.thinking, &body);
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
                    assert_eq!(messages[index + 3]["content"], format!("result-{call}"));
                }
                assert!(body.to_string().contains(PNG));
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
            total += 3;
        }
    }
    assert_eq!(total, 54);
}

#[test]
fn dynamic_declarations_use_pinned_capabilities_and_removals_fold_without_schema_loss() {
    let mut total = 0;
    for (id, key, url) in REGIONS {
        let (dir, listener) = fixture(id, key, url);
        for model in models(id) {
            for removal in [false, true] {
                let mut ask = ask_for(id, model);
                let mut tool = ask.context.tools[0].clone();
                tool.name = "extra".into();
                tool.parameters = json!({"type":"object","properties":{},"required":[],"examples":[null,[],{},u64::MAX]});
                let removed = if removal { vec!["echo".into()] } else { vec![] };
                ask.context
                    .messages
                    .push(Message::tool_changes(vec![tool.clone()], removed));
                ask.context.messages.push(Message::user("later"));
                let before = ask.context.clone();
                let server =
                    response_server(listener.try_clone().expect("listener"), 200, finished());
                let said = events(&ask_request(&dir, &[(key, "private-sentinel")], &ask));
                stopped(&said, "end_turn");
                let (headers, body) = server.join().expect("request");
                request(id, model, &headers, &body);
                let native = !binary(model) && !removal;
                let declarations: Vec<_> = body["messages"]
                    .as_array()
                    .expect("messages")
                    .iter()
                    .filter(|message| message.get("tools").is_some())
                    .collect();
                assert_eq!(declarations.len(), usize::from(native));
                let added = if native {
                    assert_eq!(declarations[0]["role"], "system");
                    assert!(declarations[0].get("content").is_none());
                    assert_eq!(body["messages"][2], *declarations[0]);
                    &declarations[0]["tools"][0]
                } else {
                    body["tools"]
                        .as_array()
                        .expect("tools")
                        .iter()
                        .find(|tool| tool["function"]["name"] == "extra")
                        .expect("extra")
                };
                assert_eq!(added["function"]["parameters"], tool.parameters);
                assert!(added["function"].get("strict").is_none());
                let initial = body["tools"].as_array().expect("initial");
                assert_eq!(
                    initial
                        .iter()
                        .any(|tool| tool["function"]["name"] == "echo"),
                    !removal
                );
                assert_eq!(initial.len(), if native || removal { 1 } else { 2 });
                assert_eq!(ask.context, before);
                total += 1;
            }
        }
    }
    assert_eq!(total, 36);
}

#[test]
fn authentication_and_truncated_arguments_are_terminal_without_retry_or_identity_fallback() {
    for (id, key, url) in REGIONS {
        let (dir, listener) = fixture(id, key, url);
        for (status, response) in [
            (401,r#"{"error":{"message":"Authentication failed"}}"#.into()),
            (200,"data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"a\",\"function\":{\"name\":\"echo\",\"arguments\":\"{\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\ndata: [DONE]\n\n".into()),
        ] {
            let model = models(id)[0];
            let server = response_server(listener.try_clone().expect("listener"), status, response);
            let said = events(&ask_request(&dir, &[(key,"private-sentinel")], &ask_for(id,model)));
            let (headers, body) = server.join().expect("request");
            request(id, model, &headers, &body);
            assert!(!said.iter().any(|e| e["event"] == "retrying"));
            if status == 200 { stopped(&said,"error"); }
            else {
                assert!(said.iter().all(|e| matches!(e["event"].as_str(), Some("request_frame" | "failed"))), "{said:?}");
                let failures: Vec<_> = said.iter().filter(|e| e["event"] == "failed").collect();
                assert_eq!(failures.len(), 1);
                assert_eq!(failures[0]["why"], "auth");
                assert_eq!(said.last().expect("terminal")["event"], "failed");
            }
            unasked(&listener);
        }
    }
}

#[test]
fn retries_preserve_regional_routes_controls_and_request_identity() {
    for (id, key, url) in REGIONS {
        let (dir, listener) = fixture(id, key, url);
        let model = models(id)[0];
        let ask = ask_for(id, model);
        let before = serde_json::to_value(&ask).expect("ask");
        let server = std::thread::spawn(move || {
            let first = response_server(
                listener.try_clone().expect("listener"),
                503,
                r#"{"error":{"message":"Service overloaded"}}"#.into(),
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
            &ask,
            std::time::Duration::from_secs(30),
        ));
        assert_eq!(said.iter().filter(|e| e["event"] == "retrying").count(), 1);
        assert!(!said.iter().any(|e| e["event"] == "failed"));
        let stops: Vec<_> = said.iter().filter(|e| e["event"] == "stop").collect();
        assert_eq!(stops.len(), 1);
        assert_eq!(stops[0]["reason"], "end_turn");
        let (first, second) = server.join().expect("server");
        request(id, model, &first.0, &first.1);
        request(id, model, &second.0, &second.1);
        controls(model, ask.wants.thinking, &second.1);
        assert_eq!(first.1, second.1);
        assert_eq!(serde_json::to_value(&ask).expect("unchanged"), before);
    }
}
