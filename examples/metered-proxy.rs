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
/// A request that died in flight stays counted at its reservation, which errs toward the cap.
fn committed(ledger: &str) -> u64 {
    let mut open: BTreeMap<String, u64> = BTreeMap::new();
    let mut settled = 0;
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

/// What the provider said a request used, from the last event of its stream or from its body.
fn usage_of(answer: &str) -> Option<serde_json::Value> {
    let whole = serde_json::from_str::<serde_json::Value>(answer).ok();
    let events = answer
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter_map(|data| serde_json::from_str::<serde_json::Value>(data).ok());
    whole
        .into_iter()
        .chain(events)
        .filter_map(|event| {
            event
                .get("usage")
                .filter(|usage| usage.is_object())
                .cloned()
        })
        .next_back()
}

/// What to charge: the provider's own figure where it gives one, else the tokens at list price,
/// else — nothing having been reported — the reservation, which is the most it could have been.
fn charge(terms: &Terms, usage: Option<&serde_json::Value>, reserved: u64) -> u64 {
    let Some(usage) = usage else {
        return reserved;
    };
    if let Some(cost) = usage.get("cost").and_then(serde_json::Value::as_f64) {
        return (cost * 1_000_000.0).ceil() as u64;
    }
    let count = |key: &str| {
        usage
            .get(key)
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
    };
    (count("prompt_tokens") * terms.input_price + count("completion_tokens") * terms.output_price)
        .div_ceil(1_000_000)
}

async fn request(socket: &mut tokio::net::TcpStream) -> std::io::Result<Option<String>> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if socket.read(&mut byte).await? == 0 {
            return Ok(None);
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
    let mut body = vec![0; size];
    socket.read_exact(&mut body).await?;
    Ok(Some(String::from_utf8_lossy(&body).into_owned()))
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
    let Some(body) = request(socket).await? else {
        return Ok(());
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

    let began = std::time::Instant::now();
    let sent = client
        .post(format!("{}/chat/completions", terms.upstream))
        .bearer_auth(&terms.key)
        .header("Content-Type", "application/json")
        .body(body.clone())
        .send()
        .await;
    let mut answer = Vec::new();
    let status = match sent {
        Ok(mut reply) => {
            let status = reply.status().as_u16();
            let kind = reply
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("application/json")
                .to_owned();
            let head = format!(
                "HTTP/1.1 {status} Metered\r\nContent-Type: {kind}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
            );
            socket.write_all(head.as_bytes()).await?;
            while let Ok(Some(part)) = reply.chunk().await {
                answer.extend_from_slice(&part);
                socket
                    .write_all(format!("{:x}\r\n", part.len()).as_bytes())
                    .await?;
                socket.write_all(&part).await?;
                socket.write_all(b"\r\n").await?;
                socket.flush().await?;
            }
            socket.write_all(b"0\r\n\r\n").await?;
            status
        }
        Err(why) => {
            refuse(
                socket,
                502,
                &format!("the provider could not be reached: {why}"),
            )
            .await?;
            0
        }
    };

    let answer = String::from_utf8_lossy(&answer);
    let usage = usage_of(&answer);
    // A request the provider refused outright was not charged for; anything else is.
    let micros = if usage.is_none() && !(200..300).contains(&status) {
        0
    } else {
        charge(terms, usage.as_ref(), reserved)
    };
    let row = serde_json::json!({
        "micros": micros, "reserved": reserved, "status": status,
        "ms": began.elapsed().as_millis() as u64, "usage": usage, "model": terms.model,
    });
    settle(terms, &id, &row);
    if let Some(path) = &terms.record
        && let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
    {
        let _ = writeln!(
            file,
            "{}",
            serde_json::json!({"id": id, "request": asked, "outcome": row})
        );
    }
    Ok(())
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
        model: need("model"),
        key,
        ledger: need("ledger"),
        record: flags.get("record").cloned(),
        cap_micros: number("cap-micros"),
        input_price: number("input-price"),
        output_price: number("output-price"),
        output_bound: number("output-bound"),
    });
    let client = reqwest::Client::new();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    println!("PORT={}", listener.local_addr()?.port());
    std::io::stdout().flush()?;

    let run = std::process::id();
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
