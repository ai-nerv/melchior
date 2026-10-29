use super::*;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt, symlink};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::AsyncWriteExt;

static NEXT: AtomicU64 = AtomicU64::new(0);

struct World(PathBuf);

impl World {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "melchior-bounded-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .expect("private world");
        Self(path)
    }
    fn path(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into_owned()
    }
    fn journal(&self) -> Vec<serde_json::Value> {
        std::fs::read_to_string(self.path("meter"))
            .expect("journal")
            .lines()
            .map(|line| serde_json::from_str(line).expect("journal row"))
            .collect()
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn flags(args: &[&str]) -> io::Result<Flags> {
    Flags::parse(args.iter().map(|s| s.to_string()))
}

#[test]
fn strict_flags_preserve_valid_legacy_and_require_all_bounded_fields() {
    let old = flags(&["--script", "fixture", "--port=0", "--record", "capture"]).expect("legacy");
    assert!(old.bounded.is_none());
    assert_eq!(old.port, 0);
    let bounded = flags(&[
        "--script=fixture",
        "--record=capture",
        "--attempt-limit=64",
        "--meter=meter",
    ])
    .expect("bounded");
    let policy = bounded.bounded.expect("policy");
    assert_eq!(policy.limit, 64);
    assert_eq!(policy.capture_limit, CAPTURE_LIMIT);
    for args in [
        vec!["--script"],
        vec!["positional"],
        vec!["--script=s", "--port=bad"],
        vec!["--script=s", "--port=65536"],
        vec!["--script=s", "--record="],
        vec!["--script=s", "--record", "--port=0"],
        vec!["--script=s", "--script=t"],
        vec!["--script=s", "--attempts-limit=3"],
        vec!["--script=s", "--attempt-limit=3"],
        vec!["--script=s", "--meter=m"],
        vec!["--script=s", "--capture-limit=1"],
        vec!["--script=s", "--attempt-limit=3", "--meter=m"],
    ] {
        assert!(flags(&args).is_err(), "{args:?}");
    }
    for limit in [
        "0",
        "65",
        "-1",
        "01",
        "1.0",
        "1e0",
        "+1",
        "18446744073709551616",
    ] {
        assert!(
            flags(&[
                "--script=s",
                "--record=r",
                "--meter=m",
                &format!("--attempt-limit={limit}")
            ])
            .is_err()
        );
    }
}

#[test]
fn capture_flags_are_canonical_bounded_and_explicit_in_the_handshake() {
    for limit in [
        "0",
        "8388609",
        "-1",
        "01",
        "+1",
        "1.0",
        "1e0",
        "18446744073709551616",
    ] {
        assert!(
            flags(&[
                "--script=s",
                "--record=r",
                "--meter=m",
                "--attempt-limit=1",
                &format!("--capture-limit={limit}")
            ])
            .is_err()
        );
    }
    let policy = flags(&[
        "--script=s",
        "--record=r",
        "--meter=m",
        "--attempt-limit=1",
        "--capture-limit=1",
    ])
    .expect("flags")
    .bounded
    .expect("policy");
    assert_eq!(policy.capture_limit, 1);
    let meter = Meter::new(1, 1, Vec::new(), Vec::new()).expect("meter");
    let start: serde_json::Value = serde_json::from_slice(&meter.journal).expect("start");
    assert_eq!(
        start,
        json!({"schema":2,"scope":"offline-provider-admission","event":"start",
        "limit":1,"capture_limit_bytes":1,"journal_limit_bytes":JOURNAL_LIMIT})
    );
}

#[test]
fn exact_capture_limit_includes_delimiters_and_preserves_original_bytes() {
    let body = "a\né";
    let mut meter = Meter::new(2, body.len() + 1, Vec::new(), Vec::new()).expect("meter");
    meter.reserve().expect("attempt");
    assert!(meter.reserve_capture(1, body.len()).expect("reserve"));
    meter.capture(1, body).expect("capture");
    assert_eq!(meter.record, format!("{body}\n").as_bytes());
    meter.reserve().expect("attempt");
    assert!(!meter.reserve_capture(2, 0).expect("overflow"));
    assert_eq!(meter.captured_bytes, body.len() + 1);
    assert_eq!(meter.reserved_bytes, body.len() + 1);
    meter.finish("stopped").expect("terminal");
    assert!(
        String::from_utf8(meter.journal)
            .expect("utf8")
            .contains("\"status\":\"FAIL\"")
    );
}

#[test]
fn capture_cannot_be_unreserved_duplicated_or_reused() {
    let mut meter = Meter::new(1, 10, Vec::new(), Vec::new()).expect("meter");
    meter.reserve().expect("attempt");
    assert!(meter.capture(1, "{}").is_err());
    assert!(meter.record.is_empty());
    let mut meter = Meter::new(1, 10, Vec::new(), Vec::new()).expect("meter");
    meter.reserve().expect("attempt");
    assert!(meter.reserve_capture(1, 2).expect("reserve"));
    meter.capture(1, "{}").expect("capture");
    assert!(meter.reserve_capture(1, 2).is_err());
    assert!(meter.capture(1, "{}").is_err());
    assert_eq!(meter.record, b"{}\n");
}

#[test]
fn concurrent_body_reservations_share_one_nonrefundable_capture_budget() {
    let meter = Arc::new(Mutex::new(
        Meter::new(64, 64, Vec::new(), Vec::new()).expect("meter"),
    ));
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let meter = meter.clone();
            scope.spawn(move || {
                for _ in 0..8 {
                    let mut held = meter.lock().expect("lock");
                    if let Ok(Some(attempt)) = held.reserve() {
                        let _ = held.reserve_capture(attempt, 3);
                    }
                }
            });
        }
    });
    let held = meter.lock().expect("lock");
    assert_eq!(held.reserved_bytes, 64);
    assert_eq!(held.captured_bytes, 0);
    assert_eq!(held.captures.len(), 16);
    assert_eq!(held.fault, Some("capture_limit"));
}

