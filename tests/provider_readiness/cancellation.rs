use super::*;
use melchior::mind::wire::Ask;
use std::io::{BufRead, BufReader};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(10);

struct Held {
    stop: Arc<AtomicBool>,
    closed: mpsc::Receiver<()>,
    task: Option<std::thread::JoinHandle<(String, Value)>>,
}

impl Held {
    fn new(listener: TcpListener) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let done = Arc::clone(&stop);
        let (sender, closed) = mpsc::channel();
        let task = std::thread::spawn(move || {
            let deadline = Instant::now() + WAIT;
            let mut socket = loop {
                assert!(
                    !done.load(Ordering::SeqCst) && Instant::now() < deadline,
                    "bounded accept"
                );
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2))
                    }
                    Err(error) => panic!("accept: {error}"),
                }
            };
            socket.set_read_timeout(Some(WAIT)).expect("read bound");
            socket.set_write_timeout(Some(WAIT)).expect("write bound");
            let mut bytes = Vec::new();
            while !bytes.ends_with(b"\r\n\r\n") {
                assert!(bytes.len() < 65536);
                let mut byte = [0];
                socket.read_exact(&mut byte).expect("header");
                bytes.push(byte[0]);
            }
            let headers = String::from_utf8(bytes)
                .expect("UTF-8")
                .to_ascii_lowercase();
            let length: usize = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .expect("length")
                .trim()
                .parse()
                .expect("integer");
            assert!(length < 65536);
            let mut bytes = vec![0; length];
            socket.read_exact(&mut bytes).expect("body");
            let body: Value = serde_json::from_slice(&bytes).expect("JSON");
            assert!(!body.to_string().contains("private-sentinel"));
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").expect("response");
            let event = "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"partial\"}}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":7,\"prompt_tokens_details\":{\"cached_tokens\":40}}}\n\n";
            write!(socket, "{:x}\r\n{event}\r\n", event.len()).expect("chunk");
            socket.flush().expect("flush");
            socket
                .set_read_timeout(Some(Duration::from_millis(100)))
                .expect("hold bound");
            let deadline = Instant::now() + WAIT;
            loop {
                assert!(
                    !done.load(Ordering::SeqCst) && Instant::now() < deadline,
                    "provider did not observe cancellation"
                );
                match socket.read(&mut [0]) {
                    Ok(0) => break,
                    Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => break,
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) => {}
                    value => panic!("unexpected input: {value:?}"),
                }
            }
            sender.send(()).expect("cancellation observer");
            (headers, body)
        });
        Self {
            stop,
            closed,
            task: Some(task),
        }
    }

    fn finish(&mut self) -> (String, Value) {
        self.closed.recv_timeout(WAIT).expect("connection closed");
        self.task
            .take()
            .expect("task")
            .join()
            .expect("server joined")
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(task) = self.task.take() {
            let _ = task.join();
        }
    }
}

struct Asking {
    child: std::process::Child,
    events: mpsc::Receiver<Value>,
    reader: Option<std::thread::JoinHandle<()>>,
}

impl Asking {
    fn new(dir: &Scratch, key: &str, ask: &Ask) -> Self {
        let mut child = command(dir)
            .env(key, "private-sentinel")
            .args(["ask", "--json"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("ask");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(&serde_json::to_vec(ask).expect("ask JSON"))
            .expect("request");
        let stdout = child.stdout.take().expect("stdout");
        let (sender, events) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let line = line.expect("line");
                assert!(!line.contains("private-sentinel"));
                if sender
                    .send(serde_json::from_str(&line).expect("event"))
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            child,
            events,
            reader: Some(reader),
        }
    }

    fn cancel(&mut self) {
        assert!(self.child.try_wait().expect("status").is_none());
        self.child.kill().expect("cancel owned ask");
        assert!(!self.child.wait().expect("reap").success());
        self.reader
            .take()
            .expect("reader")
            .join()
            .expect("reader joined");
        let mut stderr = String::new();
        self.child
            .stderr
            .take()
            .expect("stderr")
            .read_to_string(&mut stderr)
            .expect("stderr read");
        assert!(!stderr.contains("private-sentinel"));
    }
}

impl Drop for Asking {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

pub(super) fn after_usage(
    dir: &Scratch,
    listener: &TcpListener,
    key: &str,
    ask: &Ask,
) -> (String, Value) {
    let mut server = Held::new(listener.try_clone().expect("listener"));
    let mut asking = Asking::new(dir, key, ask);
    let mut spent = false;
    for _ in 0..16 {
        let event = asking
            .events
            .recv_timeout(WAIT)
            .expect("bounded event wait");
        assert!(!matches!(
            event["event"].as_str(),
            Some("stop" | "failed" | "retrying")
        ));
        if event["event"] == "spent" {
            assert_eq!(
                (
                    event["usage"]["input"].as_u64(),
                    event["usage"]["output"].as_u64(),
                    event["usage"]["cache_read"].as_u64()
                ),
                (Some(60), Some(7), Some(40))
            );
            spent = true;
            break;
        }
    }
    assert!(spent);
    asking.cancel();
    let request = server.finish();
    assert!(asking.events.try_iter().all(|event| !matches!(
        event["event"].as_str(),
        Some("stop" | "failed" | "retrying")
    )));
    presets::unasked(listener);
    request
}
