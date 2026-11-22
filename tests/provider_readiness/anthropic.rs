use super::*;
use melchior::mind::model::{Content, Context, Message, Role, ThinkingLevel, Tool};
use melchior::mind::wire::{Ask, Schema, Wants};

#[path = "anthropic/binding.rs"]
mod binding;

fn declaration(url: &str, auth: Value) -> Value {
    json!({"name":"Fixture","api":"anthropic-messages","base_url":url,"auth":auth,
        "compat":{"anthropic_thinking":"adaptive_effort","anthropic_replay":true,"allow_empty_signature":true,
        "supports_forced_tool_choice":false},"discovery":{"kind":"bundled"},"models":[{
        "id":"m","name":"Model","context_window":1000000,"max_tokens":524288,"reasoning":true,
        "thinking":{"off":null,"minimal":null,"low":"low","medium":"medium","high":"high","xhigh":"xhigh","max":"max"},
        "cost":{"input":0.15,"output":0.6,"cache_read":0.015},"input":["text","image"]}]})
}

fn fixture(dir: &Scratch, url: &str, auth: Value) {
    write_provider(dir, &declaration(url, auth));
}

fn write_provider(dir: &Scratch, provider: &Value) {
    let encoded = serde_json::to_string(&provider.to_string()).expect("Lua string");
    std::fs::write(
        dir.join("providers.lua"),
        format!(
            r#"
        for _, id in ipairs({{"openrouter", "ollama"}}) do
            melchior.provider(id, {{name=id,api="openai-completions",auth={{kind="none"}}}})
        end
        melchior.provider("fixture", melchior.json.decode_value({encoded}))
    "#
        ),
    )
    .expect("fixture");
}

fn listener() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let url = format!(
        "http://{}/anthropic",
        listener.local_addr().expect("address")
    );
    (listener, url)
}

fn request() -> Ask {
    Ask {
        model: "fixture/m".into(),
        about: String::new(),
        wants: Wants {
            thinking: Some(ThinkingLevel::XHigh),
            max_tokens: Some(4096),
            ..Default::default()
        },
        context: Context {
            messages: vec![Message::user("Use echo")],
            tools: vec![Tool {
                name: "echo".into(),
                description: "Echo data".into(),
                parameters: json!({"type":"object"}),
                strict: None,
            }],
            ..Default::default()
        },
    }
}

fn events(output: &std::process::Output) -> Vec<Value> {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for bytes in [&output.stdout, &output.stderr] {
        assert!(!String::from_utf8_lossy(bytes).contains("private-sentinel"));
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).expect("event"))
        .collect()
}

fn sse(name: &str, value: Value) -> String {
    format!("event: {name}\ndata: {value}\n\n")
}

pub(super) fn stream(blocks: &Value, reason: &str) -> String {
    let mut body = sse(
        "message_start",
        json!({"message":{"role":"assistant","content":[],
        "usage":{"input_tokens":60,"cache_read_input_tokens":30,"output_tokens":0}}}),
    );
    for (index, block) in blocks.as_array().expect("blocks").iter().enumerate() {
        body.push_str(&sse(
            "content_block_start",
            json!({"index":index,"content_block":block}),
        ));
        body.push_str(&sse("content_block_stop", json!({"index":index})));
    }
    body.push_str(&sse(
        "message_delta",
        json!({"delta":{"stop_reason":reason},"usage":{"output_tokens":12}}),
    ));
    body.push_str(&sse("message_stop", json!({})));
    body
}

pub(super) fn assistant(events: &[Value]) -> Message {
    let text = |kind: &str| {
        events
            .iter()
            .filter(|e| e["event"] == kind)
            .map(|e| e["text"].as_str().expect("visible text"))
            .collect::<String>()
    };
    let signature = events
        .iter()
        .find(|e| e["event"] == "signature")
        .expect("state")["signature"]
        .as_str()
        .expect("state");
    let call = events
        .iter()
        .find(|e| e["event"] == "tool_call_start")
        .expect("call");
    let args: String = events
        .iter()
        .filter(|e| e["event"] == "tool_call_args")
        .map(|e| e["args"].as_str().expect("args"))
        .collect();
    let mut message = Message::assistant("");
    message.content = vec![
        Content::Thinking {
            thinking: text("thinking"),
            signature: Some(signature.into()),
        },
        Content::Text {
            text: text("text"),
            signature: None,
        },
        Content::ToolCall {
            id: call["id"].as_str().expect("id").into(),
            name: call["name"].as_str().expect("name").into(),
            arguments: serde_json::from_str(&args).expect("args"),
            thought_signature: None,
        },
    ];
    message
}

