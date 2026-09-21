//! A provider that says what it was told to.
//!
//! The chat-completions wire, served over real HTTP, so a whole family can be run against answers
//! chosen in advance. The transport, the streaming and the dialect are production code; only the
//! source of the events is fake. Each request's body is recorded, so a test can assert what was
//! actually sent rather than what was meant to be.

use std::collections::BTreeMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(serde::Deserialize)]
struct Turn {
    /// Which requests this answers. Absent, the turn is taken in order.
    #[serde(default)]
    when: Option<When>,
    #[serde(default)]
    events: Vec<Event>,
    /// Refuse instead of answering, the way a provider does: a status and what it said.
    #[serde(default)]
    refuse: Option<Refusal>,
}

#[derive(serde::Deserialize)]
struct Refusal {
    status: u16,
    message: String,
}

/// What a turn answers. Named on shape rather than on wording: a system prompt is written for a
/// model and gets rewritten, where the tools a request declares are part of what it is.
#[derive(serde::Deserialize)]
struct When {
    /// A request declaring exactly this many tools. A helper job declares none.
    #[serde(default)]
    tools: Option<usize>,
    /// A request whose body contains this.
    #[serde(default)]
    body: Option<String>,
    /// A request whose body does not contain this: a lead's history quotes the brief it gave, so
    /// the brief alone cannot tell a child from the lead that started it.
    #[serde(default)]
    without: Option<String>,
    /// Answer one request and no more, so a second rule of the same shape answers the next.
    #[serde(default)]
    once: bool,
}

impl When {
    /// Whether this says anything at all; one that does not is an ordinary turn.
    fn is_rule(&self) -> bool {
        self.tools.is_some() || self.body.is_some()
    }

    fn holds(&self, body: &str, tools: usize) -> bool {
        self.tools.is_none_or(|n| n == tools)
            && self.body.as_deref().is_none_or(|b| body.contains(b))
            && self.without.as_deref().is_none_or(|b| !body.contains(b))
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum Event {
    Text(String),
    Thinking(String),
    ToolCall {
        id: String,
        name: String,
        arguments: String,
    },
    Finish(String),
    /// Text, sent the way a bad network sends it: CRLF line endings, and the bytes of one
    /// character split across two chunks.
    Split(String),
    /// Say nothing for this many milliseconds, with the answer still open.
    Pause(u64),
    /// Hang up here, with the answer unfinished and nothing to say it ended.
    Drop,
    Usage {
        input: u64,
        output: u64,
    },
}

impl Event {
    /// One `data:` payload in the dialect melchior's `openai-completions` block decodes.
    fn payload(&self) -> serde_json::Value {
        let choice = |delta: serde_json::Value, finish: serde_json::Value| serde_json::json!({"choices":[{"index":0,"delta":delta,"finish_reason":finish}]});
        match self {
            Self::Text(text) => choice(
                serde_json::json!({"content": text}),
                serde_json::Value::Null,
            ),
            Self::Thinking(text) => choice(
                serde_json::json!({"reasoning_content": text}),
                serde_json::Value::Null,
            ),
            Self::ToolCall {
                id,
                name,
                arguments,
            } => choice(
                serde_json::json!({"tool_calls":[{
                    "index": 0, "id": id, "type": "function",
                    "function": {"name": name, "arguments": arguments}
                }]}),
                serde_json::Value::Null,
            ),
            Self::Finish(reason) => choice(serde_json::json!({}), serde_json::json!(reason)),
            Self::Split(text) => choice(
                serde_json::json!({"content": text}),
                serde_json::Value::Null,
            ),
            Self::Drop => serde_json::Value::Null,
            Self::Pause(_) => serde_json::Value::Null,
            Self::Usage { input, output } => serde_json::json!({
                "choices": [],
                "usage": {"prompt_tokens": input, "completion_tokens": output}
            }),
        }
    }
}

fn flags() -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let Some(name) = arg.strip_prefix("--") else {
            continue;
        };
        match name.split_once('=') {
            Some((key, value)) => out.insert(key.to_owned(), value.to_owned()),
            None => out.insert(name.to_owned(), args.next().unwrap_or_default()),
        };
    }
    out
}

/// Read one request, and give back its body.
async fn request(socket: &mut tokio::net::TcpStream) -> std::io::Result<Option<String>> {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() > 65536 {
            return Ok(None);
        }
        match socket.read_u8().await {
            Ok(byte) => head.push(byte),
            Err(_) => return Ok(None),
        }
    }
    let text = String::from_utf8_lossy(&head).into_owned();
    let size = text
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .and_then(|n| n.trim().parse::<usize>().ok())
        })
        .unwrap_or(0);
    let mut body = vec![0; size];
    socket.read_exact(&mut body).await?;
    Ok(Some(String::from_utf8_lossy(&body).into_owned()))
}

