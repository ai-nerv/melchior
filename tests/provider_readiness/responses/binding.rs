use super::*;

const APIS: [&str; 3] = [
    "openai-responses",
    "azure-openai-responses",
    "openai-codex-responses",
];

fn output() -> Value {
    json!([
        {"type":"reasoning","id":"rs_hidden","summary":[],"encrypted_content":"opaque account state"},
        {"type":"function_call","id":"fc_first","call_id":"call_first","name":"echo",
            "arguments":"{\"a\":[null,[],{}]}"}
    ])
}

fn initial() -> Ask {
    Ask {
        model: "fixture/m".into(),
        about: String::new(),
        wants: Wants::default(),
        context: Context {
            messages: vec![Message::user("Use echo")],
            tools: vec![Tool {
                name: "echo".into(),
                description: "Echo".into(),
                parameters: json!({"type":"object","properties":{"a":{"type":"array"}}}),
                strict: None,
            }],
            ..Default::default()
        },
    }
}

fn history(first: &[Value]) -> Ask {
    let signature = first
        .iter()
        .find(|event| event["event"] == "signature")
        .expect("signature")["signature"]
        .as_str()
        .expect("string")
        .to_owned();
    let mut ask = initial();
    ask.context.messages.extend([
        Message {
            role: Role::Assistant,
            content: vec![
                Content::Thinking {
                    thinking: String::new(),
                    signature: Some(signature),
                },
                Content::ToolCall {
                    id: "call_first".into(),
                    name: "echo".into(),
                    arguments: json!({"a":[null,[],{}]}),
                    thought_signature: None,
                },
            ],
            stop_reason: None,
            usage: None,
            error: None,
        },
        Message {
            role: Role::Tool,
            content: vec![Content::ToolResult {
                id: "call_first".into(),
                name: "echo".into(),
                content: "result".into(),
                images: vec![],
                is_error: false,
            }],
            stop_reason: None,
            usage: None,
            error: None,
        },
    ]);
    ask
}

fn round(
    dir: &Scratch,
    listener: &TcpListener,
    vars: &[(&str, &str)],
    ask: &Ask,
) -> (Vec<Value>, Value) {
    let worker = response_server(
        listener.try_clone().expect("listener"),
        200,
        stream(&output()),
    );
    let result = events(&ask_request(dir, vars, ask));
    assert!(
        result
            .iter()
            .any(|value| value["event"] == "stop" && value["reason"] == "tool_use")
    );
    let (_, sent) = worker.join().expect("request");
    (result, sent)
}

fn replay(sent: &Value, expected: bool) {
    if expected {
        assert_eq!(sent["input"][1], output()[0]);
        assert_eq!(sent["input"][2], output()[1]);
    } else {
        assert_eq!(sent["input"].as_array().expect("input").len(), 3);
        assert_eq!(
            sent["input"][1],
            json!({"type":"function_call","call_id":"call_first",
            "name":"echo","arguments":"{\"a\":[null,[],{}]}"})
        );
        assert!(!sent.to_string().contains("opaque account state"));
        assert!(!sent.to_string().contains("fc_first"));
    }
    assert!(sent.to_string().contains("function_call_output"));
    assert!(sent.get("__scratch").is_none());
    assert!(!sent.to_string().contains("private-sentinel"));
}

#[test]
fn changed_api_keys_headers_and_legacy_state_cannot_replay_opaque_history() {
    for api in APIS {
        let dir = Scratch::new("melchior-ready", "responses-binding");
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let url = format!("http://{}/v1", listener.local_addr().expect("address"));
        let auth = r#"{kind="api-key",vars={"FIXTURE_KEY"}}"#;
        config(&dir, &url, auth, api);
        let a = [("FIXTURE_KEY", "private-sentinel-a")];
        let b = [("FIXTURE_KEY", "private-sentinel-b")];
        let (first, _) = round(&dir, &listener, &a, &initial());
        let mut ask = history(&first);
        let before = ask.context.clone();
        replay(&round(&dir, &listener, &a, &ask).1, true);
        replay(&round(&dir, &listener, &b, &ask).1, false);
        let path = dir.join("providers.lua");
        let declaration = std::fs::read_to_string(&path).expect("config");
        std::fs::write(
            &path,
            declaration.replace(
                "name=\"Fixture\",",
                "name=\"Fixture\",headers={[\"OpenAI-Organization\"]=\"different\"},",
            ),
        )
        .expect("other organization");
        replay(&round(&dir, &listener, &a, &ask).1, false);
        assert_eq!(ask.context, before);
        std::fs::write(path, declaration).expect("original config");
        let Content::Thinking {
            signature: Some(signature),
            ..
        } = &mut ask.context.messages[1].content[0]
        else {
            panic!("signature");
        };
        let mut saved: Value =
            serde_json::from_str(signature.trim_start_matches("melchior.responses.v1:"))
                .expect("envelope");
        saved["origin"]
            .as_object_mut()
            .expect("origin")
            .remove("credential");
        *signature = format!("melchior.responses.v1:{saved}");
        replay(&round(&dir, &listener, &a, &ask).1, false);
    }
}

