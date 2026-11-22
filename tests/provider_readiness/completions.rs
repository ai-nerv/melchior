use super::*;
use melchior::mind::model::{Content, Message};
use melchior::mind::wire::Ask;

const A: &str = r#"{"a":[null,[],{}]}"#;
const B: &str = r#"{"b":18446744073709551615}"#;

fn fixture() -> (Scratch, TcpListener, Ask) {
    let dir = Scratch::new("melchior-ready", "chat-stream");
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let url = format!("http://{}/v1", listener.local_addr().expect("address"));
    config(
        &dir,
        &url,
        r#"{kind="api-key",vars={"FIXTURE_KEY"}}"#,
        "openai-completions",
    );
    let ask = serde_json::from_value(json!({"model":"fixture/m","context":{
        "messages":[{"role":"user","content":[{"type":"text","text":"Use echo twice"}]}],
        "tools":[{"name":"echo","description":"Echo data","parameters":{"type":"object"}}]
    },"wants":{"max_tokens":128}}))
    .expect("ask");
    (dir, listener, ask)
}

fn chunk(calls: Value) -> Value {
    json!({"choices":[{"index":0,"delta":{"tool_calls":calls}}]})
}

fn call(index: usize, id: &str, args: &str) -> Value {
    json!({"index":index,"id":id,"function":{"name":"echo","arguments":args}})
}

fn terminal(reason: &str) -> Value {
    json!({"choices":[{"delta":{},"finish_reason":reason}]})
}

fn usage() -> Value {
    json!({"choices":[],"usage":{"prompt_tokens":90,"completion_tokens":12,"prompt_tokens_details":{"cached_tokens":30}}})
}

fn stream(script: Vec<Value>, sentinel: bool) -> String {
    let mut text: String = script
        .into_iter()
        .map(|value| format!("data: {value}\n\n"))
        .collect();
    if sentinel {
        text.push_str("data: [DONE]\n\n");
    }
    text
}

fn invoke(
    dir: &Scratch,
    listen: &TcpListener,
    ask: &Ask,
    script: Vec<Value>,
) -> (Vec<Value>, Value) {
    let worker = response_server(
        listen.try_clone().expect("listener"),
        200,
        stream(script, true),
    );
    let output = ask_request(dir, &[("FIXTURE_KEY", "private-sentinel")], ask);
    let events = presets::events(&output);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("private-sentinel"));
    assert!(
        !events
            .iter()
            .any(|e| matches!(e["event"].as_str(), Some("failed" | "retrying"))),
        "{events:?}"
    );
    let (headers, body) = worker.join().expect("request");
    assert!(headers.starts_with("post /v1/chat/completions "));
    assert!(headers.contains("authorization: bearer private-sentinel\r\n"));
    assert!(body.get("__scratch").is_none());
    (events, body)
}

fn assembled(events: &[Value]) -> Vec<(String, String)> {
    let mut calls = Vec::new();
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
                .push_str(event["args"].as_str().expect("args")),
            Some("retrying") => calls.clear(),
            _ => {}
        }
    }
    calls
}

fn completed() -> Vec<Value> {
    vec![
        chunk(json!([call(0, "a", "{\"a\":"), call(1, "b", "{\"b\":")])),
        chunk(
            json!([{"index":1,"function":{"arguments":"18446744073709551615}"}},
            {"index":0,"function":{"arguments":"[null,[],{}]}"}}]),
        ),
        terminal("tool_calls"),
        usage(),
    ]
}

#[test]
fn interleaved_native_calls_keep_exact_arguments_results_and_later_user_history() {
    let (dir, listener, mut ask) = fixture();
    let (events, _) = invoke(&dir, &listener, &ask, completed());
    presets::stopped(&events, "tool_use");
    let calls = assembled(&events);
    assert_eq!(calls, vec![("a".into(), A.into()), ("b".into(), B.into())]);
    let mut assistant = Message::assistant("");
    assistant.content = calls
        .iter()
        .map(|(id, args)| Content::ToolCall {
            id: id.clone(),
            name: "echo".into(),
            arguments: serde_json::from_str(args).expect("arguments"),
            thought_signature: None,
        })
        .collect();
    ask.context.messages.push(assistant);
    for (id, _) in &calls {
        ask.context.messages.push(serde_json::from_value(json!({"role":"tool","content":[{
            "type":"tool_result","id":id,"name":"echo","content":format!("result-{id}"),"is_error":false
        }]})).expect("result"));
    }
    for _ in 0..2 {
        let before = ask.context.clone();
        let (events, body) = invoke(
            &dir,
            &listener,
            &ask,
            vec![
                json!({"choices":[{"delta":{"content":"done"},"finish_reason":"stop"}]}),
                usage(),
            ],
        );
        presets::stopped(&events, "end_turn");
        for (index, (id, args)) in calls.iter().enumerate() {
            let tool = &body["messages"][1]["tool_calls"][index];
            assert_eq!(tool["id"], *id);
            assert_eq!(tool["function"]["arguments"], *args);
            assert_eq!(
                body["messages"][index + 2],
                json!({"role":"tool","tool_call_id":id,"content":format!("result-{id}")})
            );
        }
        assert_eq!(ask.context, before);
        ask.context.messages.push(Message::assistant("done"));
        ask.context.messages.push(Message::user("another round"));
    }
}

