//! Scripted Chat Completions and Anthropic Messages over HTTP, with JSONL request capture.

use std::io::Read as _;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[path = "fake-provider/anthropic.rs"]
mod anthropic;

#[path = "fake-provider/bounded.rs"]
mod bounded;

#[path = "fake-provider/rendezvous.rs"]
mod rendezvous;

#[path = "fake-provider/refusal.rs"]
mod refusal;
use refusal::Refusal;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
enum Dialect {
    #[default]
    #[serde(rename = "openai-completions")]
    Chat,
    #[serde(rename = "anthropic-messages")]
    Anthropic,
}

#[derive(Debug, Default, Clone, Copy, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Counts {
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
}

#[derive(serde::Deserialize)]
struct Turn {
    #[serde(default)]
    rendezvous: Option<String>,
    #[serde(default)]
    dialect: Dialect,
    #[serde(default)]
    usage: Option<Counts>,
    /// Which requests this answers. Absent, the turn is taken in order.
    #[serde(default)]
    when: Option<When>,
    #[serde(default)]
    events: Vec<Event>,
    /// Refuse instead of answering, the way a provider does: a status and what it said.
    #[serde(default)]
    refuse: Option<Refusal>,
}

impl Turn {
    fn validate(&self) -> std::io::Result<()> {
        if self.events.len() > 4096 {
            return Err(std::io::Error::other("too many fixture events"));
        }
        match self.dialect {
            Dialect::Anthropic if self.refuse.is_some() => Ok(()),
            Dialect::Anthropic => anthropic::validate(self),
            Dialect::Chat
                if self.usage.is_some()
                    || self.events.iter().any(|e| matches!(e, Event::Signature(_))) =>
            {
                Err(std::io::Error::other(
                    "turn-level usage and signatures require Anthropic",
                ))
            }
            Dialect::Chat => Ok(()),
        }
    }
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
    Signature(String),
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
    Metrics(serde_json::Value),
}

impl Event {
    /// One `data:` payload in the dialect melchior's `openai-completions` block decodes.
    fn payload(&self) -> std::io::Result<serde_json::Value> {
        let choice = |delta: serde_json::Value, finish: serde_json::Value| serde_json::json!({"choices":[{"index":0,"delta":delta,"finish_reason":finish}]});
        Ok(match self {
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
            Self::Metrics(metrics) => serde_json::json!({"choices":[],"usage":metrics}),
            Self::Signature(_) => {
                return Err(std::io::Error::other(
                    "signatures require the Anthropic dialect",
                ));
            }
        })
    }
}

/// Read one request, and give back its body.
async fn request(socket: &mut tokio::net::TcpStream) -> std::io::Result<Option<String>> {
    request_start(socket, Vec::new()).await
}

async fn request_start(
    socket: &mut tokio::net::TcpStream,
    head: Vec<u8>,
) -> std::io::Result<Option<String>> {
    let Some(size) = request_header(socket, head).await? else {
        return Ok(None);
    };
    request_body(socket, size).await.map(Some)
}

async fn request_header(
    socket: &mut tokio::net::TcpStream,
    mut head: Vec<u8>,
) -> std::io::Result<Option<usize>> {
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
    if size > melchior::framing::MOST {
        return Err(std::io::Error::other(
            "fixture request exceeds the frame cap",
        ));
    }
    Ok(Some(size))
}

async fn request_body(socket: &mut tokio::net::TcpStream, size: usize) -> std::io::Result<String> {
    let mut body = vec![0; size];
    socket.read_exact(&mut body).await?;
    String::from_utf8(body).map_err(|_| std::io::Error::other("non-UTF8 fixture request"))
}

async fn start_stream(socket: &mut tokio::net::TcpStream) -> std::io::Result<()> {
    socket
        .write_all(
            b"HTTP/1.1 200 Fake\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
        )
        .await
}

