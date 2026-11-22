use super::*;
use melchior::mind::model::{Content, Image, Role};

const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

fn args(id: &str) -> Value {
    if id == "a" {
        json!({"value":[null,[],{}]})
    } else {
        json!({"value":18446744073709551615u64})
    }
}

fn tools() -> String {
    [json!({"choices":[{"delta":{"reasoning_content":"reason α\n","content":"before","tool_calls":[
        {"index":0,"id":"a","function":{"name":"echo","arguments":"{\"value\":"}},
        {"index":1,"id":"b","function":{"name":"echo","arguments":"{\"value\":"}}]}}]}),
        json!({"choices":[{"delta":{"tool_calls":[
            {"index":1,"function":{"arguments":"18446744073709551615}"}},
            {"index":0,"function":{"arguments":"[null,[],{}]}"}}]},"finish_reason":"tool_calls"}]})]
        .iter().map(|value| format!("data: {value}\n\n")).collect::<String>() + "data: [DONE]\n\n"
}

fn collected(said: &[Value]) -> Message {
    let text = |kind| {
        said.iter()
            .filter(|event| event["event"] == kind)
            .filter_map(|event| event["text"].as_str())
            .collect::<String>()
    };
    let mut message = Message::assistant(text("text"));
    if !text("thinking").is_empty() {
        message.content.insert(
            0,
            Content::Thinking {
                thinking: text("thinking"),
                signature: None,
            },
        );
    }
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
    for (id, value) in calls {
        let value: Value = serde_json::from_str(&value).expect("JSON");
        assert_eq!(value, args(&id));
        message.content.push(Content::ToolCall {
            id,
            name: "echo".into(),
            arguments: value,
            thought_signature: None,
        });
    }
    message
}

fn round(
    dir: &Scratch,
    listener: &TcpListener,
    key: &str,
    ask: &Ask,
    reply: String,
) -> (Vec<Value>, Value) {
    let before = serde_json::to_value(ask).expect("snapshot");
    let server = response_server(listener.try_clone().expect("listener"), 200, reply);
    let said = presets::events(&ask_request(dir, &[(key, "private-sentinel")], ask));
    let (headers, body) = server.join().expect("request");
    assert!(headers.starts_with("post /compatible-mode/v1/chat/completions "));
    assert!(headers.contains("authorization: bearer private-sentinel\r\n"));
    assert_eq!(body["max_completion_tokens"], 128);
    assert!(body.get("max_tokens").is_none());
    assert_eq!(serde_json::to_value(ask).expect("unchanged"), before);
    (said, body)
}

#[test]
fn all_shipped_models_replay_streamed_parallel_tools_images_and_later_user_reasoning() {
    let mut requests = 0;
    for (id, key, fallback, count) in PLANS {
        let (dir, listener) = fixture(id, key, fallback, count);
        for row in rows(id) {
            let vision = row["input"]
                .as_array()
                .expect("input")
                .contains(&json!("image"));
            let mut ask = request(id, &row);
            ask.wants.schema = None;
            ask.wants.thinking = Some(
                if row["thinkingLevelMap"]["high"].is_null()
                    && row["thinkingLevelMap"]["low"].is_string()
                {
                    ThinkingLevel::Low
                } else {
                    ThinkingLevel::High
                },
            );
            ask.context.tools[0].strict = None;
            ask.context.tools[0].parameters = json!({"type":"object"});
            ask.context.messages[0].content.push(Content::Image {
                data: PNG.into(),
                media_type: "image/png".into(),
            });
            let (said, body) = round(&dir, &listener, key, &ask, tools());
            requests += 1;
            presets::stopped(&said, "tool_use");
            assert_eq!(body.to_string().contains(PNG), vision);
            let assistant = collected(&said);
            assert_eq!(
                assistant
                    .content
                    .iter()
                    .filter(|block| matches!(block, Content::ToolCall { .. }))
                    .count(),
                2
            );
            ask.context.messages.push(assistant);
            for tool_id in ["a", "b"] {
                ask.context.messages.push(Message {
                    role: Role::Tool,
                    content: vec![Content::ToolResult {
                        id: tool_id.into(),
                        name: "echo".into(),
                        content: format!("result-{tool_id}"),
                        is_error: tool_id == "b",
                        images: vec![Image {
                            data: PNG.into(),
                            media_type: "image/png".into(),
                        }],
                    }],
                    ..Message::user("")
                });
            }
            for later in [false, true] {
                let (said, body) = round(&dir, &listener, key, &ask, finished());
                requests += 1;
                presets::stopped(&said, "end_turn");
                assert_eq!(
                    body.to_string().matches(PNG).count(),
                    if vision { 3 } else { 0 }
                );
                let messages = body["messages"].as_array().expect("messages");
                let assistant = messages
                    .iter()
                    .find(|message| message["tool_calls"].is_array())
                    .expect("calls");
                assert_eq!(assistant["reasoning_content"], "reason α\n");
                assert_eq!(assistant["tool_calls"].as_array().expect("tools").len(), 2);
                for (index, tool_id) in ["a", "b"].into_iter().enumerate() {
                    assert_eq!(
                        serde_json::from_str::<Value>(
                            assistant["tool_calls"][index]["function"]["arguments"]
                                .as_str()
                                .expect("args")
                        )
                        .expect("JSON"),
                        args(tool_id)
                    );
                    assert!(messages.iter().any(|message| {
                        message["tool_call_id"] == tool_id
                            && message["content"]
                                .as_str()
                                .is_some_and(|text| text.contains(&format!("result-{tool_id}")))
                    }));
                }
                if later {
                    assert!(
                        messages
                            .last()
                            .expect("last user")
                            .to_string()
                            .contains("later")
                    );
                }
                ask.context.messages.push(collected(&said));
                ask.context.messages.push(Message::user("later"));
            }
            presets::unasked(&listener);
        }
    }
    assert_eq!(requests, 198);
}

#[test]
fn dynamic_tool_additions_and_removals_fold_without_losing_required_strictness() {
    let mut requests = 0;
    for (id, key, fallback, count) in PLANS {
        let (dir, listener) = fixture(id, key, fallback, count);
        for row in rows(id) {
            for remove in [false, true] {
                let mut ask = request(id, &row);
                ask.wants.schema = None;
                let mut extra = ask.context.tools[0].clone();
                extra.name = "extra".into();
                ask.context.messages.push(Message::tool_changes(
                    vec![extra.clone()],
                    if remove { vec!["echo".into()] } else { vec![] },
                ));
                ask.context.messages.push(Message::user("later"));
                let (said, body) = round(&dir, &listener, key, &ask, finished());
                requests += 1;
                presets::stopped(&said, "end_turn");
                assert!(
                    body["messages"]
                        .as_array()
                        .expect("messages")
                        .iter()
                        .all(|message| message.get("tools").is_none())
                );
                let tools = body["tools"].as_array().expect("tools");
                assert_eq!(tools.len(), if remove { 1 } else { 2 });
                let tool = tools
                    .iter()
                    .find(|tool| tool["function"]["name"] == "extra")
                    .expect("extra");
                assert_eq!(tool["function"]["strict"], true);
                assert_eq!(tool["function"]["parameters"], extra.parameters);
            }
        }
    }
    assert_eq!(requests, 132);
}
