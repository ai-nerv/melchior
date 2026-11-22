use super::*;

fn round(
    dir: &Scratch,
    listener: &TcpListener,
    ask: &Ask,
    vars: &[(&str, &str)],
) -> (Vec<Value>, Value) {
    let blocks = json!([
        {"type":"thinking","thinking":"reason","signature":"", "vendor_data":"x".repeat(4096)},
        {"type":"text","text":"use echo"},
        {"type":"tool_use","id":"echo-1","name":"echo","input":{}}
    ]);
    let server = response_server(
        listener.try_clone().expect("listener"),
        200,
        stream(&blocks, "tool_use"),
    );
    let said = events(&ask_request(dir, vars, ask));
    let (_, body) = server.join().expect("request");
    assert!(
        said.iter()
            .any(|e| e["event"] == "stop" && e["reason"] == "tool_use"),
        "{said:?}"
    );
    (said, body)
}

#[test]
fn bound_request_limit_refuses_before_expired_oauth_refresh_or_inference() {
    let dir = Scratch::new("melchior-anthropic", "bound-limit");
    let (inference, url) = listener();
    let (token, token_url) = listener();
    let mut provider = declaration(
        &url,
        json!({"kind":"oauth","service":"Fixture",
        "token_url":format!("{token_url}/token"),"client_id":"public"}),
    );
    write_provider(&dir, &provider);
    tokens(
        &dir,
        "private-sentinel",
        Some("private-sentinel-refresh"),
        u64::MAX,
    );
    let mut ask = request();
    ask.context
        .messages
        .push(assistant(&round(&dir, &inference, &ask, &[]).0));
    let (_, bound) = round(&dir, &inference, &ask, &[]);
    assert!(serde_json::to_vec(&bound).expect("body").len() > 3000);
    let mut visible = ask.clone();
    for content in &mut visible.context.messages[1].content {
        if let Content::Thinking { signature, .. } = content {
            *signature = None;
        }
    }
    let (_, fallback) = round(&dir, &inference, &visible, &[]);
    assert!(serde_json::to_vec(&fallback).expect("body").len() < 3000);
    provider["models"][0]["input_limits"] = json!({"max_request_bytes":3000});
    write_provider(&dir, &provider);
    let path = dir.join("data/magi/credentials.json");
    let mut store = Store::load_from(&path).expect("store");
    store
        .providers
        .get_mut("fixture")
        .expect("token")
        .expires_at = 1;
    store.save_to(&path).expect("expire");
    let before = std::fs::read(&path).expect("store");
    let said = events(&ask_request(&dir, &[], &ask));
    assert_eq!(said.len(), 1, "{said:?}");
    assert_eq!(said[0]["event"], "failed");
    assert_eq!(said[0]["why"], "invalid");
    assert!(
        said[0]["message"]
            .as_str()
            .expect("reason")
            .contains("byte limit")
    );
    for listener in [&inference, &token] {
        assert_eq!(
            listener.accept().expect_err("no request").kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
    assert_eq!(std::fs::read(&path).expect("store"), before);
}

#[test]
fn model_endpoint_header_and_login_changes_discard_only_bound_blocks() {
    let dir = Scratch::new("melchior-anthropic", "binding");
    let (inference, url) = listener();
    let auth = json!({"kind":"api-key","vars":["FIXTURE_KEY"]});
    let provider = declaration(&url, auth);
    write_provider(&dir, &provider);
    let vars = [("FIXTURE_KEY", "private-sentinel")];
    let mut ask = request();
    ask.context
        .messages
        .push(assistant(&round(&dir, &inference, &ask, &vars).0));
    let before = ask.context.clone();
    for change in ["model", "endpoint", "headers"] {
        let mut changed = provider.clone();
        let mut next = ask.clone();
        match change {
            "model" => {
                changed["models"][0]["id"] = json!("other");
                next.model = "fixture/other".into();
            }
            "endpoint" => changed["base_url"] = json!(format!("{url}/other")),
            _ => changed["headers"] = json!({"x-account-id":"different"}),
        }
        write_provider(&dir, &changed);
        let (_, body) = round(&dir, &inference, &next, &vars);
        let content = body["messages"][1]["content"]
            .as_array()
            .expect("assistant");
        assert!(content.iter().all(|b| b["type"] != "thinking"), "{change}");
        assert!(
            content
                .iter()
                .any(|b| b["type"] == "tool_use" && b["id"] == "echo-1")
        );
        assert!(!body.to_string().contains("vendor_data"));
        assert_eq!(next.context, before);
    }
    let provider = declaration(&url, json!({"kind":"oauth","service":"Fixture"}));
    write_provider(&dir, &provider);
    tokens(&dir, "private-sentinel-a", None, u64::MAX);
    let mut ask = request();
    ask.context
        .messages
        .push(assistant(&round(&dir, &inference, &ask, &[]).0));
    assert!(
        round(&dir, &inference, &ask, &[])
            .1
            .to_string()
            .contains("vendor_data")
    );
    tokens(&dir, "private-sentinel-b", None, u64::MAX);
    assert!(
        !round(&dir, &inference, &ask, &[])
            .1
            .to_string()
            .contains("vendor_data")
    );
}