#[test]
fn journal_exhaustion_keeps_space_for_a_terminal_failure() {
    let mut meter = Meter::new(1, 1, Vec::new(), Vec::new()).expect("meter");
    while meter
        .event(json!({"event":"test","padding":"x".repeat(100)}))
        .is_ok()
    {}
    let before = meter.journal.len();
    assert!(before <= JOURNAL_LIMIT - EVENT_LIMIT);
    assert!(meter.reserve().is_err());
    meter.finish("stopped").expect("reserved terminal space");
    assert!(meter.journal.len() > before && meter.journal.len() <= JOURNAL_LIMIT);
    let rows = String::from_utf8(meter.journal).expect("utf8");
    let last: serde_json::Value =
        serde_json::from_str(rows.lines().last().expect("terminal")).expect("row");
    assert_eq!(last["status"], "FAIL");
    assert_eq!(last["reason"], "journal_limit");
}

#[test]
fn admission_is_shared_and_exact_under_concurrent_reservations() {
    let meter = Arc::new(Mutex::new(
        Meter::new(64, CAPTURE_LIMIT, Vec::new(), Vec::new()).expect("meter"),
    ));
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let meter = meter.clone();
            scope.spawn(move || {
                for _ in 0..16 {
                    let _ = meter.lock().expect("lock").reserve().expect("reservation");
                }
            });
        }
    });
    let held = meter.lock().expect("lock");
    assert_eq!(held.admitted, 64);
    assert_eq!(held.rejected, 64);
    let rows: Vec<serde_json::Value> = String::from_utf8(held.journal.clone())
        .expect("utf8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("row"))
        .collect();
    let ids: Vec<_> = rows
        .iter()
        .filter(|r| r["event"] == "admit")
        .map(|r| r["attempt"].as_u64().expect("id"))
        .collect();
    assert_eq!(ids, (1..=64).collect::<Vec<_>>());
}

struct Broken;
impl Write for Broken {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(error("write failed"))
    }
    fn flush(&mut self) -> io::Result<()> {
        Err(error("flush failed"))
    }
}

#[test]
fn journal_and_request_capture_fail_closed_and_existing_files_are_untouched() {
    assert!(Meter::new(1, CAPTURE_LIMIT, Vec::new(), Broken).is_err());
    let mut meter = Meter::new(1, CAPTURE_LIMIT, Broken, Vec::new()).expect("meter");
    assert_eq!(meter.reserve().expect("reserve"), Some(1));
    assert!(meter.reserve_capture(1, 2).expect("capture reservation"));
    assert!(meter.capture(1, "{}").is_err());
    let world = World::new();
    let path = world.path("record");
    let file = private_file(&path).expect("new");
    assert_eq!(
        file.metadata().expect("metadata").permissions().mode() & 0o777,
        0o600
    );
    std::fs::write(&path, "keep").expect("existing content");
    assert!(private_file(&path).is_err());
    symlink(&path, world.path("link")).expect("link");
    assert!(private_file(&world.path("link")).is_err());
    assert_eq!(std::fs::read_to_string(path).expect("content"), "keep");
}

