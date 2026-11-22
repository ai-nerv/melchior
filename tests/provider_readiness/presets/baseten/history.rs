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
fn all_models_replay_parallel_tools_and_reasoning_through_later_user_turns() {
    let (dir, listener) = fixture();
    let mut total = 0;
    for model in MODELS {
        let mut ask = ask(model);
        let server = response_server(listener.try_clone().expect("listener"), 200, tool_stream());
        let said = run(&dir, &ask);
        stopped(&said, "tool_use");
        let (headers, body) = server.join().expect("request");
        request(model, &headers, &body);
        let key = affinity(&headers);
        let message = collected(&said);
        let calls: Vec<_> = message
            .content
            .iter()
            .filter_map(|content| match content {
                Content::ToolCall { id, arguments, .. } => Some((id.clone(), arguments.clone())),
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
        for (id, _) in &calls {
            let images = if image_model(model) {
                json!([{"media_type":"image/png","data":PNG}])
            } else {
                json!([])
            };
            ask.context.messages.push(serde_json::from_value(json!({"role":"tool","content":[{
                "type":"tool_result","id":id,"name":"echo","content":format!("result-{id}"),"is_error":false,"images":images}]})).expect("result"));
        }
        for later in [false, true] {
            let before = ask.context.clone();
            let server = response_server(listener.try_clone().expect("listener"), 200, finished());
            let said = run(&dir, &ask);
            stopped(&said, "end_turn");
            let (headers, body) = server.join().expect("followup");
            request(model, &headers, &body);
            controls(model, ask.wants.thinking, &body);
            assert_eq!(affinity(&headers), key);
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
            assert_eq!(body.to_string().contains(PNG), image_model(model));
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
    assert_eq!(total, 45);
}

#[test]
fn changed_tools_fold_at_request_root_and_keep_strict_schema_without_context_mutation() {
    let (dir, listener) = fixture();
    for model in MODELS {
        for removal in [false, true] {
            let mut ask = ask(model);
            let mut added = ask.context.tools[0].clone();
            added.name = "extra".into();
            ask.context.messages.push(Message::tool_changes(
                vec![added.clone()],
                if removal { vec!["echo".into()] } else { vec![] },
            ));
            ask.context.messages.push(Message::user("later"));
            let before = ask.context.clone();
            let server = response_server(listener.try_clone().expect("listener"), 200, finished());
            stopped(&run(&dir, &ask), "end_turn");
            let (headers, body) = server.join().expect("request");
            request(model, &headers, &body);
            let declarations = body["tools"].as_array().expect("tools");
            assert_eq!(declarations.len(), if removal { 1 } else { 2 });
            assert_eq!(
                declarations
                    .iter()
                    .any(|tool| tool["function"]["name"] == "echo"),
                !removal
            );
            let tool = declarations
                .iter()
                .find(|tool| tool["function"]["name"] == "extra")
                .expect("extra");
            assert_eq!(tool["function"]["parameters"], added.parameters);
            assert_eq!(tool["function"]["strict"], true);
            assert!(
                body["messages"]
                    .as_array()
                    .expect("messages")
                    .iter()
                    .all(|message| message.get("tools").is_none())
            );
            assert_eq!(ask.context, before);
        }
    }
}