fn token_server(
    listener: TcpListener,
    access: &str,
    refresh: Option<&str>,
) -> std::thread::JoinHandle<String> {
    let body = json!({"access_token":access,"refresh_token":refresh,"expires_in":3600}).to_string();
    std::thread::spawn(move || {
        listener.set_nonblocking(true).expect("nonblocking");
        let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut socket = loop {
            match listener.accept() {
                Ok((socket, _)) => break socket,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(std::time::Instant::now() < until, "no refresh request");
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Err(error) => panic!("accept: {error}"),
            }
        };
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .expect("deadline");
        let mut header = Vec::new();
        while !header.ends_with(b"\r\n\r\n") {
            assert!(header.len() < 8192);
            let mut byte = [0];
            socket.read_exact(&mut byte).expect("header");
            header.push(byte[0]);
        }
        let header = String::from_utf8(header)
            .expect("header")
            .to_ascii_lowercase();
        assert!(header.starts_with("post /token "));
        let length: usize = header
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .expect("length")
            .trim()
            .parse()
            .expect("number");
        assert!(length < 8192);
        let mut request = vec![0; length];
        socket.read_exact(&mut request).expect("form");
        write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).expect("response");
        String::from_utf8(request).expect("form")
    })
}

fn oauth_config(dir: &Scratch, inference: &TcpListener, token: &TcpListener, api: &str) {
    let url = format!("http://{}/v1", inference.local_addr().expect("inference"));
    let token = token.local_addr().expect("token");
    config(
        dir,
        &url,
        &format!(
            r#"{{kind="oauth",service="Fixture",token_url="http://{token}/token",client_id="public"}}"#
        ),
        api,
    );
    tokens(
        dir,
        "private-sentinel-initial",
        Some("private-sentinel-refresh-a"),
        u64::MAX,
    );
}

fn expire(dir: &Scratch) {
    let path = dir.join("data/magi/credentials.json");
    let mut store = Store::load_from(&path).expect("load");
    store
        .providers
        .get_mut("fixture")
        .expect("tokens")
        .expires_at = 1;
    store.save_to(&path).expect("expire");
}