/// The framing melchior's own fixtures use: chunked, and closed when the turn is done.
async fn stream(socket: &mut tokio::net::TcpStream, turn: &Turn) -> std::io::Result<()> {
    socket
        .write_all(
            b"HTTP/1.1 200 Fake\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
        )
        .await?;
    for event in &turn.events {
        match event {
            Event::Drop => return socket.shutdown().await,
            Event::Pause(ms) => {
                socket.flush().await?;
                tokio::time::sleep(std::time::Duration::from_millis(*ms)).await;
                continue;
            }
            Event::Split(_) => {
                let bytes = format!("data: {}\r\n\r\n", event.payload()).into_bytes();
                // Inside the first character that is more than one byte, or the middle.
                let at = bytes
                    .iter()
                    .position(|byte| *byte >= 0x80)
                    .map_or(bytes.len() / 2, |first| first + 1);
                for part in [&bytes[..at], &bytes[at..]] {
                    let mut framed = format!("{:x}\r\n", part.len()).into_bytes();
                    framed.extend_from_slice(part);
                    framed.extend_from_slice(b"\r\n");
                    socket.write_all(&framed).await?;
                    socket.flush().await?;
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
                continue;
            }
            _ => {}
        }
        chunk(socket, &format!("data: {}\n\n", event.payload())).await?;
    }
    chunk(socket, "data: [DONE]\n\n").await?;
    socket.write_all(b"0\r\n\r\n").await?;
    socket.flush().await
}

async fn chunk(socket: &mut tokio::net::TcpStream, text: &str) -> std::io::Result<()> {
    socket
        .write_all(format!("{:x}\r\n{text}\r\n", text.len()).as_bytes())
        .await
}

async fn refuse(socket: &mut tokio::net::TcpStream, status: u16, why: &str) -> std::io::Result<()> {
    let body = serde_json::json!({"error": {"message": why}}).to_string();
    socket
        .write_all(
            format!(
                "HTTP/1.1 {status} Fake\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .await
}

/// Which turn answers a request, chosen and marked under one lock so two requests arriving
/// together are never handed the same turn.
fn choose(turns: &[Turn], used: &mut [bool], body: &str) -> Option<usize> {
    let tools = match serde_json::from_str::<serde_json::Value>(body) {
        Ok(asked) => asked
            .get("tools")
            .and_then(|t| t.as_array())
            .map_or(0, Vec::len),
        Err(_) => 0,
    };
    // A rule answers as often as it holds and is never used up, because helper jobs and a
    // session's own turns arrive in no fixed order. The rest are handed out in turn.
    let rule = |at: usize| turns[at].when.as_ref().is_some_and(When::is_rule);
    let once = |at: usize| turns[at].when.as_ref().is_some_and(|w| w.once);
    let named = (0..turns.len()).find(|&at| {
        rule(at)
            && !(used[at] && once(at))
            && turns[at]
                .when
                .as_ref()
                .is_some_and(|w| w.holds(body, tools))
    });
    let chosen = named.or_else(|| (0..turns.len()).find(|&at| !used[at] && !rule(at)))?;
    used[chosen] = !rule(chosen) || once(chosen);
    Some(chosen)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    let flags = flags();
    let script = flags.get("script").expect("--script <file>");
    let turns: std::sync::Arc<Vec<Turn>> =
        std::sync::Arc::new(serde_json::from_str(&std::fs::read_to_string(script)?)?);
    let port: u16 = flags.get("port").map_or(0, |p| p.parse().unwrap_or(0));
    let record = flags.get("record").cloned();

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    println!("PORT={}", listener.local_addr()?.port());
    std::io::Write::flush(&mut std::io::stdout())?;

    let used = std::sync::Arc::new(std::sync::Mutex::new(vec![false; turns.len()]));
    loop {
        let (mut socket, _) = listener.accept().await?;
        let (turns, used, record) = (turns.clone(), used.clone(), record.clone());
        // Each on its own task, as a provider serves its customers: one answer held open must
        // not keep everybody else waiting behind it.
        tokio::spawn(async move {
            let Ok(Some(body)) = request(&mut socket).await else {
                return;
            };
            if let Some(path) = &record {
                use std::io::Write;
                if let Ok(mut file) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                {
                    let _ = writeln!(file, "{body}");
                }
            }
            let chosen = used
                .lock()
                .ok()
                .and_then(|mut used| choose(&turns, &mut used, &body));
            let _ = match chosen.map(|at| &turns[at]) {
                Some(turn) => match &turn.refuse {
                    Some(no) => refuse(&mut socket, no.status, &no.message).await,
                    None => stream(&mut socket, turn).await,
                },
                None => refuse(&mut socket, 500, "no turn was scripted for this request").await,
            };
        });
    }
}
