#![allow(clippy::expect_used, clippy::indexing_slicing)]

use melchior::mind::provider::oauth::{Store, Tokens};
use melchior::scratch::Scratch;
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Command, Stdio};

#[path = "provider_readiness/routing.rs"]
mod routing;

#[path = "provider_readiness/thinking.rs"]
mod thinking;

#[path = "provider_readiness/strict.rs"]
mod strict;

#[path = "provider_readiness/imports.rs"]
mod imports;

#[path = "provider_readiness/presets.rs"]
mod presets;

#[path = "provider_readiness/input_limits.rs"]
mod input_limits;

#[path = "provider_readiness/responses.rs"]
mod responses;

#[path = "provider_readiness/anthropic.rs"]
mod anthropic;

#[path = "provider_readiness/fireworks_foundation.rs"]
mod fireworks_foundation;

#[path = "provider_readiness/qwen_foundation.rs"]
mod qwen_foundation;

#[path = "provider_readiness/together.rs"]
mod together;

#[path = "provider_readiness/huggingface.rs"]
mod huggingface;

#[path = "provider_readiness/ant_ling.rs"]
mod ant_ling;

#[path = "provider_readiness/google_calls.rs"]
mod google_calls;

#[path = "provider_readiness/google_usage.rs"]
mod google_usage;

#[path = "provider_readiness/google_thinking.rs"]
mod google_thinking;

#[path = "provider_readiness/mistral.rs"]
mod mistral;

#[path = "provider_readiness/login.rs"]
mod login;

#[path = "provider_readiness/cancellation.rs"]
mod cancellation;

#[path = "provider_readiness/completions.rs"]
mod completions;

#[path = "provider_readiness/instructions.rs"]
mod instructions;

#[path = "provider_readiness/tool_history.rs"]
mod tool_history;

#[path = "provider_readiness/session.rs"]
mod session;

#[path = "provider_readiness/deepseek.rs"]
mod deepseek;

fn command(dir: &Scratch) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_melchior"));
    command
        .env_clear()
        .env("HOME", dir.as_os_str())
        .env("MELCHIOR_CONFIG", dir.as_os_str())
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env("XDG_CACHE_HOME", dir.join("cache"))
        .env("XDG_DATA_HOME", dir.join("data"))
        .env("XDG_RUNTIME_DIR", dir.join("runtime"));
    command
}

fn config(dir: &Scratch, url: &str, auth: &str, api: &str) {
    std::fs::write(
        dir.join("providers.lua"),
        format!(
            r#"
        for _, id in ipairs({{"openrouter", "ollama"}}) do
            melchior.provider(id, {{name=id,api="openai-completions",auth={{kind="none"}}}})
        end
        melchior.provider("fixture", {{name="Fixture",api={api:?},base_url={url:?},auth={auth},
            models={{{{id="m",name="Model",context_window=8192,max_tokens=128}}}}}})
    "#
        ),
    )
    .expect("provider config");
}

