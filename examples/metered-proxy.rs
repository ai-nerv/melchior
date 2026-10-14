//! A proxy that meters what a family spends on a live provider.
//!
//! It stands between the family and the provider and is the only thing holding the key, so the
//! scratch world under test never sees a credential. Every request is checked against one model
//! and one cumulative cap before it leaves, recorded without its headers, and settled against a
//! ledger that outlives the run: a rerun is not handed a fresh allowance.

use std::collections::BTreeMap;
use std::io::Write;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// What one run may do, fixed when it starts.
struct Terms {
    upstream: String,
    /// What is asked of the upstream: a chat completion unless told otherwise.
    path: String,
    model: String,
    key: String,
    ledger: String,
    record: Option<String>,
    cap_micros: u64,
    /// Micro-dollars per million tokens, for a reservation and for a provider that names no cost.
    input_price: u64,
    output_price: u64,
    /// What a request that names no `max_tokens` is reserved as asking for.
    output_bound: u64,
}

/// What a request could cost at most: every byte of it a token, and the whole reply it may ask
/// for. Deliberately more than it will: a reservation that is too small is no bound at all.
fn reservation(terms: &Terms, body: &str, asked: &serde_json::Value) -> u64 {
    let output = asked
        .get("max_tokens")
        .or_else(|| asked.get("max_completion_tokens"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(terms.output_bound);
    let input = body.len() as u64;
    (input * terms.input_price + output * terms.output_price).div_ceil(1_000_000)
}

/// Every line of the ledger added up: what was settled, and what was reserved and never settled.
/// A request that died in flight stays counted at its reservation, which errs toward the cap. A
/// `corrected` line replaces what its request was settled at: the ledger is only ever added to,
/// so a charge that turns out wrong is put right by a line that says so.
fn committed(ledger: &str) -> u64 {
    let mut open: BTreeMap<String, u64> = BTreeMap::new();
    // What each request was last settled at, so a correction takes that back and no more: an id
    // seen twice is two requests of two runs, and both were paid for.
    let mut last: BTreeMap<String, u64> = BTreeMap::new();
    let mut settled = 0u64;
    for line in ledger.lines() {
        let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let id = row["id"].as_str().unwrap_or_default().to_owned();
        let micros = row["micros"].as_u64().unwrap_or(0);
        match row["kind"].as_str() {
            Some("reserved") => {
                open.insert(id, micros);
            }
            Some("settled") => {
                open.remove(&id);
                settled += micros;
                last.insert(id, micros);
            }
            Some("corrected") => {
                settled = settled - last.get(&id).copied().unwrap_or(0).min(settled) + micros;
                last.insert(id, micros);
            }
            _ => {}
        }
    }
    settled + open.values().sum::<u64>()
}

/// Reserve under the ledger's lock, or say why not. Read, decided and written as one step, so two
/// requests arriving together cannot both take the last of the allowance.
fn reserve(terms: &Terms, id: &str, micros: u64) -> Result<(), String> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(&terms.ledger)
        .map_err(|why| format!("the ledger could not be opened: {why}"))?;
    rustix::fs::flock(&file, rustix::fs::FlockOperation::LockExclusive)
        .map_err(|why| format!("the ledger could not be locked: {why}"))?;
    let mut held = String::new();
    std::io::Read::read_to_string(&mut file, &mut held)
        .map_err(|why| format!("the ledger could not be read: {why}"))?;
    let spent = committed(&held);
    if spent + micros > terms.cap_micros {
        return Err(format!(
            "spend cap: {spent} micro-dollars are committed and this request reserves {micros}, \
             which would pass the cap of {}",
            terms.cap_micros
        ));
    }
    let row =
        serde_json::json!({"kind": "reserved", "id": id, "micros": micros, "model": terms.model});
    writeln!(file, "{row}").map_err(|why| format!("the ledger could not be written: {why}"))
}

fn settle(terms: &Terms, id: &str, row: &serde_json::Value) {
    let Ok(mut file) = std::fs::OpenOptions::new().append(true).open(&terms.ledger) else {
        return;
    };
    if rustix::fs::flock(&file, rustix::fs::FlockOperation::LockExclusive).is_ok() {
        let mut row = row.clone();
        row["kind"] = "settled".into();
        row["id"] = id.into();
        let _ = writeln!(file, "{row}");
    }
}

/// What the provider said a request used, gathered from its body or from every event of its
/// stream: OpenAI's `usage`, Anthropic's `usage` and `message.usage` split across events, and
/// Gemini's `usageMetadata`. A count said more than once is taken at its largest.
fn usage_of(answer: &str) -> Option<serde_json::Value> {
    let whole = serde_json::from_str::<serde_json::Value>(answer).ok();
    let events = answer
        .lines()
        .filter_map(|line| line.strip_prefix("data:").map(str::trim_start))
        .filter_map(|data| serde_json::from_str::<serde_json::Value>(data).ok());
    let mut merged = serde_json::Map::new();
    for event in whole.into_iter().chain(events) {
        let said = [
            event.get("usage"),
            event.pointer("/message/usage"),
            event.get("usageMetadata"),
        ];
        for usage in said.into_iter().flatten().filter_map(|it| it.as_object()) {
            for (key, value) in usage {
                let larger = merged
                    .get(key)
                    .and_then(serde_json::Value::as_f64)
                    .is_none_or(|held| value.as_f64().is_some_and(|now| now > held));
                if larger {
                    merged.insert(key.clone(), value.clone());
                }
            }
        }
    }
    (!merged.is_empty()).then_some(serde_json::Value::Object(merged))
}

/// What the provider said went wrong, from its body or from inside a stream it answered 200 to.
fn failure_of(answer: &str) -> Option<String> {
    let whole = serde_json::from_str::<serde_json::Value>(answer).ok();
    let events = answer
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter_map(|data| serde_json::from_str::<serde_json::Value>(data).ok());
    whole
        .into_iter()
        .chain(events)
        .find_map(|event| event["error"]["message"].as_str().map(str::to_owned))
}

/// The prompt and completion counts, under whichever names the provider used.
const PROMPT: &[&str] = &["prompt_tokens", "input_tokens", "promptTokenCount"];
const COMPLETION: &[&str] = &["completion_tokens", "output_tokens", "candidatesTokenCount"];

/// What to charge: the provider's own figure where it gives one, else the tokens at list price,
/// else — nothing it said being a count this reads — the reservation, the most it could have
/// been. A figure that is not a number, or is below zero, is no figure.
fn charge(terms: &Terms, usage: Option<&serde_json::Value>, reserved: u64) -> u64 {
    let Some(usage) = usage else {
        return reserved;
    };
    if let Some(cost) = usage.get("cost").and_then(serde_json::Value::as_f64) {
        if !cost.is_finite() || cost < 0.0 {
            return reserved;
        }
        return (cost * 1_000_000.0).ceil().min(u64::MAX as f64) as u64;
    }
    let count = |names: &[&str]| {
        names
            .iter()
            .find_map(|name| usage.get(*name).and_then(serde_json::Value::as_u64))
    };
    let (Some(prompt), completion) = (count(PROMPT), count(COMPLETION)) else {
        return reserved;
    };
    prompt
        .saturating_mul(terms.input_price)
        .saturating_add(completion.unwrap_or(0).saturating_mul(terms.output_price))
        .div_ceil(1_000_000)
}

/// The most a request's headers and body may hold, checked before anything is allocated for them.
const HEAD: usize = 64 * 1024;
const BODY: usize = 32 * 1024 * 1024;

/// How long the upstream may take to answer, and to say each next thing once it has begun.
const QUIET: std::time::Duration = std::time::Duration::from_secs(180);

/// One request's body, or why it will not be read.
async fn request<S: AsyncReadExt + Unpin>(socket: &mut S) -> std::io::Result<Result<String, u16>> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() >= HEAD {
            return Ok(Err(431));
        }
        if socket.read(&mut byte).await? == 0 {
            return Ok(Err(0));
        }
        head.push(byte[0]);
    }
    let size = String::from_utf8_lossy(&head)
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .and_then(|n| n.trim().parse::<usize>().ok())
        })
        .unwrap_or(0);
    if size > BODY {
        return Ok(Err(413));
    }
    let mut body = vec![0; size];
    socket.read_exact(&mut body).await?;
    Ok(Ok(String::from_utf8_lossy(&body).into_owned()))
}