/// The scripted dialect over chunked HTTP.
async fn stream(socket: &mut tokio::net::TcpStream, turn: &Turn) -> std::io::Result<()> {
    if turn.dialect == Dialect::Anthropic {
        return anthropic::stream(socket, turn).await;
    }
    start_stream(socket).await?;
    for event in &turn.events {
        match event {
            Event::Drop => return socket.shutdown().await,
            Event::Pause(ms) => {
                socket.flush().await?;
                tokio::time::sleep(std::time::Duration::from_millis(*ms)).await;
                continue;
            }
            Event::Split(_) => {
                let bytes = format!("data: {}\r\n\r\n", event.payload()?).into_bytes();
                // Inside the first character that is more than one byte, or the middle.
                let at = bytes
                    .iter()
                    .position(|byte| *byte >= 0x80)
                    .map_or(bytes.len() / 2, |first| first + 1);
                let (head, tail) = bytes.split_at_checked(at).unwrap_or((&bytes, &[]));
                for part in [head, tail] {
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
        chunk(socket, &format!("data: {}\n\n", event.payload()?)).await?;
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

async fn refuse(
    socket: &mut tokio::net::TcpStream,
    status: u16,
    body: serde_json::Value,
) -> std::io::Result<()> {
    let body = body.to_string();
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
    let when = |at: usize| turns.get(at).and_then(|turn| turn.when.as_ref());
    let rule = |at: usize| when(at).is_some_and(When::is_rule);
    let once = |at: usize| when(at).is_some_and(|w| w.once);
    let spent = |at: usize| used.get(at).copied().unwrap_or(true);
    let named = (0..turns.len()).find(|&at| {
        rule(at) && !(spent(at) && once(at)) && when(at).is_some_and(|w| w.holds(body, tools))
    });
    let chosen = named.or_else(|| (0..turns.len()).find(|&at| !spent(at) && !rule(at)))?;
    let mark = !rule(chosen) || once(chosen);
    *used.get_mut(chosen)? = mark;
    Some(chosen)
}

fn read_script(path: &str) -> std::io::Result<Vec<Turn>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(melchior::framing::MOST as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > melchior::framing::MOST {
        return Err(std::io::Error::other(
            "fixture script exceeds the frame cap",
        ));
    }
    let turns: Vec<Turn> = serde_json::from_slice(&bytes)?;
    if turns.len() > 4096 {
        return Err(std::io::Error::other("too many fixture turns"));
    }
    for turn in &turns {
        turn.validate()?;
    }
    rendezvous::Groups::new(&turns)?;
    Ok(turns)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    let flags = bounded::Flags::parse(std::env::args().skip(1))?;
    let turns = std::sync::Arc::new(read_script(&flags.script)?);
    if let Some(policy) = flags.bounded {
        let record = flags
            .record
            .ok_or_else(|| std::io::Error::other("bounded capture missing"))?;
        return bounded::run(turns, flags.port, &record, policy).await;
    }
    if turns.iter().any(|turn| turn.rendezvous.is_some()) {
        return Err(std::io::Error::other(
            "rendezvous requires bounded provider mode",
        ));
    }
    let port = flags.port;
    let record = flags
        .record
        .as_ref()
        .map(|path| {
            use std::os::unix::fs::OpenOptionsExt as _;
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .mode(0o600)
                .open(path)
        })
        .transpose()?;
    let record = std::sync::Arc::new(std::sync::Mutex::new(record));

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    println!("PORT={}", listener.local_addr()?.port());
    std::io::Write::flush(&mut std::io::stdout())?;

    let used = std::sync::Arc::new(std::sync::Mutex::new(vec![false; turns.len()]));
    let capacity = std::sync::Arc::new(tokio::sync::Semaphore::new(16));
    loop {
        let permit = std::sync::Arc::clone(&capacity)
            .acquire_owned()
            .await
            .map_err(|_| std::io::Error::other("fixture capacity closed"))?;
        let (mut socket, _) = listener.accept().await?;
        let (turns, used, record) = (turns.clone(), used.clone(), record.clone());
        // Each on its own task, as a provider serves its customers: one answer held open must
        // not keep everybody else waiting behind it.
        tokio::spawn(async move {
            let _permit = permit;
            let Ok(Ok(Some(body))) =
                tokio::time::timeout(std::time::Duration::from_secs(15), request(&mut socket))
                    .await
            else {
                return;
            };
            let recorded = (|| {
                use std::io::Write;
                let mut held = record
                    .lock()
                    .map_err(|_| std::io::Error::other("fixture capture lock poisoned"))?;
                if let Some(file) = &mut *held {
                    writeln!(file, "{body}")?;
                }
                Ok::<(), std::io::Error>(())
            })();
            if let Err(error) = recorded {
                eprintln!("fixture capture failed: {error}");
                return;
            }
            let chosen = used
                .lock()
                .ok()
                .and_then(|mut used| choose(&turns, &mut used, &body));
            let _ = match chosen.and_then(|at| turns.get(at)) {
                Some(turn) => match &turn.refuse {
                    Some(no) => refuse(&mut socket, no.status, no.body()).await,
                    None => stream(&mut socket, turn).await,
                },
                None => refuse(&mut socket, 500, serde_json::json!({"error":{"message":"no turn was scripted for this request"}})).await,
            };
        });
    }
}