#[test]
fn full_content_replays_through_native_credentials_and_key_changes_discard_only_opaque_state() {
    let dir = Scratch::new("melchior-anthropic", "bound-replay");
    let (listener, url) = listener();
    fixture(&dir, &url, json!({"kind":"api-key","vars":["FIXTURE_KEY"]}));
    let blocks = json!([{"type":"thinking","thinking":"first "}, {"type":"text","text":"before "},
        {"type":"thinking","thinking":"second","signature":""},
        {"type":"tool_use","id":"echo-1","name":"echo","input":{"n":18446744073709551615u64,"nested":[null,[],{}]}},
        {"type":"text","text":"after"}]);
    let mut ask = request();
    let server = response_server(
        listener.try_clone().expect("listener"),
        200,
        stream(&blocks, "tool_use"),
    );
    let first = events(&ask_request(
        &dir,
        &[("FIXTURE_KEY", "private-sentinel")],
        &ask,
    ));
    let (headers, body) = server.join().expect("server");
    assert!(headers.starts_with("post /anthropic/v1/messages "));
    assert!(headers.contains("x-api-key: private-sentinel\r\n"));
    assert_eq!(body["thinking"], json!({"type":"adaptive"}));
    assert_eq!(body["output_config"], json!({"effort":"xhigh"}));
    assert_eq!(body["max_tokens"], 4096);
    assert!(body.get("__scratch").is_none() && body.get("__budget").is_none());
    assert_eq!(
        first.iter().filter(|e| e["event"] == "signature").count(),
        1,
        "{first:?}"
    );
    assert_eq!(
        first.iter().filter(|e| e["event"] == "stop").count(),
        1,
        "{first:?}"
    );
    ask.context.messages.push(assistant(&first));
    ask.context.messages.push(Message {
        role: Role::Tool,
        content: vec![Content::ToolResult {
            id: "echo-1".into(),
            name: "echo".into(),
            content: "ok".into(),
            images: vec![],
            is_error: false,
        }],
        stop_reason: None,
        usage: None,
        error: None,
    });
    let expected = ask.context.clone();
    for (key, same) in [
        ("private-sentinel", true),
        ("private-sentinel-other", false),
    ] {
        let server = response_server(
            listener.try_clone().expect("listener"),
            200,
            stream(&json!([{"type":"text","text":"ok"}]), "end_turn"),
        );
        let output = events(&ask_request(&dir, &[("FIXTURE_KEY", key)], &ask));
        let (_, body) = server.join().expect("server");
        if same {
            assert_eq!(body["messages"][1]["content"], blocks);
        } else {
            assert!(
                body["messages"][1]["content"]
                    .as_array()
                    .expect("visible fallback")
                    .iter()
                    .all(|b| b["type"] != "thinking")
            );
        }
        assert_eq!(body["messages"][2]["content"][0]["tool_use_id"], "echo-1");
        assert_eq!(ask.context, expected);
        assert_eq!(
            output.iter().filter(|e| e["event"] == "stop").count(),
            1,
            "{output:?}"
        );
        assert!(!body.to_string().contains("melchior.anthropic.v1:"));
    }
}

#[test]
fn automatic_only_schema_refusal_precedes_even_expired_oauth_refresh() {
    let dir = Scratch::new("melchior-anthropic", "forced-schema");
    let (listener, url) = listener();
    fixture(
        &dir,
        &url,
        json!({"kind":"oauth","service":"Fixture","authorize_url":format!("{url}/authorize"),
        "token_url":format!("{url}/token"),"client_id":"public"}),
    );
    tokens(
        &dir,
        "private-sentinel",
        Some("private-sentinel-refresh"),
        1,
    );
    let path = dir.join("data/magi/credentials.json");
    let before = std::fs::read(&path).expect("credentials");
    let mut ask = request();
    ask.wants.schema = Some(Schema {
        name: "answer".into(),
        schema: json!({"type":"object"}),
    });
    let output = events(&ask_request(&dir, &[], &ask));
    assert_eq!(output.len(), 1, "{output:?}");
    assert_eq!(output[0]["event"], "failed");
    assert_eq!(output[0]["why"], "invalid");
    assert!(
        output[0]["message"]
            .as_str()
            .expect("reason")
            .contains("forced tool selection")
    );
    assert_eq!(
        listener
            .accept()
            .expect_err("no refresh or inference")
            .kind(),
        std::io::ErrorKind::WouldBlock
    );
    assert_eq!(std::fs::read(&path).expect("credentials"), before);
}
