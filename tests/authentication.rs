#![allow(clippy::expect_used, clippy::indexing_slicing)]

use melchior::{mind::provider::oauth::Store, scratch::Scratch};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

const SECRET: &str = "synthetic-private-api-key";

fn fixture(name: &str, auth: &str) -> Scratch {
    let dir = Scratch::new("login", name);
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
        .expect("private directory");
    config(&dir, auth, "https://fixture.invalid");
    dir
}

fn config(dir: &Scratch, auth: &str, base: &str) {
    std::fs::write(dir.join("providers.lua"), format!(r#"
        for _, id in ipairs({{"openrouter", "ollama"}}) do
            melchior.provider(id, {{name=id,api="openai-completions",auth={{kind="none"}}}})
        end
        melchior.provider("fixture", {{name="Fixture",api="openai-completions",base_url={base:?},auth={auth},
            models={{{{id="m",name="Fixture",context_window=8192,max_tokens=128}}}}}})
    "#)).expect("config");
}

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

struct Running {
    child: Child,
    replies: Receiver<Value>,
}
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Running {
    fn reply(&self) -> Value {
        self.replies
            .recv_timeout(Duration::from_secs(15))
            .expect("bounded auth reply")
    }
}

fn start(dir: &Scratch, request: Value, vars: &[(&str, &str)]) -> Running {
    let mut child = command(dir)
        .envs(vars.iter().copied())
        .args(["auth", "--stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("auth process");
    writeln!(child.stdin.take().expect("stdin"), "{request}").expect("request");
    let out = child.stdout.take().expect("stdout");
    let (tx, replies) = channel();
    std::thread::spawn(move || {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            assert!(!line.contains(SECRET), "credential leaked in reply");
            let Ok(value) = serde_json::from_str(&line) else {
                return;
            };
            if tx.send(value).is_err() {
                return;
            }
        }
    });
    Running { child, replies }
}

fn call(dir: &Scratch, op: &str, key: &str) -> Value {
    start(
        dir,
        json!({"version":1,"event":op,"provider":"fixture","key":key}),
        &[],
    )
    .reply()
}

fn account(reply: &Value) -> &Value {
    reply["providers"]
        .as_array()
        .expect("providers")
        .iter()
        .find(|a| a["id"] == "fixture")
        .expect("fixture")
}

fn path(dir: &Scratch) -> std::path::PathBuf {
    dir.join("data/magi/credentials.json")
}

#[test]
fn saved_key_is_private_redacted_ready_and_removable_with_env_precedence() {
    let dir = fixture("key", r#"{kind="api-key",vars={"FIXTURE_KEY"}}"#);
    assert_eq!(account(&call(&dir, "list", ""))["source"], "none");
    let saved = call(&dir, "set_key", SECRET);
    assert!(saved["error"].is_null(), "{saved}");
    assert_eq!(account(&saved)["source"], "saved");
    assert_eq!(
        std::fs::metadata(path(&dir))
            .expect("file")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let cards = command(&dir)
        .args(["models", "--json"])
        .output()
        .expect("cards");
    let cards: Value = serde_json::from_slice(&cards.stdout).expect("json");
    assert!(
        cards["result"]
            .as_array()
            .expect("models")
            .iter()
            .any(|v| v["id"] == "fixture/m" && v["ready"] == true)
    );
    let reply = start(
        &dir,
        json!({"version":1,"event":"remove","provider":"fixture"}),
        &[("FIXTURE_KEY", "environment-secret")],
    )
    .reply();
    assert_eq!(account(&reply)["source"], "environment");
    assert_eq!(account(&reply)["saved"], false);
    assert_eq!(account(&call(&dir, "list", ""))["source"], "none");
}

#[test]
fn saved_key_cannot_follow_changed_provider_configuration() {
    let auth = r#"{kind="api-key",vars={"FIXTURE_KEY"}}"#;
    let dir = fixture("scope", auth);
    assert!(call(&dir, "set_key", SECRET)["error"].is_null());
    config(&dir, auth, "https://different.invalid");
    let reply = call(&dir, "list", "");
    assert_eq!(
        account(&reply)["status"],
        "Expired or configuration changed"
    );
    assert_eq!(account(&reply)["saved"], true);
}

#[test]
fn bad_keys_and_unconfigured_oauth_do_not_overwrite_credentials() {
    let dir = fixture("invalid", r#"{kind="api-key",vars={"FIXTURE_KEY"}}"#);
    assert!(call(&dir, "set_key", SECRET)["error"].is_null());
    for key in ["", "spaces are invalid", "control\nchar"] {
        assert!(call(&dir, "set_key", key)["error"].is_string());
    }
    assert_eq!(
        Store::load_from(&path(&dir)).expect("store").api_keys["fixture"].value,
        SECRET
    );
    assert!(call(&dir, "set_key", &"x".repeat(8193))["error"].is_string());
    config(
        &dir,
        r#"{kind="oauth",service="Fixture"}"#,
        "https://fixture.invalid",
    );
    assert_eq!(account(&call(&dir, "list", ""))["available"], false);
    assert!(call(&dir, "login", "")["error"].is_string());
}

#[test]
fn corrupt_store_is_not_replaced_or_echoed() {
    let dir = fixture("corrupt", r#"{kind="api-key",vars={"FIXTURE_KEY"}}"#);
    call(&dir, "set_key", SECRET);
    std::fs::write(path(&dir), format!("{SECRET} malformed")).expect("corrupt fixture");
    let before = std::fs::read(path(&dir)).expect("before");
    assert!(call(&dir, "set_key", "replacement")["error"].is_string());
    assert_eq!(std::fs::read(path(&dir)).expect("after"), before);
}

fn oauth_fixture(name: &str) -> (Scratch, TcpListener) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("token server");
    let token = format!("http://{}/token", listener.local_addr().expect("addr"));
    let auth = format!(
        r#"{{kind="oauth",service="Fixture",authorize_url="https://fixture.invalid/authorize",token_url={token:?},client_id="test-public-client",scopes={{"offline_access"}}}}"#
    );
    (fixture(name, &auth), listener)
}

fn send_callback(url: &reqwest::Url, state: &str, code: &str) -> String {
    let redirect = url
        .query_pairs()
        .find(|(k, _)| k == "redirect_uri")
        .expect("redirect")
        .1
        .into_owned();
    let target = reqwest::Url::parse(&redirect).expect("redirect URL");
    let mut socket =
        TcpStream::connect(("127.0.0.1", target.port().expect("port"))).expect("callback");
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout");
    write!(
        socket,
        "GET /?state={state}&code={code} HTTP/1.1\r\nHost: localhost\r\n\r\n"
    )
    .expect("callback request");
    let mut answer = String::new();
    socket
        .read_to_string(&mut answer)
        .expect("callback response");
    answer
}

fn token_server(listener: TcpListener) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        listener.set_nonblocking(true).expect("nonblocking");
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut stream = loop {
            if let Ok((stream, _)) = listener.accept() {
                break stream;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "token request deadline"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("timeout");
        let mut reader = BufReader::new(stream.try_clone().expect("clone"));
        let mut length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).expect("header");
            if line == "\r\n" {
                break;
            }
            if let Some((name, value)) = line.split_once(':')
                && name.eq_ignore_ascii_case("content-length")
            {
                length = value.trim().parse::<usize>().expect("length");
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).expect("body");
        let body = String::from_utf8(body).expect("form");
        assert!(body.contains("code_verifier="));
        assert!(body.contains("grant_type=authorization_code"));
        let response = r#"{"access_token":"oauth-private-access","refresh_token":"oauth-private-refresh","expires_in":3600}"#;
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).expect("response");
    })
}

#[test]
fn browser_pkce_ignores_bad_state_then_saves_and_removes_tokens() {
    let (dir, listener) = oauth_fixture("oauth");
    let running = start(
        &dir,
        json!({"version":1,"event":"login","provider":"fixture"}),
        &[],
    );
    let browser = running.reply();
    assert_eq!(browser["event"], "browser");
    let url = reqwest::Url::parse(browser["url"].as_str().expect("url")).expect("URL");
    assert!(
        url.query_pairs()
            .any(|(k, v)| k == "code_challenge_method" && v == "S256")
    );
    assert!(send_callback(&url, "invalid", "ignored").contains("400 Bad Request"));
    let state = url
        .query_pairs()
        .find(|(k, _)| k == "state")
        .expect("state")
        .1
        .into_owned();
    let server = token_server(listener);
    assert!(send_callback(&url, &state, "test-code").contains("200 OK"));
    let finished = running.reply();
    assert!(finished["error"].is_null(), "{finished}");
    assert!(!finished.to_string().contains("oauth-private"));
    server.join().expect("token server");
    assert_eq!(account(&finished)["status"], "Signed in");
    assert!(
        Store::load_from(&path(&dir))
            .expect("store")
            .get("fixture")
            .is_some()
    );
    let provider_file = dir.join("providers.lua");
    let declaration = std::fs::read_to_string(&provider_file).expect("fixture declaration");
    std::fs::write(
        &provider_file,
        declaration.replace("test-public-client", "different-client"),
    )
    .expect("changed client");
    assert_eq!(
        account(&call(&dir, "list", ""))["status"],
        "Expired or configuration changed"
    );
    assert_eq!(account(&call(&dir, "remove", ""))["saved"], false);
}

#[test]
fn removed_provider_credentials_remain_visible_and_removable() {
    let dir = fixture("orphan", r#"{kind="api-key",vars={"FIXTURE_KEY"}}"#);
    assert!(call(&dir, "set_key", SECRET)["error"].is_null());
    let provider_file = dir.join("providers.lua");
    let declaration = std::fs::read_to_string(&provider_file).expect("fixture declaration");
    std::fs::write(
        &provider_file,
        declaration.replace("\"fixture\"", "\"renamed\""),
    )
    .expect("rename provider");
    let reply = call(&dir, "list", "");
    assert_eq!(account(&reply)["status"], "Provider no longer configured");
    assert_eq!(account(&reply)["saved"], true);
    assert!(call(&dir, "remove", "")["error"].is_null());
    assert!(
        !Store::load_from(&path(&dir))
            .expect("store")
            .api_keys
            .contains_key("fixture")
    );
}

#[test]
fn removing_during_browser_login_prevents_late_resurrection() {
    let (dir, listener) = oauth_fixture("remove-race");
    let running = start(
        &dir,
        json!({"version":1,"event":"login","provider":"fixture"}),
        &[],
    );
    let browser = running.reply();
    let url = reqwest::Url::parse(browser["url"].as_str().expect("url")).expect("URL");
    assert!(call(&dir, "remove", "")["error"].is_null());
    let server = token_server(listener);
    let state = url
        .query_pairs()
        .find(|(k, _)| k == "state")
        .expect("state")
        .1
        .into_owned();
    send_callback(&url, &state, "late-code");
    assert!(running.reply()["error"].is_string());
    server.join().expect("token server");
    assert!(
        Store::load_from(&path(&dir))
            .expect("store")
            .get("fixture")
            .is_none()
    );
}
