use super::*;
use melchior::mind::model::{Content, Image, Role};

const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

fn image() -> Image {
    Image {
        data: PNG.into(),
        media_type: "image/png".into(),
    }
}

fn arguments(id: &str) -> Value {
    if id == "a" {
        json!({"value":[null,[],{}]})
    } else {
        json!({"value":18446744073709551615u64})
    }
}

fn tools(row: &Value) -> String {
    if anthropic(row) {
        return super::super::anthropic::stream(
            &json!([
                {"type":"thinking","thinking":"reason α\n","signature":""},
                {"type":"text","text":"before"},
                {"type":"tool_use","id":"a","name":"echo","input":arguments("a")},
                {"type":"tool_use","id":"b","name":"echo","input":arguments("b")}
            ]),
            "tool_use",
        );
    }
    [json!({"choices":[{"delta":{"reasoning_content":"reason α\n","content":"before","tool_calls":[
        {"index":0,"id":"a","function":{"name":"echo","arguments":"{\"value\":"}},
        {"index":1,"id":"b","function":{"name":"echo","arguments":"{\"value\":"}}]}}]}),
        json!({"choices":[{"delta":{"tool_calls":[
            {"index":1,"function":{"arguments":"18446744073709551615}"}},
            {"index":0,"function":{"arguments":"[null,[],{}]}"}}]},"finish_reason":"tool_calls"}]})]
        .iter().map(|value| format!("data: {value}\n\n")).collect::<String>() + "data: [DONE]\n\n"
}