async fn server(
    world: &World,
    limit: u64,
    script: serde_json::Value,
) -> (
    u16,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<io::Result<()>>,
) {
    capped_server(world, limit, CAPTURE_LIMIT, script).await
}

async fn capped_server(
    world: &World,
    limit: u64,
    capture_limit: usize,
    script: serde_json::Value,
) -> (
    u16,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<io::Result<()>>,
) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("listener");
    let port = listener.local_addr().expect("address").port();
    let turns: Vec<Turn> = serde_json::from_value(script).expect("script");
    for turn in &turns {
        turn.validate().expect("valid turn");
    }
    let meter = Arc::new(Mutex::new(
        Meter::new(
            limit,
            capture_limit,
            private_file(&world.path("record")).expect("record"),
            private_file(&world.path("meter")).expect("meter"),
        )
        .expect("state"),
    ));
    let (send, recv) = tokio::sync::oneshot::channel();
    let handle = tokio::spawn(serve(listener, Arc::new(turns), meter, async {
        let _ = recv.await;
    }));
    (port, send, handle)
}

async fn exchange(port: u16, body: &str) -> io::Result<String> {
    let mut socket = TcpStream::connect(("127.0.0.1", port)).await?;
    socket.write_all(format!("POST /chat/completions HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{body}", body.len()).as_bytes()).await?;
    let mut response = Vec::new();
    socket.read_to_end(&mut response).await?;
    String::from_utf8(response).map_err(|_| error("response utf8"))
}

async fn joined(handle: tokio::task::JoinHandle<io::Result<()>>) -> io::Result<()> {
    tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("server bounded completion")
        .expect("server task")
}

fn repeat() -> serde_json::Value {
    json!([{"when":{"body":"model"},"events":[{"text":"ok"},{"finish":"stop"}]}])
}

#[tokio::test]
async fn rendezvous_waiters_cannot_hide_shutdown_or_attempt_overrun() {
    for excess in [false, true] {
        let world = World::new();
        let script = json!([
            {"rendezvous":"pair","events":[{"text":"released"},{"finish":"stop"}]},
            {"rendezvous":"pair","events":[{"text":"released"},{"finish":"stop"}]}
        ]);
        let (port, stop, handle) = server(&world, 1, script).await;
        let first = tokio::spawn(exchange(port, "{}"));
        wait_event(&world, "rendezvous_arrive").await;
        if excess {
            assert!(
                exchange(port, "{}")
                    .await
                    .expect("overrun")
                    .starts_with("HTTP/1.1 429")
            );
        } else {
            stop.send(()).expect("stop");
        }
        assert!(joined(handle).await.is_err());
        assert!(
            !first
                .await
                .expect("client")
                .unwrap_or_default()
                .contains("released")
        );
        let rows = world.journal();
        let last = rows.last().expect("terminal");
        assert_eq!(last["status"], "FAIL");
        assert_eq!(last["responses"], 0);
        assert_eq!(last["incomplete"], 1);
        assert_eq!(
            last["reason"],
            if excess {
                "attempt_limit"
            } else {
                "shutdown_with_incomplete_requests"
            }
        );
        assert!(!rows.iter().any(|r| r["event"] == "rendezvous_release"));
        assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
    }
}

