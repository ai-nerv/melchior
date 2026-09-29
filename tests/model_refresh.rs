//! Explicit model discovery against an isolated provider and cache.

use melchior::scratch::Scratch;
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::Duration;

struct Provider {
    url: String,
    response: Arc<Mutex<(u16, Value)>>,
    calls: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Provider {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("local provider");
        listener.set_nonblocking(true).expect("refresh fixture");
        let url = format!(
            "http://{}/v1",
            listener.local_addr().expect("refresh fixture")
        );
        let response = Arc::new(Mutex::new((200, json!({"data":[{"id":"old"}]}))));
        let calls = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (reply, count, finished) = (response.clone(), calls.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            while !finished.load(Ordering::SeqCst) {
                let Ok((mut stream, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(2));
                    continue;
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .expect("refresh fixture");
                let mut bytes = [0; 4096];
                let n = stream.read(&mut bytes).expect("refresh fixture");
                assert!(String::from_utf8_lossy(&bytes[..n]).starts_with("GET /v1/models "));
                count.fetch_add(1, Ordering::SeqCst);
                let (status, body) = reply.lock().expect("refresh fixture").clone();
                let body = body.to_string();
                write!(stream, "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).expect("refresh fixture");
            }
        });
        Self {
            url,
            response,
            calls,
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for Provider {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.thread
            .take()
            .expect("refresh fixture")
            .join()
            .expect("refresh fixture");
    }
}

fn list(dir: &Scratch, refresh: bool) -> Value {
    let mut command = Command::new(env!("CARGO_BIN_EXE_melchior"));
    command
        .env_clear()
        .env("HOME", dir.as_os_str())
        .env("MELCHIOR_CONFIG", dir.as_os_str())
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env("XDG_CACHE_HOME", dir.join("cache"))
        .env("XDG_DATA_HOME", dir.join("data"))
        .env("XDG_RUNTIME_DIR", dir.join("runtime"))
        .args(["models", "--json"]);
    if refresh {
        command.arg("--refresh");
    }
    let out = command.output().expect("model listing");
    assert!(out.status.success());
    let reply: Value = serde_json::from_slice(&out.stdout).expect("JSON reply");
    assert_eq!(reply["ok"], true, "{reply}");
    reply
}

fn models(reply: &Value) -> Vec<String> {
    reply["result"]
        .as_array()
        .expect("refresh fixture")
        .iter()
        .filter(|card| card["provider"] == "ollama")
        .map(|card| card["id"].as_str().expect("refresh fixture").to_owned())
        .collect()
}

#[test]
fn refresh_bypasses_fresh_cache_and_empty_success_removes_models() {
    let server = Provider::new();
    let dir = Scratch::new("melchior", "model-refresh");
    std::fs::write(
        dir.join("providers.lua"),
        format!(
            r#"
melchior.provider("openrouter", {{ discover = false, models = {{}} }})
melchior.provider("ollama", {{ name = "Fixture", api = "openai-completions",
  base_url = "{}", auth = {{ kind = "none" }}, discover = true }})
"#,
            server.url
        ),
    )
    .expect("refresh fixture");
    assert_eq!(models(&list(&dir, false)), ["ollama/old"]);
    *server.response.lock().expect("refresh fixture") = (200, json!({"data":[{"id":"new"}]}));
    assert_eq!(models(&list(&dir, false)), ["ollama/old"]);
    assert_eq!(server.calls.load(Ordering::SeqCst), 1);
    let fresh = list(&dir, true);
    assert_eq!(fresh["refreshed"], true);
    assert_eq!(fresh["failed"], json!([]));
    assert_eq!(models(&fresh), ["ollama/new"]);
    assert_eq!(server.calls.load(Ordering::SeqCst), 2);
    assert_eq!(models(&list(&dir, false)), ["ollama/new"]);
    for (status, body) in [(500, json!({"data":[]})), (200, json!({"error":"bad"}))] {
        *server.response.lock().expect("refresh fixture") = (status, body);
        let failed = list(&dir, true);
        assert_eq!(models(&failed), ["ollama/new"]);
        assert_eq!(failed["failed"], json!(["ollama"]));
    }
    *server.response.lock().expect("refresh fixture") = (200, json!({"data":[]}));
    assert!(models(&list(&dir, true)).is_empty());
    assert!(models(&list(&dir, false)).is_empty());
    let cached: Value = serde_json::from_slice(
        &std::fs::read(dir.join("cache/melchior/models/ollama.json")).expect("refresh fixture"),
    )
    .expect("refresh fixture");
    assert_eq!(cached, json!([]));
}
