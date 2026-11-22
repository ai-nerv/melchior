use super::*;

const APIS: [&str; 3] = [
    "openai-responses",
    "azure-openai-responses",
    "openai-codex-responses",
];
const A: &str = r#"{"a":[null,[],{}]}"#;
const B: &str = r#"{"b":18446744073709551615}"#;

fn fixture(api: &str) -> (Scratch, TcpListener, Ask) {
    let dir = Scratch::new("melchior-ready", "responses-streams");
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let url = format!("http://{}/v1", listener.local_addr().expect("address"));
    config(&dir, &url, r#"{kind="api-key",vars={"FIXTURE_KEY"}}"#, api);
    let path = dir.join("providers.lua");
    let config = std::fs::read_to_string(&path).expect("config");
    std::fs::write(
        path,
        config.replace("name=\"Model\"", "name=\"Model\",reasoning=true"),
    )
    .expect("reasoning");
    let ask = serde_json::from_value(json!({"model":"fixture/m","context":{
        "messages":[{"role":"user","content":[{"type":"text","text":"Use echo twice"}]}],
        "tools":[{"name":"echo","description":"Echo data","parameters":{"type":"object","properties":{}}}]
    },"wants":{"max_tokens":128}})).expect("ask");
    (dir, listener, ask)
}

fn call(id: &str, arguments: &str) -> Value {
    json!({"type":"function_call","id":format!("fc_{id}"),"call_id":id,"name":"echo","arguments":arguments})
}

fn item(kind: &str, index: usize, value: Value) -> Value {
    json!({"type":format!("response.output_item.{kind}"),"output_index":index,"item":value})
}

fn args(index: usize, id: &str, delta: &str) -> Value {
    json!({"type":"response.function_call_arguments.delta","output_index":index,
        "item_id":format!("fc_{id}"),"delta":delta})
}

fn terminal(kind: &str, output: Option<Value>) -> Value {
    let mut value = json!({"type":format!("response.{kind}"),"response":{
        "usage":{"input_tokens":90,"output_tokens":12,"input_tokens_details":{"cached_tokens":30}},
        "incomplete_details":{"reason":"max_output_tokens"}}});
    if let Some(output) = output {
        value["response"]["output"] = output;
    }
    value
}

fn invoke(
    dir: &Scratch,
    listener: &TcpListener,
    ask: &Ask,
    script: Vec<Value>,
) -> (Vec<Value>, Value) {
    let stream = script
        .into_iter()
        .map(|value| format!("data: {value}\n\n"))
        .collect();
    let worker = response_server(listener.try_clone().expect("clone"), 200, stream);
    let said = events(&ask_request(
        dir,
        &[("FIXTURE_KEY", "private-sentinel")],
        ask,
    ));
    assert!(
        !said
            .iter()
            .any(|e| matches!(e["event"].as_str(), Some("retrying" | "failed"))),
        "{said:?}"
    );
    let (_, body) = worker.join().expect("HTTP request");
    assert_eq!(body["store"], false);
    assert!(body.get("__scratch").is_none());
    assert!(body.get("__budget").is_none());
    (said, body)
}

fn assembled(events: &[Value]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for event in events {
        match event["event"].as_str() {
            Some("tool_call_start") => {
                assert_eq!(event["name"], "echo");
                out.push((event["id"].as_str().expect("call id").into(), String::new()));
            }
            Some("tool_call_args") => out
                .last_mut()
                .expect("active call")
                .1
                .push_str(event["args"].as_str().expect("args")),
            _ => {}
        }
    }
    out
}

fn stop(events: &[Value], reason: &str) {
    let stops: Vec<_> = events.iter().filter(|e| e["event"] == "stop").collect();
    assert_eq!(stops.len(), 1, "{events:?}");
    assert_eq!(stops[0]["reason"], reason);
    let usage = events
        .iter()
        .find(|e| e["event"] == "spent")
        .expect("usage");
    assert_eq!(usage["usage"]["input"], 60);
    assert_eq!(usage["usage"]["cache_read"], 30);
    assert_eq!(usage["usage"]["output"], 12);
}

#[test]
fn interleaved_and_done_only_calls_have_exact_native_followups_in_each_hosting() {
    let reasoning = json!({"type":"reasoning","id":"rs","summary":[],"encrypted_content":"private opaque α+/="});
    let output = json!([reasoning, call("a", A), call("b", B)]);
    for api in APIS {
        for done_only in [false, true] {
            let (dir, listener, mut ask) = fixture(api);
            let source = if done_only {
                vec![
                    item("done", 2, call("b", B)),
                    item("done", 0, reasoning.clone()),
                    item("done", 1, call("a", A)),
                    terminal("completed", None),
                ]
            } else {
                vec![
                    item("done", 0, reasoning.clone()),
                    item("added", 1, call("a", "")),
                    args(1, "a", "{\"a\":"),
                    item("added", 2, call("b", "")),
                    args(2, "b", B),
                    item("done", 2, call("b", B)),
                    args(1, "a", "[null,[],{}]}"),
                    item("done", 1, call("a", A)),
                    terminal("completed", Some(output.clone())),
                ]
            };
            let (said, _) = invoke(&dir, &listener, &ask, source);
            stop(&said, "tool_use");
            let calls = assembled(&said);
            assert_eq!(calls, vec![("a".into(), A.into()), ("b".into(), B.into())]);
            let signature = said
                .iter()
                .find(|e| e["event"] == "signature")
                .expect("signature")["signature"]
                .as_str()
                .expect("signature");
            let mut message = Message::assistant("");
            message.content = vec![Content::Thinking {
                thinking: String::new(),
                signature: Some(signature.into()),
            }];
            for (id, arguments) in &calls {
                message.content.push(Content::ToolCall {
                    id: id.clone(),
                    name: "echo".into(),
                    arguments: serde_json::from_str(arguments).expect("exact JSON"),
                    thought_signature: None,
                });
            }
            ask.context.messages.push(message);
            for (id, _) in calls {
                ask.context.messages.push(Message {
                    role: Role::Tool,
                    content: vec![Content::ToolResult {
                        images: vec![],
                        id,
                        name: "echo".into(),
                        content: "ok".into(),
                        is_error: false,
                    }],
                    stop_reason: None,
                    usage: None,
                    error: None,
                });
            }
            let before = ask.context.clone();
            let (said, body) = invoke(
                &dir,
                &listener,
                &ask,
                vec![
                    json!({"type":"response.output_text.delta","delta":"done"}),
                    terminal("completed", Some(json!([]))),
                ],
            );
            stop(&said, "end_turn");
            for index in 0..3 {
                assert_eq!(body["input"][index + 1], output[index], "{api} {done_only}");
            }
            assert_eq!(body["input"][4]["call_id"], "a");
            assert_eq!(body["input"][5]["call_id"], "b");
            assert_eq!(ask.context, before);
        }
    }
}

#[test]
fn incomplete_and_conflicting_native_streams_never_advertise_executable_completion() {
    for api in APIS {
        for conflict in [false, true] {
            let (dir, listener, ask) = fixture(api);
            let mut source = vec![
                item("added", 0, call("a", "")),
                args(0, "a", "{\"a\":"),
                item("added", 1, call("b", "")),
                args(1, "b", "["),
            ];
            if conflict {
                source.push(item("done", 0, call("different", "{}")));
            }
            source.push(terminal(
                if conflict { "completed" } else { "incomplete" },
                None,
            ));
            let (said, _) = invoke(&dir, &listener, &ask, source);
            stop(&said, if conflict { "error" } else { "length" });
            assert!(!said.iter().any(|e| e["event"] == "signature"));
            if !conflict {
                assert_eq!(
                    assembled(&said),
                    vec![("a".into(), "{\"a\":".into()), ("b".into(), "[".into())]
                );
            }
        }
    }
}
