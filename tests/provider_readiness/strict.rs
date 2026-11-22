use super::*;
use melchior::mind::model::{Context, Message, Tool, ToolStrictness};
use melchior::mind::wire::Ask;

fn request(strict: ToolStrictness, parameters: Value) -> Ask {
    Ask {
        model: "fixture/m".into(),
        context: Context {
            messages: vec![Message::user("lookup")],
            tools: vec![Tool {
                name: "lookup".into(),
                description: "Lookup".into(),
                parameters,
                strict: Some(strict),
            }],
            ..Default::default()
        },
        wants: Default::default(),
        about: String::new(),
    }
}

fn capabilities(dir: &Scratch, url: &str, auth: &str, supported: bool) {
    config(dir, url, auth, "openai-completions");
    std::fs::OpenOptions::new()
        .append(true)
        .open(dir.join("providers.lua"))
        .expect("config")
        .write_all(
            format!(
                r#"
        melchior.provider("fixture", {{name="Fixture",api="openai-completions",base_url={url:?},
            auth={auth}, compat={{supports_strict_mode={supported}}},
            models={{{{id="m",name="M",context_window=8192,max_tokens=128}}}}}})
        "#
            )
            .as_bytes(),
        )
        .expect("strict config");
}

fn events(output: &std::process::Output) -> Vec<Value> {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).expect("event"))
        .collect()
}

#[test]
fn strict_fallback_feedback_can_be_reused_by_the_next_process_without_downgrading_require() {
    let dir = Scratch::new("melchior-strict", "fallback-feedback");
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let url = format!("http://{}", listener.local_addr().expect("address"));
    config(&dir, &url, r#"{kind="none"}"#, "anthropic-messages");
    std::fs::OpenOptions::new()
        .append(true)
        .open(dir.join("providers.lua"))
        .expect("config")
        .write_all(
            format!(
                r#"
        melchior.provider("fixture", {{name="Fixture",api="anthropic-messages",base_url={url:?},
            auth={{kind="none"}},compat={{supports_strict_tools=true}},
            models={{{{id="m",name="M",context_window=8192,max_tokens=128}}}}}})
        "#
            )
            .as_bytes(),
        )
        .expect("strict provider");
    let server = fallback_server(listener);
    let mut ask = request(
        ToolStrictness::Prefer,
        json!({"type":"object","properties":{"term":{"type":"string"}}}),
    );
    let mut required = ask.context.tools[0].clone();
    required.name = "required".into();
    required.strict = Some(ToolStrictness::Require);
    ask.context.tools.push(required);
    let first = events(&ask_request(&dir, &[], &ask));
    assert!(first.iter().any(|event| event["event"] == "stop"));
    assert!(!first.iter().any(|event| event["event"] == "failed"));
    assert_eq!(
        first
            .iter()
            .filter(|event| event["event"] == "retrying")
            .count(),
        1
    );
    let frames: Vec<melchior::mind::model::RequestFrame> = first
        .iter()
        .filter(|event| event["event"] == "request_frame")
        .map(|event| serde_json::from_value(event["frame"].clone()).expect("frame"))
        .collect();
    assert_eq!(frames.len(), 2);
    assert!(!frames[0].disable_preferred_strict);
    assert!(frames[1].disable_preferred_strict);
    assert_ne!(frames[0].policy, frames[1].policy);
    ask.wants.disable_preferred_strict = frames[1].disable_preferred_strict;
    let mut bytes = Vec::new();
    ciborium::into_writer(&ask, &mut bytes).expect("CBOR ask");
    let ask: Ask = ciborium::from_reader(bytes.as_slice()).expect("CBOR options");
    assert!(ask.wants.disable_preferred_strict);
    bytes.clear();
    ciborium::into_writer(&frames[1], &mut bytes).expect("CBOR frame");
    assert_eq!(
        ciborium::from_reader::<melchior::mind::model::RequestFrame, _>(bytes.as_slice())
            .expect("CBOR frame"),
        frames[1]
    );
    let second = events(&ask_request(&dir, &[], &ask));
    assert!(second.iter().any(|event| event["event"] == "stop"));
    assert!(
        !second
            .iter()
            .any(|event| matches!(event["event"].as_str(), Some("failed" | "retrying")))
    );
    let next = second
        .iter()
        .find(|event| event["event"] == "request_frame")
        .expect("next frame");
    assert_eq!(
        next["frame"],
        serde_json::to_value(frames[1]).expect("frame")
    );
    let bodies = server.join().expect("server");
    assert_eq!(bodies[0]["tools"][0]["strict"], true);
    for body in &bodies[1..] {
        assert!(body["tools"][0].get("strict").is_none());
        assert_eq!(
            body["tools"][0]["input_schema"],
            ask.context.tools[0].parameters
        );
        assert_eq!(body["tools"][1], bodies[0]["tools"][1]);
        assert_eq!(body["tools"][1]["strict"], true);
    }
}

fn fallback_server(listener: TcpListener) -> std::thread::JoinHandle<Vec<Value>> {
    std::thread::spawn(move || {
        use std::time::{Duration, Instant};
        listener.set_nonblocking(true).expect("nonblocking");
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut bodies = Vec::new();
        for attempt in 0..3 {
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "missing attempt {attempt}");
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => panic!("accept: {error}"),
                }
            };
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("read deadline");
            socket
                .set_write_timeout(Some(Duration::from_secs(2)))
                .expect("write deadline");
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") {
                assert!(header.len() < 65536);
                let mut byte = [0];
                socket.read_exact(&mut byte).expect("header");
                header.push(byte[0]);
            }
            let header = String::from_utf8(header)
                .expect("header")
                .to_ascii_lowercase();
            let length: usize = header
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .expect("length")
                .trim()
                .parse()
                .expect("size");
            assert!(length < 65536);
            let mut body = vec![0; length];
            socket.read_exact(&mut body).expect("body");
            bodies.push(serde_json::from_slice(&body).expect("JSON body"));
            let (status, body) = if attempt == 0 {
                (
                    400,
                    r#"{"type":"error","error":{"type":"invalid_request_error","message":"Too many strict tools (21)"}}"#,
                )
            } else {
                (
                    200,
                    "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
                )
            };
            write!(socket, "HTTP/1.1 {status} Fixture\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).expect("reply");
        }
        bodies
    })
}