fn collected(said: &[Value]) -> Message {
    let text = |kind: &str| {
        said.iter()
            .filter(|e| e["event"] == kind)
            .filter_map(|e| e["text"].as_str())
            .collect::<String>()
    };
    let signature = said
        .iter()
        .find(|e| e["event"] == "signature")
        .and_then(|e| e["signature"].as_str())
        .map(str::to_owned);
    let mut message = Message::assistant(text("text"));
    if signature.is_some() || !text("thinking").is_empty() {
        message.content.insert(
            0,
            Content::Thinking {
                thinking: text("thinking"),
                signature,
            },
        );
    }
    let mut calls: Vec<(String, String)> = Vec::new();
    for event in said {
        match event["event"].as_str() {
            Some("tool_call_start") => {
                assert_eq!(event["name"], "echo");
                calls.push((event["id"].as_str().expect("ID").into(), String::new()));
            }
            Some("tool_call_args") => calls
                .last_mut()
                .expect("tool")
                .1
                .push_str(event["args"].as_str().expect("args")),
            _ => {}
        }
    }
    for (id, args) in calls {
        let value: Value = serde_json::from_str(&args).expect("JSON");
        assert_eq!(value, arguments(&id));
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
    ask: &Ask,
    response: String,
) -> (Vec<Value>, String, Value) {
    let before = serde_json::to_value(ask).expect("snapshot");
    let server = response_server(listener.try_clone().expect("listener"), 200, response);
    let said = events(&ask_request(dir, &[(KEY, "private-sentinel")], ask));
    let (headers, body) = server.join().expect("request");
    assert_eq!(serde_json::to_value(ask).expect("unchanged"), before);
    (said, headers, body)
}

#[test]
fn all_models_replay_real_streamed_tools_images_and_unsigned_thinking_after_later_users() {
    let (dir, listener, _) = fixture();
    for row in rows() {
        let vision = row["input"]
            .as_array()
            .expect("input")
            .contains(&json!("image"));
        let mut ask = request(&row);
        ask.wants.schema = None;
        ask.wants.thinking = Some(ThinkingLevel::High);
        ask.context.messages[0].content.push(Content::Image {
            data: PNG.into(),
            media_type: "image/png".into(),
        });
        let (said, headers, body) = round(&dir, &listener, &ask, tools(&row));
        presets::stopped(&said, "tool_use");
        let identity = affinity(&headers, &row);
        assert_eq!(body.to_string().contains(PNG), vision);
        let assistant = collected(&said);
        assert_eq!(
            assistant
                .content
                .iter()
                .filter(|c| matches!(c, Content::ToolCall { .. }))
                .count(),
            2
        );
        ask.context.messages.push(assistant);
        for id in ["a", "b"] {
            ask.context.messages.push(Message {
                role: Role::Tool,
                content: vec![Content::ToolResult {
                    id: id.into(),
                    name: "echo".into(),
                    content: format!("result-{id}"),
                    is_error: id == "b",
                    images: vec![image()],
                }],
                ..Message::user("")
            });
        }
        for later in [false, true] {
            let (said, headers, body) = round(&dir, &listener, &ask, finished(&row));
            presets::stopped(&said, "end_turn");
            assert_eq!(affinity(&headers, &row), identity);
            assert_eq!(
                body.to_string().matches(PNG).count(),
                if vision { 3 } else { 0 }
            );
            let messages = body["messages"].as_array().expect("messages");
            if anthropic(&row) {
                let blocks: Vec<_> = messages
                    .iter()
                    .flat_map(|m| m["content"].as_array().into_iter().flatten())
                    .collect();
                assert!(blocks.iter().any(|b| b["type"] == "thinking"
                    && b["thinking"] == "reason α\n"
                    && b["signature"] == ""));
                for id in ["a", "b"] {
                    let call = blocks
                        .iter()
                        .find(|b| b["type"] == "tool_use" && b["id"] == id)
                        .expect("tool");
                    assert_eq!(call["input"], arguments(id));
                    let result = blocks
                        .iter()
                        .find(|b| b["type"] == "tool_result" && b["tool_use_id"] == id)
                        .expect("result");
                    assert_eq!(result["is_error"], id == "b");
                    if vision {
                        assert_eq!(result["content"][0]["text"], format!("result-{id}"));
                    } else {
                        assert!(
                            result["content"]
                                .to_string()
                                .contains(&format!("result-{id}"))
                        );
                    }
                }
            } else {
                let assistant = messages
                    .iter()
                    .find(|m| m["tool_calls"].is_array())
                    .expect("calls");
                assert_eq!(assistant["reasoning_content"], "reason α\n");
                assert_eq!(assistant["tool_calls"].as_array().expect("tools").len(), 2);
                for (index, id) in ["a", "b"].into_iter().enumerate() {
                    assert_eq!(
                        serde_json::from_str::<Value>(
                            assistant["tool_calls"][index]["function"]["arguments"]
                                .as_str()
                                .expect("arguments")
                        )
                        .expect("JSON"),
                        arguments(id)
                    );
                    assert!(messages.iter().any(|m| {
                        m["tool_call_id"] == id
                            && m["content"]
                                .as_str()
                                .is_some_and(|text| text.contains(&format!("result-{id}")))
                    }));
                }
            }
            if later {
                assert!(
                    messages
                        .last()
                        .expect("later user")
                        .to_string()
                        .contains("later")
                );
            }
            ask.context.messages.push(collected(&said));
            ask.context.messages.push(Message::user("later"));
        }
        unasked(&listener);
    }
}

#[test]
fn tool_updates_preserve_declared_positions_fallbacks_and_strict_guarantees() {
    let (dir, listener, _) = fixture();
    for row in rows() {
        for removal in [false, true] {
            let mut ask = request(&row);
            ask.wants.schema = None;
            ask.context.tools[0].strict = Some(if anthropic(&row) {
                ToolStrictness::Prefer
            } else {
                ToolStrictness::Require
            });
            let mut added = ask.context.tools[0].clone();
            added.name = "extra".into();
            ask.context.messages.push(Message::tool_changes(
                vec![added.clone()],
                if removal { vec!["echo".into()] } else { vec![] },
            ));
            ask.context.messages.push(Message::user("later"));
            let (said, _, body) = round(&dir, &listener, &ask, finished(&row));
            presets::stopped(&said, "end_turn");
            let positional = !removal && row["compat"]["supportsMidConvoToolAdditions"] == true;
            let top = body["tools"].as_array().expect("tools");
            assert_eq!(top.len(), if removal || positional { 1 } else { 2 });
            let inline: Vec<_> = body["messages"]
                .as_array()
                .expect("messages")
                .iter()
                .filter(|m| m.get("tools").is_some())
                .collect();
            assert_eq!(inline.len(), usize::from(positional));
            let declarations: Vec<_> = top
                .iter()
                .chain(
                    inline
                        .iter()
                        .flat_map(|m| m["tools"].as_array().expect("inline")),
                )
                .collect();
            let tool = declarations
                .iter()
                .find(|t| {
                    if anthropic(&row) {
                        t["name"] == "extra"
                    } else {
                        t["function"]["name"] == "extra"
                    }
                })
                .expect("extra tool");
            if anthropic(&row) {
                assert_eq!(tool["input_schema"], added.parameters);
                assert!(tool.get("strict").is_none());
            } else {
                assert_eq!(tool["function"]["parameters"], added.parameters);
                assert_eq!(tool["function"]["strict"], true);
            }
        }
    }
}