#[test]
fn delayed_and_repeated_identity_and_indexless_id_routing_work_through_native_transport() {
    for script in [
        vec![
            chunk(json!([call(1, "b", B)])),
            chunk(json!([{"index":0,"function":{"arguments":A}}])),
            chunk(json!([{"index":0,"id":"a"}])),
            chunk(json!([{"index":0,"function":{"name":"echo"}}])),
        ],
        vec![
            chunk(json!([{"id":"a","function":{"name":"echo","arguments":A}},
            {"id":"b","function":{"name":"echo","arguments":B}}])),
            chunk(json!([{"id":"b","function":{"name":"echo"}}])),
        ],
    ] {
        let (dir, listener, ask) = fixture();
        let (events, _) = invoke(
            &dir,
            &listener,
            &ask,
            [script, vec![terminal("tool_calls"), usage()]].concat(),
        );
        presets::stopped(&events, "tool_use");
        assert_eq!(
            assembled(&events),
            vec![("a".into(), A.into()), ("b".into(), B.into())]
        );
    }
}

#[test]
fn interrupted_or_invalid_native_calls_have_nonexecuting_stops_and_keep_their_charge() {
    for (reason, expected) in [
        ("repetition_truncation", "length"),
        ("length", "length"),
        ("content_filter", "error"),
        ("unknown", "error"),
        ("conflict", "error"),
        ("malformed", "error"),
    ] {
        let (dir, listener, ask) = fixture();
        let mut script = vec![chunk(json!([call(0, "a", A), call(1, "b", B)]))];
        if reason == "conflict" {
            script.push(chunk(json!([{"index":0,"id":"changed"}])));
        }
        if reason == "malformed" {
            script.push(chunk(json!([{"index":1,"function":{"arguments":"{"}}])));
        }
        script.push(terminal(if matches!(reason, "conflict" | "malformed") {
            "tool_calls"
        } else {
            reason
        }));
        script.push(usage());
        let (events, _) = invoke(&dir, &listener, &ask, script);
        presets::stopped(&events, expected);
        let spent = &events
            .iter()
            .find(|e| e["event"] == "spent")
            .expect("usage")["usage"];
        assert_eq!(
            (
                spent["input"].as_u64(),
                spent["cache_read"].as_u64(),
                spent["output"].as_u64()
            ),
            (Some(60), Some(30), Some(12))
        );
    }
}

#[test]
fn native_retry_drops_abandoned_and_buffered_calls_without_mutating_the_ask() {
    let (dir, listener, ask) = fixture();
    let fixture = std::thread::spawn(move || {
        let first = response_server(
            listener.try_clone().expect("clone"),
            200,
            stream(
                vec![
                    chunk(json!([
                        call(0, "abandoned", "{"),
                        call(1, "buffered", "{}")
                    ])),
                    usage(),
                ],
                false,
            ),
        )
        .join()
        .expect("first");
        let second = response_server_with_timeout(
            listener,
            200,
            stream(completed(), true),
            std::time::Duration::from_secs(15),
        )
        .join()
        .expect("retry");
        (first, second)
    });
    let events = presets::events(&ask_request_with_timeout(
        &dir,
        &[("FIXTURE_KEY", "private-sentinel")],
        &ask,
        std::time::Duration::from_secs(30),
    ));
    assert_eq!(
        events.iter().filter(|e| e["event"] == "retrying").count(),
        1
    );
    assert!(!events.iter().any(|e| e["event"] == "failed"));
    assert!(
        !events
            .iter()
            .any(|e| e["event"] == "tool_call_start" && e["id"] == "buffered")
    );
    assert_eq!(
        assembled(&events),
        vec![("a".into(), A.into()), ("b".into(), B.into())]
    );
    let (first, second) = fixture.join().expect("fixture");
    assert_eq!(first.1, second.1);
    let usage = &events
        .iter()
        .rfind(|e| e["event"] == "spent")
        .expect("usage")["usage"];
    assert_eq!(usage["output"], 24, "both attempts remain charged");
}