fn cards(dir: &Scratch, vars: &[(&str, &str)]) -> Value {
    let output = command(dir)
        .envs(vars.iter().copied())
        .args(["models", "--json"])
        .output()
        .expect("models");
    assert!(output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("private-sentinel"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("private-sentinel"));
    let value: Value = serde_json::from_slice(&output.stdout).expect("model reply");
    assert_eq!(value["ok"], true, "{value}");
    value["result"]
        .as_array()
        .expect("cards")
        .iter()
        .find(|card| card["provider"] == "fixture")
        .expect("fixture card")
        .clone()
}

fn tokens(dir: &Scratch, access: &str, refresh: Option<&str>, expires_at: u64) {
    let mut store = Store::default();
    store.put(
        "fixture",
        Tokens {
            access: access.into(),
            refresh: refresh.map(str::to_owned),
            expires_at,
            binding: None,
        },
    );
    store
        .save_to(&dir.join("data/magi/credentials.json"))
        .expect("private store");
}

#[test]
fn api_key_aliases_and_whitespace_are_reported_authoritatively() {
    let dir = Scratch::new("melchior-ready", "aliases");
    config(
        &dir,
        "https://fixture.invalid",
        r#"{kind="api-key",vars={"PRIMARY_KEY","ALTERNATE_KEY"}}"#,
        "openai-completions",
    );
    for vars in [vec![], vec![("PRIMARY_KEY", " \t\n")]] {
        let card = cards(&dir, &vars);
        assert_eq!(card["ready"], false);
        assert_eq!(card["setup"]["state"], "credential_missing");
        assert_eq!(
            card["setup"]["variables"],
            json!([["PRIMARY_KEY", "ALTERNATE_KEY"]])
        );
    }
    let card = cards(
        &dir,
        &[("PRIMARY_KEY", " "), ("ALTERNATE_KEY", "private-sentinel")],
    );
    assert_eq!(card["ready"], true);
    assert_eq!(card["setup"]["state"], "ready");
    assert!(card.get("needs").is_none());
}

#[test]
fn oauth_readiness_distinguishes_missing_fresh_expired_refreshable_and_corrupt() {
    let dir = Scratch::new("melchior-ready", "oauth");
    let listener = TcpListener::bind("127.0.0.1:0").expect("token listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let url = format!("http://{}", listener.local_addr().expect("address"));
    let auth = format!(
        r#"{{kind="oauth",service="Fixture",authorize_url="{url}/authorize",token_url="{url}/token",client_id="public"}}"#
    );
    config(&dir, &url, &auth, "openai-completions");
    let missing = cards(&dir, &[]);
    assert_eq!(missing["setup"]["state"], "credential_missing");
    assert_eq!(missing["setup"]["login"], true);
    for (access, refresh, expiry, state) in [
        ("private-sentinel", None, u64::MAX, "ready"),
        ("private-sentinel", None, 1, "credential_expired"),
        (
            "private-sentinel",
            Some("private-sentinel-refresh"),
            1,
            "ready",
        ),
        (" ", None, u64::MAX, "credential_expired"),
        ("private-sentinel", Some(" "), 1, "credential_expired"),
    ] {
        tokens(&dir, access, refresh, expiry);
        let before = std::fs::read(dir.join("data/magi/credentials.json")).expect("store");
        let card = cards(&dir, &[]);
        assert_eq!(card["setup"]["state"], state);
        assert_eq!(card["ready"], state == "ready");
        assert_eq!(
            std::fs::read(dir.join("data/magi/credentials.json")).expect("store"),
            before
        );
    }
    tokens(
        &dir,
        "private-sentinel",
        Some("private-sentinel-refresh"),
        1,
    );
    config(
        &dir,
        &url,
        r#"{kind="oauth",service="Fixture"}"#,
        "openai-completions",
    );
    assert_eq!(
        cards(&dir, &[])["setup"]["state"],
        "configuration_incomplete"
    );
    config(
        &dir,
        &url,
        r#"{kind="oauth",service="Fixture",client_id="public"}"#,
        "openai-completions",
    );
    assert_eq!(
        cards(&dir, &[])["setup"]["fields"],
        json!(["auth.token_url"])
    );
    std::fs::write(
        dir.join("data/magi/credentials.json"),
        "private-sentinel-corrupt",
    )
    .expect("corrupt fixture");
    assert_eq!(cards(&dir, &[])["setup"]["state"], "credential_unreadable");
    assert_eq!(
        listener
            .accept()
            .expect_err("listing must not exchange credentials")
            .kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[test]
fn unusable_endpoints_protocols_and_auth_methods_are_not_ready() {
    let dir = Scratch::new("melchior-ready", "setup");
    for url in [
        "",
        "not a url",
        "http://remote.invalid",
        "https://name:private-sentinel@fixture.invalid",
    ] {
        config(&dir, url, r#"{kind="none"}"#, "openai-completions");
        let card = cards(&dir, &[]);
        assert_eq!(card["ready"], false);
        assert_eq!(card["setup"]["state"], "configuration_incomplete");
    }
    config(
        &dir,
        "https://fixture.invalid",
        r#"{kind="none"}"#,
        "bedrock-converse-stream",
    );
    assert_eq!(cards(&dir, &[])["setup"]["state"], "protocol_unavailable");
    config(
        &dir,
        "https://fixture.invalid",
        r#"{kind="google-adc"}"#,
        "google-vertex",
    );
    assert_eq!(cards(&dir, &[])["setup"]["state"], "auth_unavailable");
    config(
        &dir,
        "https://fixture.invalid",
        r#"{kind="none"}"#,
        "openai-completions",
    );
    std::fs::write(
        dir.join("apis.lua"),
        r#"melchior.api("openai-completions", {request=function() return {} end})"#,
    )
    .expect("incomplete protocol");
    assert_eq!(cards(&dir, &[])["setup"]["state"], "protocol_unavailable");
}

fn server(listener: TcpListener) -> std::thread::JoinHandle<(String, Value)> {
    response_server(listener, 200,
        "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n".into())
}

fn response_server(
    listener: TcpListener,
    status: u16,
    body: String,
) -> std::thread::JoinHandle<(String, Value)> {
    response_server_with_timeout(listener, status, body, std::time::Duration::from_secs(5))
}

fn response_server_with_timeout(
    listener: TcpListener,
    status: u16,
    body: String,
    wait: std::time::Duration,
) -> std::thread::JoinHandle<(String, Value)> {
    response_server_observing(listener, status, body, wait, || {})
}

fn response_server_observing(
    listener: TcpListener,
    status: u16,
    body: String,
    wait: std::time::Duration,
    before_reply: impl FnOnce() + Send + 'static,
) -> std::thread::JoinHandle<(String, Value)> {
    response_server_sized(listener, status, body, wait, 0, before_reply)
}

fn response_server_sized(
    listener: TcpListener,
    status: u16,
    body: String,
    wait: std::time::Duration,
    missing_bytes: usize,
    before_reply: impl FnOnce() + Send + 'static,
) -> std::thread::JoinHandle<(String, Value)> {
    std::thread::spawn(move || {
        listener.set_nonblocking(true).expect("nonblocking");
        let deadline = std::time::Instant::now() + wait;
        let mut socket = loop {
            match listener.accept() {
                Ok((socket, _)) => break socket,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(std::time::Instant::now() < deadline, "no inference request");
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Err(error) => panic!("accept: {error}"),
            }
        };
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .expect("read deadline");
        let mut bytes = Vec::new();
        while !bytes.ends_with(b"\r\n\r\n") {
            assert!(bytes.len() < 65536);
            let mut byte = [0];
            socket.read_exact(&mut byte).expect("header");
            bytes.push(byte[0]);
        }
        let header = String::from_utf8(bytes)
            .expect("UTF-8")
            .to_ascii_lowercase();
        let length: usize = header
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .expect("length")
            .trim()
            .parse()
            .expect("length number");
        assert!(length < 65536);
        let mut request = vec![0; length];
        socket.read_exact(&mut request).expect("body");
        assert!(!String::from_utf8_lossy(&request).contains("private-sentinel"));
        before_reply();
        write!(socket,"HTTP/1.1 {status} Fixture\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()+missing_bytes).expect("response");
        (
            header,
            serde_json::from_slice(&request).expect("request JSON"),
        )
    })
}

fn ask(dir: &Scratch, vars: &[(&str, &str)]) -> std::process::Output {
    ask_with(dir, vars, Default::default())
}

fn ask_with(
    dir: &Scratch,
    vars: &[(&str, &str)],
    wants: melchior::mind::wire::Wants,
) -> std::process::Output {
    let ask = melchior::mind::wire::Ask {
        model: "fixture/m".into(),
        context: melchior::mind::model::Context {
            messages: vec![melchior::mind::model::Message::user("hi")],
            ..Default::default()
        },
        wants,
        about: String::new(),
    };
    ask_request(dir, vars, &ask)
}

fn ask_request(
    dir: &Scratch,
    vars: &[(&str, &str)],
    ask: &melchior::mind::wire::Ask,
) -> std::process::Output {
    ask_request_with_timeout(dir, vars, ask, std::time::Duration::from_secs(10))
}

fn ask_request_with_timeout(
    dir: &Scratch,
    vars: &[(&str, &str)],
    ask: &melchior::mind::wire::Ask,
    wait: std::time::Duration,
) -> std::process::Output {
    let mut child = command(dir)
        .envs(vars.iter().copied())
        .arg("ask")
        .arg("--json")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("ask");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(&serde_json::to_vec(ask).expect("ask JSON"))
        .expect("send ask");
    let deadline = std::time::Instant::now() + wait;
    while child.try_wait().expect("inference status").is_none() {
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("inference fixture exceeded its deadline");
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let out = child.wait_with_output().expect("inference");
    assert!(!String::from_utf8_lossy(&out.stdout).contains("private-sentinel"));
    assert!(!String::from_utf8_lossy(&out.stderr).contains("private-sentinel"));
    out
}

#[test]
fn fresh_stored_oauth_token_can_infer_without_refresh_configuration() {
    let dir = Scratch::new("melchior-ready", "fresh-inference");
    let listener = TcpListener::bind("127.0.0.1:0").expect("inference listener");
    let url = format!("http://{}", listener.local_addr().expect("address"));
    config(
        &dir,
        &url,
        r#"{kind="oauth",service="Fixture"}"#,
        "openai-completions",
    );
    tokens(&dir, "private-sentinel", None, u64::MAX);
    assert_eq!(cards(&dir, &[])["ready"], true);
    let server = server(listener);
    let out = ask(&dir, &[]);
    let (header, _) = server.join().expect("server");
    assert!(header.starts_with("post /chat/completions "));
    assert!(header.contains("authorization: bearer private-sentinel\r\n"));
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).expect("UTF-8");
    assert!(
        stdout
            .lines()
            .any(|line| serde_json::from_str::<Value>(line)
                .is_ok_and(|event| event["event"] == "stop")),
        "{stdout}"
    );
    assert!(!stdout.contains("private-sentinel"));
}
