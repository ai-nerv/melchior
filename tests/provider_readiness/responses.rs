use super::*;
use melchior::mind::model::{Content, Context, Message, Role, Tool};
use melchior::mind::wire::{Ask, Wants};

#[path = "responses/streaming.rs"]
mod streaming;

#[path = "responses/binding.rs"]
mod binding;

fn events(output: &std::process::Output) -> Vec<Value> {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).expect("wire event"))
        .collect()
}

fn stream(output: &Value) -> String {
    [
        json!({"type":"response.output_item.added","output_index":0,
            "item":{"type":"reasoning","id":"rs_hidden","summary":[]}}),
        json!({"type":"response.output_item.done","output_index":0,
            "item":{"type":"reasoning","id":"rs_hidden","summary":[]}}),
        json!({"type":"response.output_item.added","output_index":1,
            "item":{"type":"function_call","id":"fc_first","call_id":"call_first","name":"echo"}}),
        json!({"type":"response.function_call_arguments.delta","output_index":1,"delta":"{\"a\":"}),
        json!({"type":"response.function_call_arguments.delta","output_index":1,"delta":"[null,[],{}]}"}),
        json!({"type":"response.completed","response":{"output":output,
            "usage":{"input_tokens":90,"output_tokens":12,"input_tokens_details":{"cached_tokens":30}}}}),
    ].into_iter().map(|value| format!("data: {value}\n\n")).collect()
}

#[test]
fn hidden_reasoning_from_terminal_response_survives_native_tool_followup() {
    for api in [
        "openai-responses",
        "azure-openai-responses",
        "openai-codex-responses",
    ] {
        let dir = Scratch::new("melchior-ready", "responses-continuation");
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let url = format!("http://{}/v1", listener.local_addr().expect("address"));
        config(&dir, &url, r#"{kind="api-key",vars={"FIXTURE_KEY"}}"#, api);
        let path = dir.join("providers.lua");
        let declaration = std::fs::read_to_string(&path).expect("config");
        std::fs::write(
            path,
            declaration.replace("name=\"Model\"", "name=\"Model\",reasoning=true"),
        )
        .expect("reasoning");
        let opaque = "encrypted \u{1}α+/=";
        let output = json!([
            {"type":"reasoning","id":"rs_hidden","summary":[],"encrypted_content":opaque},
            {"type":"function_call","id":"fc_first","call_id":"call_first","name":"echo",
                "arguments":"{\"a\":[null,[],{}]}"}
        ]);
        let mut ask = Ask {
            model: "fixture/m".into(),
            about: String::new(),
            wants: Wants::default(),
            context: Context {
                messages: vec![Message::user("Use echo")],
                tools: vec![Tool {
                    name: "echo".into(),
                    description: "Echo data".into(),
                    parameters: json!({"type":"object","properties":{"a":{"type":"array"}}}),
                    strict: None,
                }],
                ..Default::default()
            },
        };
        let worker = response_server(listener.try_clone().expect("clone"), 200, stream(&output));
        let first = events(&ask_request(
            &dir,
            &[("FIXTURE_KEY", "private-sentinel")],
            &ask,
        ));
        assert_eq!(
            first
                .iter()
                .filter(|event| event["event"] == "stop")
                .count(),
            1,
            "{first:?}"
        );
        assert!(!first.iter().any(|event| event["event"] == "thinking"));
        let signature = first
            .iter()
            .find(|event| event["event"] == "signature")
            .expect("encrypted reasoning")["signature"]
            .as_str()
            .expect("signature");
        let (headers, sent) = worker.join().expect("request");
        assert_eq!(sent["store"], false);
        assert_eq!(sent["include"], json!(["reasoning.encrypted_content"]));
        assert!(sent.get("__scratch").is_none());
        assert!(sent.get("__budget").is_none());
        assert!(headers.contains(if api == "azure-openai-responses" {
            "api-key: private-sentinel\r\n"
        } else {
            "authorization: bearer private-sentinel\r\n"
        }));
        let call = first
            .iter()
            .find(|event| event["event"] == "tool_call_start")
            .expect("call");
        let arguments: String = first
            .iter()
            .filter(|event| event["event"] == "tool_call_args")
            .map(|event| event["args"].as_str().expect("arguments"))
            .collect();
        let mut assistant = Message::assistant("");
        assistant.content = vec![
            Content::Thinking {
                thinking: String::new(),
                signature: Some(signature.into()),
            },
            Content::ToolCall {
                id: call["id"].as_str().expect("id").into(),
                name: call["name"].as_str().expect("name").into(),
                arguments: serde_json::from_str(&arguments).expect("JSON arguments"),
                thought_signature: None,
            },
        ];
        ask.context.messages.push(assistant);
        ask.context.messages.push(Message {
            role: Role::Tool,
            content: vec![Content::ToolResult {
                images: vec![],
                id: "call_first".into(),
                name: "echo".into(),
                content: "tool result".into(),
                is_error: false,
            }],
            stop_reason: None,
            usage: None,
            error: None,
        });
        let before = ask.context.clone();
        let worker = response_server(listener, 200,
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"done\"}\n\ndata: {\"type\":\"response.completed\",\"response\":{\"output\":[]}}\n\n".into());
        let second = events(&ask_request(
            &dir,
            &[("FIXTURE_KEY", "private-sentinel")],
            &ask,
        ));
        assert_eq!(
            second
                .iter()
                .filter(|event| event["event"] == "stop")
                .count(),
            1
        );
        let (_, sent) = worker.join().expect("followup");
        assert_eq!(sent["input"][1], output[0]);
        assert_eq!(sent["input"][2], output[1]);
        assert_eq!(
            sent["input"][3],
            json!({"type":"function_call_output","call_id":"call_first","output":"tool result"})
        );
        assert_eq!(ask.context, before);
    }
}
