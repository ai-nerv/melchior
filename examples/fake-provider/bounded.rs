use super::Turn;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream};

const CAPTURE_LIMIT: usize = 8 * 1024 * 1024;
const JOURNAL_LIMIT: usize = 128 * 1024;
const EVENT_LIMIT: usize = 1024;

#[derive(Debug)]
pub(super) struct Policy {
    limit: u64,
    meter: String,
    capture_limit: usize,
}

#[derive(Debug)]
pub(super) struct Flags {
    pub script: String,
    pub port: u16,
    pub record: Option<String>,
    pub bounded: Option<Policy>,
}

impl Flags {
    pub fn parse(args: impl IntoIterator<Item = String>) -> io::Result<Self> {
        let mut args = args.into_iter();
        let mut values = BTreeMap::new();
        while let Some(arg) = args.next() {
            let raw = arg
                .strip_prefix("--")
                .ok_or_else(|| error("unexpected argument"))?;
            let (key, value) = match raw.split_once('=') {
                Some((key, value)) => (key, value.to_owned()),
                None => (raw, args.next().ok_or_else(|| error("missing flag value"))?),
            };
            if !matches!(
                key,
                "script" | "port" | "record" | "attempt-limit" | "meter" | "capture-limit"
            ) || value.is_empty()
                || value.starts_with("--")
                || values.insert(key.to_owned(), value).is_some()
            {
                return Err(error("unknown, repeated or empty fixture flag"));
            }
        }
        let script = values
            .remove("script")
            .ok_or_else(|| error("--script is required"))?;
        let port = values.remove("port").map_or(Ok(0), |p| {
            p.parse::<u16>().map_err(|_| error("invalid port"))
        })?;
        let record = values.remove("record");
        let capture_limit = values.remove("capture-limit");
        let bounded = match (values.remove("attempt-limit"), values.remove("meter")) {
            (None, None) if capture_limit.is_none() => None,
            (Some(limit), Some(meter)) if record.is_some() => {
                let number = limit
                    .parse::<u64>()
                    .map_err(|_| error("invalid attempt limit"))?;
                if !(1..=64).contains(&number) || number.to_string() != limit {
                    return Err(error(
                        "attempt limit must be a canonical integer from 1 to 64",
                    ));
                }
                Some(Policy {
                    limit: number,
                    meter,
                    capture_limit: capture_limit.map_or(Ok(CAPTURE_LIMIT), |raw| {
                        let number = raw
                            .parse::<usize>()
                            .map_err(|_| error("invalid capture limit"))?;
                        if !(1..=CAPTURE_LIMIT).contains(&number) || number.to_string() != raw {
                            return Err(error(
                                "capture limit must be a canonical integer from 1 to 8388608",
                            ));
                        }
                        Ok(number)
                    })?,
                })
            }
            _ => {
                return Err(error(
                    "bounded mode requires --attempt-limit, --meter and --record",
                ));
            }
        };
        Ok(Self {
            script,
            port,
            record,
            bounded,
        })
    }
}

fn error(message: &str) -> io::Error {
    io::Error::other(message)
}

fn private_file(path: &str) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

struct Meter<R: Write, M: Write> {
    limit: u64,
    admitted: u64,
    rejected: u64,
    responses: u64,
    capture_limit: usize,
    reserved_bytes: usize,
    captured_bytes: usize,
    journal_bytes: usize,
    fault: Option<&'static str>,
    captures: BTreeMap<u64, Option<usize>>,
    record: R,
    journal: M,
}

impl<R: Write, M: Write> Meter<R, M> {
    fn new(limit: u64, capture_limit: usize, record: R, journal: M) -> io::Result<Self> {
        if !(1..=64).contains(&limit) || !(1..=CAPTURE_LIMIT).contains(&capture_limit) {
            return Err(error("invalid meter limit"));
        }
        let mut meter = Self {
            limit,
            admitted: 0,
            rejected: 0,
            responses: 0,
            capture_limit,
            reserved_bytes: 0,
            captured_bytes: 0,
            journal_bytes: 0,
            fault: None,
            captures: BTreeMap::new(),
            record,
            journal,
        };
        meter.event(
            json!({"schema":2,"scope":"offline-provider-admission","event":"start","limit":limit,
                "capture_limit_bytes":capture_limit,"journal_limit_bytes":JOURNAL_LIMIT}),
        )?;
        Ok(meter)
    }

    fn event(&mut self, value: serde_json::Value) -> io::Result<()> {
        let terminal = value.get("event").and_then(serde_json::Value::as_str) == Some("terminal");
        let mut bytes = serde_json::to_vec(&value)?;
        bytes.push(b'\n');
        let available = JOURNAL_LIMIT - if terminal { 0 } else { EVENT_LIMIT };
        if bytes.len() > EVENT_LIMIT || self.journal_bytes + bytes.len() > available {
            self.fault = Some("journal_limit");
            return Err(error("journal_limit"));
        }
        self.journal_bytes += bytes.len();
        let result = self
            .journal
            .write_all(&bytes)
            .and_then(|()| self.journal.flush());
        if result.is_err() {
            self.fault = Some("journal_io");
        }
        result
    }