#[tokio::test]
async fn helpers_children_and_retries_share_one_cap_and_excess_is_terminal() {
    let world = World::new();
    let (port, _stop, handle) = server(&world, 3, repeat()).await;
    for body in [
        r#"{"model":"main","tools":[{}]}"#,
        r#"{"model":"helper","tools":[]}"#,
        r#"{"model":"child","tools":[{},{}]}"#,
    ] {
        assert!(
            exchange(port, body)
                .await
                .expect("response")
                .starts_with("HTTP/1.1 200")
        );
    }
    let refused = exchange(port, r#"{"model":"main","retry":true}"#)
        .await
        .expect("refusal");
    assert!(refused.starts_with("HTTP/1.1 429"));
    assert!(refused.contains("evaluation_attempt_limit"));
    assert!(joined(handle).await.is_err());
    let rows = world.journal();
    let last = rows.last().expect("terminal");
    assert_eq!(last["event"], "terminal");
    assert_eq!(last["status"], "FAIL");
    assert_eq!(last["reason"], "attempt_limit");
    assert_eq!(last["admitted"], 3);
    assert_eq!(last["rejected"], 1);
    assert_eq!(last["responses"], 3);
    assert_eq!(last["incomplete"], 0);
    assert_eq!(last["reserved_bytes"], last["captured_bytes"]);
    assert_eq!(
        std::fs::read_to_string(world.path("record"))
            .expect("capture")
            .lines()
            .count(),
        3
    );
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
}

#[tokio::test]
async fn probes_are_free_and_exact_limit_can_stop_with_complete_capture() {
    let world = World::new();
    let (port, stop, handle) = server(&world, 1, repeat()).await;
    drop(
        TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("probe"),
    );
    let body = r#"{"model":"one"}"#;
    assert!(exchange(port, body).await.expect("response").contains("ok"));
    stop.send(()).expect("stop");
    joined(handle).await.expect("clean stop");
    let rows = world.journal();
    assert_eq!(rows.iter().filter(|r| r["event"] == "admit").count(), 1);
    let captured = rows.iter().find(|r| r["event"] == "body").expect("body");
    assert_eq!(
        captured["sha256"],
        format!("{:x}", Sha256::digest(body.as_bytes()))
    );
    assert_eq!(rows.last().expect("terminal")["status"], "STOPPED");
}

#[tokio::test]
async fn malformed_request_reserves_before_parsing_and_aborts_the_server() {
    let world = World::new();
    let (port, _stop, handle) = server(&world, 2, repeat()).await;
    let mut socket = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect");
    socket
        .write_all(b"POST / HTTP/1.1\r\n")
        .await
        .expect("partial");
    socket.shutdown().await.expect("close write");
    assert!(joined(handle).await.is_err());
    let rows = world.journal();
    assert_eq!(rows.last().expect("terminal")["admitted"], 1);
    assert_eq!(rows.last().expect("terminal")["incomplete"], 1);
}

#[tokio::test]
async fn concurrent_excess_aborts_held_responses_without_extra_script_execution() {
    let world = World::new();
    let script = json!([{"when":{"body":"model"},"events":[{"pause":30000},{"finish":"stop"}]}]);
    let (port, _stop, handle) = server(&world, 1, script).await;
    let mut held = TcpStream::connect(("127.0.0.1", port)).await.expect("held");
    held.write_all(b"POST / HTTP/1.1\r\nContent-Length: 13\r\n\r\n{\"model\":\"a\"}")
        .await
        .expect("request");
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if world.journal().iter().any(|r| r["event"] == "body") {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("admitted request");
    let _ = exchange(port, r#"{"model":"excess"}"#).await;
    assert!(joined(handle).await.is_err());
    let rows = world.journal();
    assert_eq!(rows.last().expect("terminal")["admitted"], 1);
    assert_eq!(rows.last().expect("terminal")["incomplete"], 1);
    assert_eq!(rows.iter().filter(|r| r["event"] == "response").count(), 0);
    let mut response = Vec::new();
    tokio::time::timeout(Duration::from_secs(1), held.read_to_end(&mut response))
        .await
        .expect("held socket closed")
        .expect("read");
}

#[tokio::test]
async fn scripted_refusal_consumes_budget_without_making_transport_server_fail() {
    let world = World::new();
    let (port, stop, handle) = server(
        &world,
        1,
        json!([{"refuse":{"status":400,"message":"output budget"}}]),
    )
    .await;
    assert!(
        exchange(port, "{}")
            .await
            .expect("refusal")
            .starts_with("HTTP/1.1 400")
    );
    stop.send(()).expect("stop");
    joined(handle).await.expect("scripted refusal is served");
    assert_eq!(world.journal().last().expect("terminal")["responses"], 1);
}

#[tokio::test]
async fn shutdown_cannot_hide_an_already_recorded_overrun() {
    let world = World::new();
    let (port, stop, handle) = server(&world, 1, repeat()).await;
    exchange(port, r#"{"model":"one"}"#)
        .await
        .expect("admitted");
    let mut excess = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("excess");
    excess.write_all(b"P").await.expect("first byte");
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if world.journal().iter().any(|r| r["event"] == "reject") {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("rejection recorded");
    stop.send(()).expect("stop");
    assert!(joined(handle).await.is_err());
    let rows = world.journal();
    assert_eq!(rows.last().expect("terminal")["status"], "FAIL");
    assert_eq!(rows.last().expect("terminal")["reason"], "attempt_limit");
}

#[tokio::test]
async fn a_meter_write_failure_terminates_before_capture_or_script_execution() {
    let world = World::new();
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("listener");
    let port = listener.local_addr().expect("address").port();
    let mut meter = Meter::new(
        1,
        CAPTURE_LIMIT,
        private_file(&world.path("record")).expect("record"),
        private_file(&world.path("meter")).expect("meter"),
    )
    .expect("state");
    meter.journal = File::open(world.path("meter")).expect("read-only journal");
    let turns: Vec<Turn> = serde_json::from_value(repeat()).expect("script");
    let handle = tokio::spawn(serve(
        listener,
        Arc::new(turns),
        Arc::new(Mutex::new(meter)),
        std::future::pending(),
    ));
    let response = exchange(port, r#"{"model":"one"}"#)
        .await
        .unwrap_or_default();
    assert!(!response.contains("fixture-ok") && !response.contains("200 Fake"));
    assert!(joined(handle).await.is_err());
    assert_eq!(world.journal().len(), 1);
    assert_eq!(
        std::fs::metadata(world.path("record"))
            .expect("capture")
            .len(),
        0
    );
}

async fn wait_event(world: &World, event: &str) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if world.journal().iter().any(|row| row["event"] == event) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("event recorded");
}

#[tokio::test]
async fn excessive_declared_body_is_refused_without_receiving_or_capturing_it() {
    let world = World::new();
    let (port, _stop, handle) = capped_server(&world, 2, 32, repeat()).await;
    let mut socket = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("socket");
    socket
        .write_all(b"POST / HTTP/1.1\r\nContent-Length: 8388608\r\n\r\n")
        .await
        .expect("header only");
    let mut response = String::new();
    tokio::time::timeout(Duration::from_secs(2), socket.read_to_string(&mut response))
        .await
        .expect("bounded refusal")
        .expect("read");
    assert!(response.starts_with("HTTP/1.1 413") && response.contains("evaluation_capture_limit"));
    assert!(joined(handle).await.is_err());
    assert_eq!(
        std::fs::metadata(world.path("record"))
            .expect("record")
            .len(),
        0
    );
    let rows = world.journal();
    let last = rows.last().expect("terminal");
    assert_eq!(last["reason"], "capture_limit");
    assert_eq!(last["reserved_bytes"], 0);
    assert_eq!(last["responses"], 0);
    assert_eq!(last["incomplete"], 1);
}

#[tokio::test]
async fn partial_body_reservations_cannot_be_reused_by_another_connection() {
    let world = World::new();
    let (port, _stop, handle) = capped_server(&world, 2, 8, repeat()).await;
    let mut held = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("socket");
    held.write_all(b"POST / HTTP/1.1\r\nContent-Length: 7\r\n\r\n")
        .await
        .expect("header only");
    wait_event(&world, "reserve_capture").await;
    assert!(
        exchange(port, "")
            .await
            .expect("refusal")
            .starts_with("HTTP/1.1 413")
    );
    assert!(joined(handle).await.is_err());
    let rows = world.journal();
    let last = rows.last().expect("terminal");
    assert_eq!(last["reserved_bytes"], 8);
    assert_eq!(last["captured_bytes"], 0);
    assert_eq!(last["incomplete"], 2);
    assert_eq!(last["responses"], 0);
    let mut rest = Vec::new();
    tokio::time::timeout(Duration::from_secs(1), held.read_to_end(&mut rest))
        .await
        .expect("closed")
        .expect("read");
}

#[tokio::test]
async fn shutdown_cannot_hide_a_recorded_capture_limit() {
    let world = World::new();
    let (port, stop, handle) = capped_server(&world, 2, 1, repeat()).await;
    let mut socket = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("socket");
    socket
        .write_all(b"POST / HTTP/1.1\r\nContent-Length: 1\r\n\r\n")
        .await
        .expect("header");
    wait_event(&world, "reject_capture").await;
    stop.send(()).expect("stop");
    assert!(joined(handle).await.is_err());
    let rows = world.journal();
    let last = rows.last().expect("terminal");
    assert_eq!(last["reason"], "capture_limit");
    assert_eq!(last["status"], "FAIL");
}
