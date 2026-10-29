use super::*;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;
use std::time::Duration;

const WAIT: Duration = Duration::from_secs(10);

struct HeldProvider {
    port: u16,
    asked: Arc<AtomicUsize>,
    closed: Receiver<()>,
    stop: Arc<AtomicBool>,
    task: Option<JoinHandle<()>>,
}

impl HeldProvider {
    fn new(reported: bool) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("local provider");
        let port = listener.local_addr().expect("provider port").port();
        listener.set_nonblocking(true).expect("bounded accept");
        let asked = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (sender, closed) = mpsc::channel();
        let (count, done) = (Arc::clone(&asked), Arc::clone(&stop));
        let task = std::thread::spawn(move || {
            while !done.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        count.fetch_add(1, Ordering::SeqCst);
                        held(stream, reported, &done);
                        let _ = sender.send(());
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("provider accept: {error}"),
                }
            }
        });
        Self {
            port,
            asked,
            closed,
            stop,
            task: Some(task),
        }
    }
}

impl Drop for HeldProvider {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(task) = self.task.take() {
            let result = task.join();
            if !std::thread::panicking() {
                result.expect("provider joined");
            }
        }
    }
}

fn held(stream: TcpStream, reported: bool, stop: &AtomicBool) {
    stream
        .set_read_timeout(Some(WAIT))
        .expect("request timeout");
    stream
        .set_write_timeout(Some(WAIT))
        .expect("response timeout");
    let mut reader = BufReader::new(stream);
    let mut header = Vec::new();
    loop {
        let mut byte = [0];
        reader.read_exact(&mut byte).expect("request header");
        header.push(byte[0]);
        assert!(header.len() <= 65536, "bounded request header");
        if header.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let header = String::from_utf8(header).expect("HTTP header");
    let length = header
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .and_then(|value| value.trim().parse::<usize>().ok())
        })
        .expect("body length");
    assert!(length <= 65536, "bounded request body");
    reader
        .read_exact(&mut vec![0; length])
        .expect("request body");
    let mut usage = serde_json::json!({"prompt_tokens":1,"completion_tokens":1});
    if reported {
        usage["cost"] = serde_json::json!(0.25);
    }
    let event = format!("data: {}\n\n", serde_json::json!({"usage":usage}));
    let stream = reader.get_mut();
    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").expect("response header");
    for _ in 0..2 {
        write!(stream, "{:x}\r\n{event}\r\n", event.len()).expect("paid usage");
    }
    stream.flush().expect("usage flushed");
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .expect("bounded hold");
    while !stop.load(Ordering::SeqCst) {
        match reader.read(&mut [0]) {
            Ok(0) => return,
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => return,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            result => panic!("unexpected held response input: {result:?}"),
        }
    }
}

struct Asking {
    child: Child,
    lines: Receiver<serde_json::Value>,
    reader: Option<JoinHandle<()>>,
}

impl Asking {
    fn new(world: &World, envs: &[(&str, &str)]) -> Self {
        let mut child = world.asking(envs);
        let stdout = child.stdout.take().expect("ask stdout");
        let (sender, lines) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let event = serde_json::from_str(&line.expect("stream line")).expect("JSON event");
                if sender.send(event).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            lines,
            reader: Some(reader),
        }
    }
}

impl Drop for Asking {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let result = reader.join();
            if !std::thread::panicking() {
                result.expect("reader joined");
            }
        }
    }
}

#[test]
fn cancelled_paid_asks_keep_their_charge_and_refuse_the_next_capped_request() {
    for reported in [false, true] {
        let provider = HeldProvider::new(reported);
        let world = World::at("cancelled-paid", provider.port, Arc::clone(&provider.asked));
        let (charge, cost, estimate, cap) = if reported {
            (250_000, 250_000, 0, "0.25")
        } else {
            (2_000_000, 0, 2_000_000, "2")
        };
        world.capped(&format!("melchior.spend_cap_usd={cap}"));
        let envs = [("MAGI_MELCHIOR_SESSION", "cancelled-paid-run")];
        let mut asking = Asking::new(&world, &envs);
        let mut spent = 0;
        for _ in 0..16 {
            let event = asking.lines.recv_timeout(WAIT).expect("bounded usage wait");
            assert_ne!(event["event"], "stop");
            assert_ne!(event["event"], "failed");
            assert_ne!(event["event"], "retrying");
            if event["event"] == "spent" {
                assert_eq!(event["usage"]["input"], 1);
                assert_eq!(event["usage"]["output"], 1);
                assert_eq!(event["usage"]["cost_micros"], cost);
                assert_eq!(event["usage"]["estimated_micros"], estimate);
                spent += 1;
                if spent == 2 {
                    break;
                }
            }
        }
        assert_eq!(spent, 2, "duplicate paid events observed");
        assert_eq!(world.ledger("cancelled-paid-run").spent(), charge);
        assert!(asking.child.try_wait().expect("active ask").is_none());
        asking.child.kill().expect("cancel actual ask");
        assert!(!asking.child.wait().expect("cancelled ask reaped").success());
        drop(asking);
        provider
            .closed
            .recv_timeout(WAIT)
            .expect("provider saw cancellation");
        assert_eq!(world.ledger("cancelled-paid-run").spent(), charge);
        let mut next = Asking::new(&world, &envs);
        let refusal = next.lines.recv_timeout(WAIT).expect("capped refusal");
        assert_eq!(refusal["event"], "failed", "{refusal}");
        assert!(refusal["message"].as_str().expect("reason").contains("cap"));
        assert!(next.child.wait().expect("refused ask reaped").success());
        assert_eq!(
            world.asked(),
            1,
            "no retry or post-cancellation capped request"
        );
        assert_eq!(world.ledger("cancelled-paid-run").spent(), charge);
    }
}