#[test]
fn oauth_refresh_preserves_legacy_lineage_but_relogin_and_logout_do_not() {
    for api in APIS {
        let dir = Scratch::new("melchior-ready", "responses-oauth-binding");
        let inference = TcpListener::bind("127.0.0.1:0").expect("inference");
        let token = TcpListener::bind("127.0.0.1:0").expect("token");
        oauth_config(&dir, &inference, &token, api);
        let path = dir.join("data/magi/credentials.json");
        let first = round(&dir, &inference, &[], &initial()).0;
        let ask = history(&first);
        let before = ask.context.clone();
        let mut binding = None;
        for (access, expected_refresh, next_refresh) in [
            (
                "private-sentinel-renewed-a",
                "private-sentinel-refresh-a",
                Some("private-sentinel-refresh-b"),
            ),
            (
                "private-sentinel-renewed-b",
                "private-sentinel-refresh-b",
                None,
            ),
        ] {
            expire(&dir);
            let worker = token_server(token.try_clone().expect("listener"), access, next_refresh);
            replay(&round(&dir, &inference, &[], &ask).1, true);
            let form = worker.join().expect("refresh");
            assert!(form.contains(&format!("refresh_token={expected_refresh}")));
            let store = Store::load_from(&path).expect("renewed store");
            let tokens = store.get("fixture").expect("tokens");
            assert_eq!(tokens.access, access);
            assert_eq!(
                tokens.refresh.as_deref(),
                Some("private-sentinel-refresh-b")
            );
            assert_eq!(tokens.binding.as_ref().expect("promoted lineage").len(), 64);
            if let Some(binding) = &binding {
                assert_eq!(&tokens.binding, binding);
            }
            binding = Some(tokens.binding.clone());
        }
        tokens(&dir, "private-sentinel-another-account", None, u64::MAX);
        replay(&round(&dir, &inference, &[], &ask).1, false);
        let mut store = Store::load_from(&path).expect("store");
        assert!(store.forget("fixture"));
        store.save_to(&path).expect("logout");
        let result = events(&ask_request(&dir, &[], &ask));
        assert!(
            result
                .iter()
                .any(|event| event["event"] == "failed" && event["why"] == "auth")
        );
        assert_eq!(
            inference
                .accept()
                .expect_err("no request after logout")
                .kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert_eq!(ask.context, before);
    }
}

#[test]
fn oversized_bound_history_fails_before_oauth_refresh_or_inference() {
    for api in APIS {
        let dir = Scratch::new("melchior-ready", "responses-binding-limit");
        let inference = TcpListener::bind("127.0.0.1:0").expect("inference");
        let token = TcpListener::bind("127.0.0.1:0").expect("token");
        token.set_nonblocking(true).expect("nonblocking");
        oauth_config(&dir, &inference, &token, api);
        let mut ask = history(&round(&dir, &inference, &[], &initial()).0);
        let Content::Thinking {
            signature: Some(signature),
            ..
        } = &mut ask.context.messages[1].content[0]
        else {
            panic!("signature");
        };
        let mut saved: Value =
            serde_json::from_str(signature.trim_start_matches("melchior.responses.v1:"))
                .expect("state");
        saved["items"][0]["encrypted_content"] = "x".repeat(4096).into();
        *signature = format!("melchior.responses.v1:{saved}");
        let path = dir.join("providers.lua");
        let config = std::fs::read_to_string(&path).expect("config");
        std::fs::write(
            path,
            config.replace(
                "context_window=8192",
                "input_limits={max_request_bytes=3000},context_window=8192",
            ),
        )
        .expect("limit");
        expire(&dir);
        let path = dir.join("data/magi/credentials.json");
        let before = std::fs::read(&path).expect("store");
        let result = events(&ask_request(&dir, &[], &ask));
        assert!(
            result
                .iter()
                .any(|event| event["event"] == "failed" && event["why"] == "invalid"),
            "{result:?}"
        );
        assert!(!result.iter().any(|event| event["event"] == "retrying"));
        for listener in [inference, token] {
            assert_eq!(
                listener
                    .accept()
                    .expect_err("preflight without network")
                    .kind(),
                std::io::ErrorKind::WouldBlock
            );
        }
        assert_eq!(std::fs::read(path).expect("store"), before);
    }
}

#[test]
fn retry_rebinds_to_credentials_replaced_before_the_refusal_is_delivered() {
    for api in APIS {
        let dir = Scratch::new("melchior-ready", "responses-retry-binding");
        let inference = TcpListener::bind("127.0.0.1:0").expect("inference");
        let token = TcpListener::bind("127.0.0.1:0").expect("token");
        oauth_config(&dir, &inference, &token, api);
        let first = round(&dir, &inference, &[], &initial()).0;
        let ask = history(&first);
        let mut replacement = Store::default();
        replacement.put(
            "fixture",
            Tokens {
                access: "private-sentinel-replacement".into(),
                refresh: None,
                expires_at: u64::MAX,
                binding: None,
            },
        );
        let path = dir.join("data/magi/credentials.json");
        let worker = std::thread::spawn(move || {
            let refused = response_server_observing(
                inference.try_clone().expect("listener"),
                503,
                r#"{"error":{"message":"temporary fixture overload"}}"#.into(),
                std::time::Duration::from_secs(5),
                move || replacement.save_to(&path).expect("replace credential"),
            )
            .join()
            .expect("refused attempt");
            let retried = response_server_with_timeout(
                inference,
                200,
                stream(&output()),
                std::time::Duration::from_secs(10),
            )
            .join()
            .expect("retried attempt");
            (refused, retried)
        });
        let result = events(&ask_request_with_timeout(
            &dir,
            &[],
            &ask,
            std::time::Duration::from_secs(15),
        ));
        let (refused, retried) = worker.join().expect("server");
        replay(&refused.1, true);
        replay(&retried.1, false);
        assert!(refused.0.contains("private-sentinel-initial"));
        assert!(!refused.0.contains("private-sentinel-replacement"));
        assert!(retried.0.contains("private-sentinel-replacement"));
        assert!(!retried.0.contains("private-sentinel-initial"));
        assert_eq!(
            result
                .iter()
                .filter(|event| event["event"] == "retrying")
                .count(),
            1
        );
        assert!(
            result
                .iter()
                .any(|event| event["event"] == "stop" && event["reason"] == "tool_use")
        );
    }
}