    fn healthy(&self) -> io::Result<()> {
        self.fault.map_or(Ok(()), |reason| Err(error(reason)))
    }

    fn reserve(&mut self) -> io::Result<Option<u64>> {
        self.healthy()?;
        if self.admitted == self.limit {
            self.rejected += 1;
            self.event(json!({"event":"reject","reason":"attempt_limit","rejected":self.rejected,"admitted":self.admitted}))?;
            return Ok(None);
        }
        self.admitted += 1;
        self.event(json!({"event":"admit","attempt":self.admitted}))?;
        Ok(Some(self.admitted))
    }

    fn reserve_capture(&mut self, attempt: u64, size: usize) -> io::Result<bool> {
        self.healthy()?;
        if attempt == 0 || attempt > self.admitted || self.captures.contains_key(&attempt) {
            self.fault = Some("capture_sequence");
            return Err(error("capture_sequence"));
        }
        let bytes = size
            .checked_add(1)
            .ok_or_else(|| error("capture_size_overflow"))?;
        if bytes > self.capture_limit - self.reserved_bytes {
            self.fault = Some("capture_limit");
            self.event(
                json!({"event":"reject_capture","reason":"capture_limit","attempt":attempt,
                "bytes":size,"reserved_bytes":self.reserved_bytes}),
            )?;
            return Ok(false);
        }
        self.reserved_bytes += bytes;
        self.captures.insert(attempt, Some(size));
        self.event(
            json!({"event":"reserve_capture","attempt":attempt,"bytes":size,
            "reserved_bytes":self.reserved_bytes}),
        )?;
        Ok(true)
    }

    fn capture(&mut self, attempt: u64, body: &str) -> io::Result<()> {
        self.healthy()?;
        let bytes = body
            .len()
            .checked_add(1)
            .ok_or_else(|| error("capture_size_overflow"))?;
        if self.captures.get_mut(&attempt).and_then(Option::take) != Some(body.len()) {
            self.fault = Some("unreserved_capture");
            return Err(error("unreserved_capture"));
        }
        let result = writeln!(self.record, "{body}").and_then(|()| self.record.flush());
        if let Err(e) = result {
            self.fault = Some("capture_io");
            return Err(e);
        }
        self.captured_bytes += bytes;
        self.event(json!({"event":"body","attempt":attempt,"bytes":body.len(),
            "sha256":format!("{:x}", Sha256::digest(body.as_bytes()))}))
    }

    fn responded(&mut self, attempt: u64, turn: usize) -> io::Result<()> {
        self.healthy()?;
        self.responses += 1;
        self.event(json!({"event":"response","attempt":attempt,"turn":turn}))
    }

    fn finish(&mut self, reason: &str) -> io::Result<()> {
        let reason = self.fault.unwrap_or(reason);
        let reason: String = reason.chars().take(128).collect();
        self.event(json!({"event":"terminal","status":if reason=="stopped" {"STOPPED"} else {"FAIL"},
            "reason":reason,"admitted":self.admitted,"rejected":self.rejected,"responses":self.responses,
            "incomplete":self.admitted-self.responses,"reserved_bytes":self.reserved_bytes,
            "captured_bytes":self.captured_bytes}))
    }
}

type Shared = Arc<Mutex<Meter<File, File>>>;

fn lock(meter: &Shared) -> io::Result<std::sync::MutexGuard<'_, Meter<File, File>>> {
    meter.lock().map_err(|_| error("meter lock poisoned"))
}

async fn discard(socket: &mut TcpStream, mut size: usize) -> io::Result<()> {
    let mut buffer = [0; 8192];
    while size > 0 {
        let chunk = size.min(buffer.len());
        socket
            .read_exact(
                buffer
                    .get_mut(..chunk)
                    .ok_or_else(|| error("discard buffer"))?,
            )
            .await?;
        size -= chunk;
    }
    Ok(())
}

