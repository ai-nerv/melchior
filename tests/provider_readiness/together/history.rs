use super::*;
use melchior::mind::model::{Content, Image, Role};

const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

fn arguments(id: &str) -> Value {
    if id == "a" {
        json!({"value":[null,[],{}]})
    } else {
        json!({"value":18446744073709551615u64})
    }
}

fn tools(field: &str) -> String {
    let mut first = json!({"choices":[{"delta":{"content":"before","tool_calls":[
        {"index":0,"id":"a","function":{"name":"echo","arguments":"{\"value\":"}},
        {"index":1,"id":"b","function":{"name":"echo","arguments":"{\"value\":"}}]}}]});
    first["choices"][0]["delta"][field] = json!("reason α\n");
    let last = json!({"choices":[{"delta":{"tool_calls":[
        {"index":1,"function":{"arguments":"18446744073709551615}"}},
        {"index":0,"function":{"arguments":"[null,[],{}]}"}}]},"finish_reason":"tool_calls"}]});
    format!("data: {first}\n\ndata: {last}\n\ndata: [DONE]\n\n")
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
        let signature = said
            .iter()
            .find(|event| event["event"] == "signature")
            .and_then(|event| event["signature"].as_str())
            .expect("reasoning field");
        message.content.insert(
            0,
            Content::Thinking {
                thinking: text("thinking"),
                signature: Some(signature.into()),
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

#[test]
fn all_models_replay_both_reasoning_fields_parallel_tools_images_and_later_users() {
    let (dir, listener) = fixture();
    let mut requests = 0;
    for row in rows() {
        let vision = row["input"]
            .as_array()
            .expect("input")
            .contains(&json!("image"));
        for field in ["reasoning", "reasoning_content"] {
            let mut ask = request(&row);
            ask.wants.schema = None;
            ask.wants.thinking = Some(ThinkingLevel::High);
            ask.context.tools[0].strict = None;
            ask.context.tools[0].parameters = json!({"type":"object"});
            ask.context.messages[0].content.push(Content::Image {
                data: PNG.into(),
                media_type: "image/png".into(),
            });
            let (said, body) = round(&dir, &listener, &row, &ask, tools(field));
            requests += 1;
            presets::stopped(&said, "tool_use");
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
                        images: vec![Image {
                            data: PNG.into(),
                            media_type: "image/png".into(),
                        }],
                    }],
                    ..Message::user("")
                });
            }
            for later in [false, true] {
                let (said, body) = round(&dir, &listener, &row, &ask, finished());
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
                assert_eq!(assistant[field], "reason α\n");
                assert!(
                    assistant
                        .get(if field == "reasoning" {
                            "reasoning_content"
                        } else {
                            "reasoning"
                        })
                        .is_none()
                );
                assert_eq!(assistant["tool_calls"].as_array().expect("tools").len(), 2);
                for (index, id) in ["a", "b"].into_iter().enumerate() {
                    assert_eq!(
                        serde_json::from_str::<Value>(
                            assistant["tool_calls"][index]["function"]["arguments"]
                                .as_str()
                                .expect("args")
                        )
                        .expect("JSON"),
                        arguments(id)
                    );
                    assert!(messages.iter().any(|message| {
                        message["tool_call_id"] == id
                            && message["content"]
                                .as_str()
                                .is_some_and(|text| text.contains(&format!("result-{id}")))
                    }));
                }
                if later {
                    assert!(messages.last().expect("last").to_string().contains("later"));
                }
                ask.context.messages.push(collected(&said));
                ask.context.messages.push(Message::user("later"));
            }
        }
        presets::unasked(&listener);
    }
    assert_eq!(requests, 96);
}

#[test]
fn tool_updates_fold_exact_schemas_and_keep_unsupported_strictness_absent() {
    let (dir, listener) = fixture();
    let mut requests = 0;
    for row in rows() {
        for remove in [false, true] {
            let mut ask = request(&row);
            let mut extra = ask.context.tools[0].clone();
            extra.name = "extra".into();
            ask.context.messages.push(Message::tool_changes(
                vec![extra.clone()],
                if remove { vec!["echo".into()] } else { vec![] },
            ));
            ask.context.messages.push(Message::user("later"));
            let (said, body) = round(&dir, &listener, &row, &ask, finished());
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
            let extra_wire = tools
                .iter()
                .find(|tool| tool["function"]["name"] == "extra")
                .expect("extra");
            assert_eq!(extra_wire["function"]["parameters"], extra.parameters);
            assert!(
                tools
                    .iter()
                    .all(|tool| tool["function"].get("strict").is_none())
            );
        }
    }
    assert_eq!(requests, 32);
}