#[test]
fn strict_policy_survives_roles_and_reaches_the_actual_http_request() {
    for (supported, policy) in [
        (true, ToolStrictness::Require),
        (false, ToolStrictness::Prefer),
    ] {
        let dir = Scratch::new("melchior-strict", "request");
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let url = format!("http://{}", listener.local_addr().expect("address"));
        capabilities(&dir, &url, r#"{kind="none"}"#, supported);
        let card = cards(&dir, &[]);
        assert_eq!(card["capabilities"]["strict_tools"], supported);
        let output = command(&dir)
            .args(["models", "--cbor"])
            .output()
            .expect("CBOR models");
        assert!(output.status.success());
        let reply: Value = ciborium::from_reader(output.stdout.as_slice()).expect("CBOR reply");
        assert!(reply["result"].as_array().expect("cards").contains(&card));
        let parameters = json!({"type":"object","properties":{"term":{"type":"string"}}});
        let ask = request(policy, parameters.clone());
        let mut bytes = Vec::new();
        ciborium::into_writer(&ask, &mut bytes).expect("CBOR ask");
        let decoded: Ask = ciborium::from_reader(bytes.as_slice()).expect("CBOR tool policy");
        assert_eq!(decoded.context, ask.context);
        let server = server(listener);
        let output = ask_request(&dir, &[], &ask);
        let (_, body) = server.join().expect("server");
        let tool = &body["tools"][0]["function"];
        if supported {
            assert_eq!(tool["strict"], true);
            assert_eq!(tool["parameters"]["required"], json!(["term"]));
            assert_eq!(tool["parameters"]["additionalProperties"], false);
        } else {
            assert!(tool.get("strict").is_none());
            assert_eq!(tool["parameters"], parameters);
        }
        assert!(output.status.success());
        let events = events(&output);
        assert!(events.iter().any(|event| event["event"] == "stop"));
        assert!(!events.iter().any(|event| event["event"] == "failed"));
        assert!(!body.to_string().contains("__strict"));
    }
}

#[test]
fn required_strict_refusal_precedes_oauth_refresh_network_and_retry() {
    for supported in [false, true] {
        let dir = Scratch::new("melchior-strict", "refusal");
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        listener.set_nonblocking(true).expect("nonblocking");
        let url = format!("http://{}", listener.local_addr().expect("address"));
        let auth = format!(
            r#"{{kind="oauth",service="Fixture",authorize_url="{url}/authorize",token_url="{url}/token",client_id="public"}}"#
        );
        capabilities(&dir, &url, &auth, supported);
        tokens(
            &dir,
            "private-sentinel",
            Some("private-sentinel-refresh"),
            1,
        );
        let before = std::fs::read(dir.join("data/magi/credentials.json")).expect("store");
        let parameters = if supported {
            json!({"type":"object","$ref":"private-sentinel"})
        } else {
            json!({"type":"object"})
        };
        let output = ask_request(&dir, &[], &request(ToolStrictness::Require, parameters));
        let events = events(&output);
        let failed: Vec<_> = events
            .iter()
            .filter(|event| event["event"] == "failed")
            .collect();
        assert_eq!(failed.len(), 1, "{events:?}");
        assert!(
            failed[0]["message"]
                .as_str()
                .expect("message")
                .contains("requires strict arguments")
        );
        assert_eq!(failed[0]["why"], "invalid");
        assert!(!events.iter().any(|event| event["event"] == "retrying"));
        assert_eq!(
            listener
                .accept()
                .expect_err("no refresh or inference")
                .kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert_eq!(
            std::fs::read(dir.join("data/magi/credentials.json")).expect("store"),
            before
        );
    }
}