async fn connection(
    mut socket: TcpStream,
    turns: Arc<Vec<Turn>>,
    used: Arc<Mutex<Vec<bool>>>,
    meter: Shared,
    groups: Arc<super::rendezvous::Groups>,
) -> io::Result<()> {
    let first = match tokio::time::timeout(Duration::from_secs(15), socket.read_u8()).await {
        Ok(Ok(byte)) => byte,
        Ok(Err(e)) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
        Ok(Err(e)) => return Err(e),
        Err(_) => return Err(error("request_timeout")),
    };
    let Some(attempt) = lock(&meter)?.reserve()? else {
        let _ = tokio::time::timeout(Duration::from_millis(250), async {
            if let Some(size) = super::request_header(&mut socket, vec![first]).await? {
                discard(&mut socket, size).await?;
            }
            Ok::<(), io::Error>(())
        })
        .await;
        let _ = tokio::time::timeout(Duration::from_secs(1), super::refuse(&mut socket, 429,
            json!({"error":{"type":"evaluation_attempt_limit","message":"cumulative fixture attempt limit exceeded"}}))).await;
        return Err(error("attempt_limit"));
    };
    let size = tokio::time::timeout(
        Duration::from_secs(15),
        super::request_header(&mut socket, vec![first]),
    )
    .await
    .map_err(|_| error("request_timeout"))??
    .ok_or_else(|| error("incomplete_request"))?;
    let admitted = lock(&meter)?.reserve_capture(attempt, size)?;
    if !admitted {
        let _ = tokio::time::timeout(Duration::from_millis(250), discard(&mut socket, size)).await;
        let _ = tokio::time::timeout(Duration::from_secs(1), super::refuse(&mut socket, 413,
            json!({"error":{"type":"evaluation_capture_limit","message":"cumulative fixture capture limit exceeded"}}))).await;
        return Err(error("capture_limit"));
    }
    let body = tokio::time::timeout(
        Duration::from_secs(15),
        super::request_body(&mut socket, size),
    )
    .await
    .map_err(|_| error("request_timeout"))??;
    lock(&meter)?.capture(attempt, &body)?;
    let chosen = {
        let mut held = used.lock().map_err(|_| error("script lock poisoned"))?;
        super::choose(&turns, &mut held, &body)
    };
    let at = chosen.ok_or_else(|| error("script_exhausted"))?;
    let turn = turns.get(at).ok_or_else(|| error("script_index"))?;
    if let Some(group) = &turn.rendezvous {
        lock(&meter)?.event(
            json!({"event":"rendezvous_arrive","group":group,"attempt":attempt,"turn":at}),
        )?;
        groups.arrive(group).await?;
        lock(&meter)?.event(
            json!({"event":"rendezvous_release","group":group,"attempt":attempt,"turn":at}),
        )?;
    }
    match &turn.refuse {
        Some(no) => super::refuse(&mut socket, no.status, no.body()).await?,
        None => super::stream(&mut socket, turn).await?,
    }
    lock(&meter)?.responded(attempt, at)
}

async fn serve(
    listener: TcpListener,
    turns: Arc<Vec<Turn>>,
    meter: Shared,
    stop: impl std::future::Future<Output = ()>,
) -> io::Result<()> {
    let used = Arc::new(Mutex::new(vec![false; turns.len()]));
    let groups = Arc::new(super::rendezvous::Groups::new(&turns)?);
    let mut workers = tokio::task::JoinSet::new();
    tokio::pin!(stop);
    let failure = loop {
        tokio::select! {
            biased;
            result = workers.join_next(), if !workers.is_empty() => {
                match result {
                    Some(Ok(Ok(()))) => {},
                    Some(Ok(Err(e))) => break Some(e),
                    _ => break Some(error("connection_task_failed")),
                }
            },
            () = &mut stop => break None,
            result = listener.accept(), if workers.len()<16 => {
                match result {
                    Ok((socket,_)) => { workers.spawn(connection(socket, turns.clone(), used.clone(), meter.clone(), groups.clone())); },
                    Err(e) => break Some(e),
                }
            },
        }
    };
    drop(listener);
    workers.abort_all();
    while workers.join_next().await.is_some() {}
    let mut held = lock(&meter)?;
    let failure = failure
        .or_else(|| held.fault.map(error))
        .or_else(|| (held.rejected > 0).then(|| error("attempt_limit")))
        .or_else(|| {
            (held.admitted != held.responses).then(|| error("shutdown_with_incomplete_requests"))
        });
    let reason = if let Some(fault) = held.fault {
        fault.to_owned()
    } else if held.rejected > 0 {
        "attempt_limit".to_owned()
    } else {
        failure
            .as_ref()
            .map_or_else(|| "stopped".to_owned(), ToString::to_string)
    };
    held.finish(&reason)?;
    failure.map_or(Ok(()), Err)
}

pub(super) async fn run(
    turns: Arc<Vec<Turn>>,
    port: u16,
    record: &str,
    policy: Policy,
) -> io::Result<()> {
    let meter = Arc::new(Mutex::new(Meter::new(
        policy.limit,
        policy.capture_limit,
        private_file(record)?,
        private_file(&policy.meter)?,
    )?));
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let listener = TcpListener::bind(("127.0.0.1", port)).await?;
    println!("PORT={}", listener.local_addr()?.port());
    io::stdout().flush()?;
    serve(listener, turns, meter, async move {
        tokio::select! { _ = term.recv() => {}, _ = interrupt.recv() => {} }
    })
    .await
}

#[cfg(test)]
#[path = "bounded_tests.rs"]
mod tests;