/// A reservation not yet settled: settled when this goes, however the request ended — a caller
/// that hung up, an upstream that failed, or the task dropped.
struct Unsettled<'a> {
    terms: &'a Terms,
    id: String,
    reserved: u64,
    began: std::time::Instant,
    asked: serde_json::Value,
    status: u16,
    answer: Vec<u8>,
}

impl Drop for Unsettled<'_> {
    fn drop(&mut self) {
        let answer = String::from_utf8_lossy(&self.answer);
        let usage = usage_of(&answer);
        let failure = failure_of(&answer);
        // A request refused outright, or failed inside its stream with nothing used, was not
        // charged for; anything else is.
        let refused = !(200..300).contains(&self.status) || failure.is_some();
        let micros = if usage.is_none() && refused {
            0
        } else {
            charge(self.terms, usage.as_ref(), self.reserved)
        };
        let row = serde_json::json!({
            "micros": micros, "reserved": self.reserved, "status": self.status,
            "ms": self.began.elapsed().as_millis() as u64, "usage": usage,
            "model": self.terms.model, "error": failure,
        });
        settle(self.terms, &self.id, &row);
        if let Some(path) = &self.terms.record
            && let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
        {
            let _ = writeln!(
                file,
                "{}",
                serde_json::json!({"id": self.id, "request": self.asked, "outcome": row})
            );
        }
    }
}

