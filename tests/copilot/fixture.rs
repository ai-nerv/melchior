use melchior::{
    mind::{
        model::{Context, Message},
        provider::oauth,
        wire::Ask,
    },
    scratch::Scratch,
};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    os::unix::fs::PermissionsExt,
    process::{Child, Command, Output, Stdio},
    sync::mpsc::{Receiver, channel},
    time::{Duration, Instant},
};

pub const GITHUB: &str = "github-private-sentinel";
pub const COPILOT: &str = "copilot-private-sentinel";

pub fn config(url: &str) -> Scratch {
    let dir = Scratch::new("copilot", "native");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).expect("private");
    std::fs::write(dir.join("providers.lua"), format!(r#"
        for _, id in ipairs({{"openrouter", "ollama"}}) do
            melchior.provider(id, {{name=id,api="openai-completions",auth={{kind="none"}}}})
        end
        melchior.provider("fixture", {{name="Copilot fixture",api="openai-completions",base_url={url:?},
            auth={{kind="github-copilot",client_id="fixture-client",device_url="{url}/device",oauth_url="{url}/oauth",token_url="{url}/token",verification_url="{url}/verify"}},
            models={{{{id="mc",name="Chat",context_window=8192,max_tokens=128}},
                {{id="ma",name="Messages",api="anthropic-messages",context_window=8192,max_tokens=128}},
                {{id="mr",name="Responses",api="openai-responses",context_window=8192,max_tokens=128}},
                {{id="disabled",name="Disabled",context_window=8192,max_tokens=128}}}}}})
    "#)).expect("config");
    dir
}

pub fn command(dir: &Scratch) -> Command {
    let mut command = Command::new(binary());
    command
        .env_clear()
        .env("HOME", dir.as_os_str())
        .env("MELCHIOR_CONFIG", dir.as_os_str())
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env("XDG_DATA_HOME", dir.join("data"))
        .env("XDG_CACHE_HOME", dir.join("cache"))
        .env("XDG_RUNTIME_DIR", dir.join("runtime"));
    command
}

fn binary() -> std::path::PathBuf {
    std::fs::canonicalize(
        std::env::var_os("MELCHIOR_TEST_BINARY")
            .unwrap_or_else(|| env!("CARGO_BIN_EXE_melchior").into()),
    )
    .expect("selected Melchior")
}
pub fn path(dir: &Scratch) -> std::path::PathBuf {
    dir.join("data/magi/credentials.json")
}
pub struct Login {
    pub child: Child,
    receiver: Receiver<Value>,
}
impl Drop for Login {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Login {
    pub fn reply(&self) -> Value {
        let reply = self
            .receiver
            .recv_timeout(Duration::from_secs(20))
            .expect("auth deadline");
        if reply["event"] == "device" {
            assert_eq!(
                std::fs::read_link(format!("/proc/{}/exe", self.child.id()))
                    .expect("running Melchior image"),
                binary()
            );
        }
        reply
    }
}
pub fn auth(dir: &Scratch, op: &str) -> Login {
    let mut child = command(dir)
        .args(["auth", "--stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("auth");
    writeln!(
        child.stdin.take().expect("stdin"),
        "{}",
        json!({"version":1,"event":op,"provider":"fixture"})
    )
    .expect("request");
    let stdout = child.stdout.take().expect("stdout");
    let (tx, receiver) = channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            assert!(!line.contains("private-sentinel"));
            let value = serde_json::from_str(&line).expect("reply");
            if tx.send(value).is_err() {
                break;
            }
        }
    });
    Login { child, receiver }
}

pub fn ask_start(dir: &Scratch, model: &str) -> Child {
    let mut child = command(dir)
        .args(["ask", "--json"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("ask");
    let request = Ask {
        model: format!("fixture/{model}"),
        context: Context {
            messages: vec![Message::user("hello")],
            ..Default::default()
        },
        wants: Default::default(),
        about: String::new(),
    };
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(&serde_json::to_vec(&request).expect("request"))
        .expect("send");
    child
}
pub fn finish(mut child: Child) -> Output {
    let deadline = Instant::now() + Duration::from_secs(20);
    while child.try_wait().expect("status").is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("native request deadline");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let out = child.wait_with_output().expect("output");
    assert!(!String::from_utf8_lossy(&out.stdout).contains("private-sentinel"));
    assert!(!String::from_utf8_lossy(&out.stderr).contains("private-sentinel"));
    out
}
pub fn ask(dir: &Scratch, model: &str) -> Output {
    finish(ask_start(dir, model))
}
pub fn assert_answer(out: Output) {
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("\"text\":\"ok\""),
        "{} {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

pub struct Step {
    path: &'static str,
    body: String,
    content: &'static str,
    status: u16,
}
impl Step {
    pub fn status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }
    pub fn body(mut self, body: String) -> Self {
        self.body = body;
        self
    }
}
pub fn step(path: &'static str, value: Value) -> Step {
    Step {
        path,
        body: value.to_string(),
        content: "application/json",
        status: 200,
    }
}
pub fn device(url: &str, expires: u64) -> Step {
    step(
        "/device",
        json!({"device_code":"device-private-sentinel","user_code":"ABCD-EFGH","verification_uri":format!("{url}/verify"),"expires_in":expires,"interval":1}),
    )
}
pub fn granted() -> Step {
    step(
        "/oauth",
        json!({"access_token":GITHUB,"token_type":"bearer"}),
    )
}
pub fn token() -> Step {
    step(
        "/token",
        json!({"token":COPILOT,"expires_at":oauth::now()+3600}),
    )
}
pub fn models() -> Step {
    step(
        "/models",
        json!({"data":[
            {"id":"mc","model_picker_enabled":true,"policy":{"state":"enabled"}},
            {"id":"ma","model_picker_enabled":true},
            {"id":"mr","model_picker_enabled":true,"policy":{"state":"enabled"}},
            {"id":"disabled","model_picker_enabled":true,"policy":{"state":"disabled"}},
            {"id":"unconfigured","model_picker_enabled":true,"policy":{"state":"unconfigured"}},
            {"id":"no-tools","model_picker_enabled":true,"policy":{"state":"enabled"},"capabilities":{"supports":{"tool_calls":false}}}
        ]}),
    )
}
pub fn chat() -> Step {
    Step { status:200, path:"/chat/completions", content:"text/event-stream", body: "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":2,\"completion_tokens\":1}}\n\ndata: [DONE]\n\n".into() }
}
pub fn anthropic() -> Step {
    Step { status:200, path:"/v1/messages", content:"text/event-stream", body: concat!(
    "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":2,\"output_tokens\":0}}}\n\n",
    "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\n",
    "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\n",
    "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n").into() }
}
pub fn responses() -> Step {
    Step { status:200, path:"/responses", content:"text/event-stream", body: concat!(
    "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"ok\"}\n\n",
    "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":2,\"output_tokens\":1}}}\n\n").into() }
}

pub struct Request {
    pub path: String,
    pub headers: String,
    pub body: String,
}
pub struct Mock {
    listener: TcpListener,
    pub url: String,
}
impl Mock {
    pub fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let url = format!("http://{}", listener.local_addr().expect("address"));
        Self { listener, url }
    }
    pub fn serve(&self, steps: Vec<Step>) -> std::thread::JoinHandle<Vec<Request>> {
        let listener = self.listener.try_clone().expect("listener");
        std::thread::spawn(move || {
            listener.set_nonblocking(true).expect("nonblocking");
            let mut requests = Vec::new();
            for step in steps {
                let deadline = Instant::now() + Duration::from_secs(20);
                let socket = loop {
                    match listener.accept() {
                        Ok((socket, _)) => break socket,
                        Err(error)
                            if error.kind() == std::io::ErrorKind::WouldBlock
                                && Instant::now() < deadline =>
                        {
                            std::thread::sleep(Duration::from_millis(2))
                        }
                        Err(error) => panic!("fixture request {}: {error}", step.path),
                    }
                };
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .expect("timeout");
                let mut reader = BufReader::new(socket);
                let mut headers = String::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).expect("line");
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    headers.push_str(&line);
                    assert!(headers.len() < 32768);
                }
                let path = headers
                    .lines()
                    .next()
                    .expect("request")
                    .split_whitespace()
                    .nth(1)
                    .expect("path")
                    .to_owned();
                assert_eq!(path, step.path);
                let length = headers
                    .lines()
                    .filter_map(|line| line.split_once(':'))
                    .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                    .map_or(0, |(_, value)| {
                        value.trim().parse::<usize>().expect("length")
                    });
                assert!(length < 1024 * 1024);
                let mut body = vec![0; length];
                reader.read_exact(&mut body).expect("body");
                requests.push(Request {
                    path,
                    headers,
                    body: String::from_utf8(body).expect("body text"),
                });
                let mut socket = reader.into_inner();
                let _ = write!(
                    socket,
                    "HTTP/1.1 {} Fixture\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    step.status,
                    step.content,
                    step.body.len(),
                    step.body
                );
            }
            requests
        })
    }
}
