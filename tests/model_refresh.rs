//! Explicit model discovery against an isolated provider and cache.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

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
    requests: Arc<Mutex<Vec<String>>>,
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
        let requests = Arc::new(Mutex::new(Vec::new()));
        let paths = requests.clone();
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
                let request = String::from_utf8_lossy(&bytes[..n]);
                let first = request.lines().next().expect("request");
                assert!(
                    first.starts_with("GET /v1/models ")
                        || first.starts_with("GET /v1/models?sort=most-popular ")
                );
                paths.lock().expect("requests").push(first.to_owned());
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
            requests,
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

#[test]
fn openrouter_refresh_carries_prices_and_weekly_positions_through_cache_and_wire() {
    let server = Provider::new();
    let dir = Scratch::new("melchior", "model-metadata");
    std::fs::write(
        dir.join("providers.lua"),
        format!(
            r#"
melchior.provider("ollama", {{ discover = false, models = {{}} }})
melchior.provider("openrouter", {{ name = "Fixture", api = "openai-completions",
  base_url = "{}", auth = {{ kind = "none" }}, discover = true, details = "openrouter" }})
"#,
            server.url
        ),
    )
    .expect("provider");
    *server.response.lock().expect("response") = (
        200,
        json!({"data":[
            {"id":"popular","context_length":128000,"pricing":{"prompt":"0.0000003","completion":"0.0000012"},
             "architecture":{"input_modalities":["text","image","video"],"output_modalities":["text"]},
             "supported_parameters":["reasoning"],"reasoning":{"supported_efforts":["low","high","xhigh"]}},
            {"id":"free","pricing":{"prompt":"0","completion":"0"}},
            {"id":"unknown","pricing":{"prompt":"-1","completion":"-1"}}
        ]}),
    );
    let initial = list(&dir, true);
    let find = |reply: &Value, id: &str| {
        reply["result"]
            .as_array()
            .expect("cards")
            .iter()
            .find(|card| card["id"] == id)
            .expect("card")
            .clone()
    };
    let popular = find(&initial, "openrouter/popular");
    assert_eq!(popular["pricing"], json!({"input":"0.3","output":"1.2"}));
    assert_eq!(popular["popularity_rank"], 1);
    assert_eq!(
        popular["capabilities"],
        json!({
            "input":["text","image","video"], "output":["text"], "thinking":["low","high","xhigh"]
        })
    );
    assert_eq!(popular["vision"], true);
    assert_eq!(
        find(&initial, "openrouter/free")["pricing"],
        json!({"input":"0","output":"0"})
    );
    assert!(
        find(&initial, "openrouter/unknown")
            .get("pricing")
            .is_none()
    );
    assert!(
        server
            .requests
            .lock()
            .expect("requests")
            .iter()
            .all(|request| request.starts_with("GET /v1/models?sort=most-popular "))
    );
    let cached = list(&dir, false);
    assert_eq!(find(&cached, "openrouter/popular"), popular);
    assert_eq!(server.calls.load(Ordering::SeqCst), 1);
    *server.response.lock().expect("response") = (
        200,
        json!({"data":[
            {"id":"free","pricing":{"prompt":"0","completion":"0"}},
            {"id":"popular","pricing":{"prompt":"0.000002","completion":"0.000004"},
             "architecture":{"input_modalities":["text","audio"],"output_modalities":["audio"]},
             "supported_parameters":["reasoning"],"reasoning":{"supported_efforts":["medium"]}}
        ]}),
    );
    let updated = list(&dir, true);
    assert_eq!(
        find(&updated, "openrouter/popular")["capabilities"],
        json!({
            "input":["text","audio"], "output":["audio"], "thinking":["medium"]
        })
    );
    let updated = find(&updated, "openrouter/popular");
    assert_eq!(updated["pricing"], json!({"input":"2","output":"4"}));
    assert_eq!(updated["popularity_rank"], 2);
    *server.response.lock().expect("response") = (500, json!({"data":[]}));
    let failed = list(&dir, true);
    assert_eq!(failed["failed"], json!(["openrouter"]));
    assert_eq!(find(&failed, "openrouter/popular"), updated);
}