async fn refuse(socket: &mut tokio::net::TcpStream, status: u16, why: &str) -> std::io::Result<()> {
    let body = serde_json::json!({"error": {"message": why}}).to_string();
    let head = format!(
        "HTTP/1.1 {status} Metered\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    socket.write_all(format!("{head}{body}").as_bytes()).await
}

/// One request: checked, reserved, sent on, passed back as it arrives, then settled and recorded.
async fn serve(
    terms: &Terms,
    client: &reqwest::Client,
    socket: &mut tokio::net::TcpStream,
    id: String,
) -> std::io::Result<()> {
    let body = match request(socket).await? {
        Ok(body) => body,
        Err(0) => return Ok(()),
        Err(status) => {
            return refuse(socket, status, "that request is larger than this reads").await;
        }
    };
    let asked: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
    let model = asked["model"].as_str().unwrap_or_default();
    if model != terms.model {
        let why = format!("only `{}` is authorized here, not `{model}`", terms.model);
        return refuse(socket, 403, &why).await;
    }
    let reserved = reservation(terms, &body, &asked);
    if let Err(why) = reserve(terms, &id, reserved) {
        return refuse(socket, 402, &why).await;
    }
    let mut held = Unsettled {
        terms,
        id,
        reserved,
        began: std::time::Instant::now(),
        asked,
        status: 0,
        answer: Vec::new(),
    };

    let sent = client
        .post(format!("{}{}", terms.upstream, terms.path))
        .bearer_auth(&terms.key)
        .header("Content-Type", "application/json")
        .body(body)
        .send();
    let mut reply = match tokio::time::timeout(QUIET, sent).await {
        Ok(Ok(reply)) => reply,
        Ok(Err(why)) => {
            let why = format!("the provider could not be reached: {}", why.without_url());
            return refuse(socket, 502, &why).await;
        }
        Err(_) => return refuse(socket, 504, "the provider did not answer in time").await,
    };
    held.status = reply.status().as_u16();
    let kind = reply
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json")
        .to_owned();
    let head = format!(
        "HTTP/1.1 {} Metered\r\nContent-Type: {kind}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
        held.status
    );
    socket.write_all(head.as_bytes()).await?;
    while let Ok(Ok(Some(part))) = tokio::time::timeout(QUIET, reply.chunk()).await {
        if held.answer.len() < BODY {
            held.answer.extend_from_slice(&part);
        }
        socket
            .write_all(format!("{:x}\r\n", part.len()).as_bytes())
            .await?;
        socket.write_all(&part).await?;
        socket.write_all(b"\r\n").await?;
        socket.flush().await?;
    }
    socket.write_all(b"0\r\n\r\n").await
}

fn flags() -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        if let (Some(key), Some(value)) = (flag.strip_prefix("--"), args.next()) {
            out.insert(key.to_owned(), value);
        }
    }
    out
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    let flags = flags();
    let need = |key: &str| {
        flags
            .get(key)
            .cloned()
            .unwrap_or_else(|| panic!("--{key} is required"))
    };
    let number = |key: &str| {
        need(key)
            .parse::<u64>()
            .unwrap_or_else(|_| panic!("--{key} is a number"))
    };
    // Named rather than passed, so the key is in no argument list anybody can read.
    let key = std::env::var(need("key-env")).unwrap_or_default();
    let terms = std::sync::Arc::new(Terms {
        upstream: need("upstream").trim_end_matches('/').to_owned(),
        path: flags
            .get("path")
            .cloned()
            .unwrap_or_else(|| "/chat/completions".to_owned()),
        model: need("model"),
        key,
        ledger: need("ledger"),
        record: flags.get("record").cloned(),
        cap_micros: number("cap-micros"),
        input_price: number("input-price"),
        output_price: number("output-price"),
        output_bound: number("output-bound"),
    });
    // A redirect followed would carry the key to wherever it pointed.
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("an HTTP client");
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    println!("PORT={}", listener.local_addr()?.port());
    std::io::stdout().flush()?;

    // The hour it started as well as the process: a process id comes round again, and a ledger
    // that outlives its runs must not take two requests for one.
    let run = format!(
        "{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_secs()),
        std::process::id()
    );
    let mut count = 0u64;
    loop {
        let (mut socket, _) = listener.accept().await?;
        count += 1;
        let (terms, client, id) = (terms.clone(), client.clone(), format!("{run}-{count}"));
        tokio::spawn(async move {
            let _ = serve(&terms, &client, &mut socket, id).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terms() -> Terms {
        Terms {
            upstream: String::new(),
            path: String::new(),
            model: "m".into(),
            key: String::new(),
            ledger: String::new(),
            record: None,
            cap_micros: 0,
            input_price: 1_000_000,
            output_price: 2_000_000,
            output_bound: 0,
        }
    }

    #[test]
    fn every_provider_s_counts_are_charged_and_none_is_free() {
        let anthropic = "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":10}}}\n\
                         data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":5}}\n";
        assert_eq!(charge(&terms(), usage_of(anthropic).as_ref(), 999), 20);
        let gemini =
            "data: {\"usageMetadata\":{\"promptTokenCount\":7,\"candidatesTokenCount\":3}}\n";
        assert_eq!(charge(&terms(), usage_of(gemini).as_ref(), 999), 13);
        let openai = r#"{"usage":{"prompt_tokens":1,"completion_tokens":1}}"#;
        assert_eq!(charge(&terms(), usage_of(openai).as_ref(), 999), 3);
    }

    #[test]
    fn a_figure_that_is_no_figure_is_charged_at_the_reservation() {
        let unknown = serde_json::json!({"tokens_in": 10});
        assert_eq!(charge(&terms(), Some(&unknown), 777), 777);
        for cost in [f64::NAN, -1.0, f64::INFINITY] {
            let said = serde_json::json!({"cost": cost});
            assert_eq!(charge(&terms(), Some(&said), 777), 777, "{cost}");
        }
        assert_eq!(charge(&terms(), None, 777), 777);
    }

    #[tokio::test]
    async fn headers_and_a_body_past_their_bound_are_refused_unread() {
        let mut long = std::io::Cursor::new(vec![b'x'; HEAD + 10]);
        assert_eq!(request(&mut long).await.expect("read"), Err(431));
        let huge = format!("POST / HTTP/1.1\r\nContent-Length: {}\r\n\r\n", BODY + 1);
        let mut huge = std::io::Cursor::new(huge.into_bytes());
        assert_eq!(request(&mut huge).await.expect("read"), Err(413));
        let fine = b"POST / HTTP/1.1\r\nContent-Length: 2\r\n\r\n{}".to_vec();
        let mut fine = std::io::Cursor::new(fine);
        assert_eq!(request(&mut fine).await.expect("read"), Ok("{}".to_owned()));
    }
}
