//! `atom serve`, driven the way a harness drives it.
//!
//! Everything else about the layer is tested against itself. This runs the real binary, with a
//! real pipe and a real socket, because the two things it has to get right are only true at
//! that level: what crosses the pipe, and that nothing outlives the parent.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};

/// A running `atom serve`, and the runtime directory it is alone in.
struct Serving {
    child: Child,
    out: BufReader<std::process::ChildStdout>,
    runtime: std::path::PathBuf,
    /// Where it said it was listening, rather than where a test guessed it would be.
    at: std::path::PathBuf,
}

impl Serving {
    /// Start one, and wait until it says it is listening.
    fn start(name: &str) -> Self {
        // Short, because a unix socket path is capped at about a hundred bytes and a temp
        // directory under a long prefix silently exhausts it.
        let runtime =
            std::path::PathBuf::from(format!("/tmp/atom-t-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&runtime);
        std::fs::create_dir_all(&runtime).expect("mkdir");

        let mut child = Command::new(env!("CARGO_BIN_EXE_atom"))
            .arg("serve")
            .env("ATOM_PROJECT", "demo")
            .env("ATOM_ID", "alpha-rho")
            .env("XDG_RUNTIME_DIR", &runtime)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("atom serve");

        let mut out = BufReader::new(child.stdout.take().expect("stdout"));
        let mut first = String::new();
        out.read_line(&mut first).expect("it says something");
        let said: serde_json::Value = serde_json::from_str(&first).expect("one JSON object");
        assert_eq!(said["heard"], "listening", "{first}");
        let at = std::path::PathBuf::from(said["at"].as_str().expect("it says where"));
        Self {
            child,
            out,
            runtime,
            at,
        }
    }

    /// Where it said it is listening.
    fn at(&self) -> std::path::PathBuf {
        self.at.clone()
    }

    /// Tell it what the session is doing.
    fn told(&mut self, line: &str) {
        let stdin = self.child.stdin.as_mut().expect("stdin");
        writeln!(stdin, "{line}").expect("wrote");
        stdin.flush().expect("flushed");
    }

    /// Read until it says something of `kind`.
    ///
    /// Skipping the rest, because a parent does: `around` arrives on its own schedule and a
    /// reader that took "the next line" as the answer to its own question would read a roster
    /// as a message the moment the two happened to cross.
    fn heard(&mut self, kind: &str) -> serde_json::Value {
        for _ in 0..64 {
            let mut line = String::new();
            self.out.read_line(&mut line).expect("it says something");
            let said: serde_json::Value = serde_json::from_str(&line).expect("one JSON object");
            if said["heard"] == kind {
                return said;
            }
        }
        panic!("nothing of kind {kind} came back");
    }

    /// One call over the socket, framed by hand, the way a sibling would.
    fn asked(&self, body: &str) -> serde_json::Value {
        let mut sock = std::os::unix::net::UnixStream::connect(self.at()).expect("connected");
        sock.set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .expect("a timeout");
        let mut frame = u32::try_from(body.len())
            .expect("fits")
            .to_be_bytes()
            .to_vec();
        frame.extend_from_slice(body.as_bytes());
        sock.write_all(&frame).expect("wrote");
        sock.shutdown(std::net::Shutdown::Write).expect("hung up");
        let mut back = Vec::new();
        sock.read_to_end(&mut back).expect("read");
        serde_json::from_slice(&back[4..]).expect("it is JSON")
    }

    /// Let go of the pipe, and wait.
    fn let_go(mut self) -> bool {
        drop(self.child.stdin.take());
        for _ in 0..100 {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                let left = self.at().exists();
                let _ = std::fs::remove_dir_all(&self.runtime);
                return !left;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = std::fs::remove_dir_all(&self.runtime);
        panic!("atom serve outlived the parent that started it");
    }
}

#[test]
fn it_says_where_it_is_listening_before_anything_else() {
    // A parent that spawned this has no other way to know the socket exists, and one that
    // started sending before the bind would meet its own session as "nothing is listening".
    let serving = Serving::start("bound");
    assert!(serving.at().exists());
    assert!(serving.let_go());
}

#[test]
fn a_message_from_a_sibling_comes_up_the_pipe() {
    // The whole reason there is a pipe. The socket is where a message lands and the harness is
    // where it has to end up, and those are two processes.
    let mut serving = Serving::start("message");
    let reply = serving.asked(
        r#"{"call":"tell","args":["build is green","attention",null],"from":"demo/main/socat"}"#,
    );
    assert_eq!(reply["ok"], true, "{reply}");

    let heard = serving.heard("message");
    assert_eq!(heard["heard"], "message");
    assert_eq!(heard["who"], "demo/main/socat");
    assert_eq!(heard["sort"], "attention");
    assert_eq!(heard["text"], "build is green");
    assert!(serving.let_go());
}

#[test]
fn what_the_parent_says_it_is_doing_is_what_a_sibling_is_told() {
    // The other direction, and the only thing that travels it. Without it `status` would answer
    // plausibly rather than truthfully: this process cannot see a turn running.
    let mut serving = Serving::start("doing");
    assert_eq!(
        serving.asked(r#"{"call":"status","from":"demo/main/socat"}"#)["result"][0]["busy"],
        false
    );

    serving.told(r#"{"say":"doing","busy":true,"working_for":7,"waiting":0}"#);
    // Given to the reader thread and through the channel; a moment, not a race worth a retry
    // loop, because the next call is a fresh connection either way.
    std::thread::sleep(std::time::Duration::from_millis(300));

    let status = serving.asked(r#"{"call":"status","from":"demo/main/socat"}"#);
    assert_eq!(status["result"][0]["busy"], true, "{status}");
    assert_eq!(status["result"][0]["working_for"], 7);
    assert!(serving.let_go());
}

#[test]
fn nothing_outlives_the_parent() {
    // The promise the whole shape rests on. It was broken once and looked fine: a `select!` arm
    // whose pattern does not match is disabled rather than taken, so the closed pipe dropped
    // that branch and the loop went on waiting on the socket forever.
    let serving = Serving::start("lifetime");
    let at = serving.at();
    assert!(at.exists(), "it never bound");
    assert!(
        serving.let_go(),
        "the socket outlived the session it belonged to"
    );
}

#[test]
fn a_line_it_cannot_read_does_not_stop_it_answering() {
    // The parent's bug is not a reason to stop answering a socket other sessions are using.
    let mut serving = Serving::start("garbage");
    serving.told("this is not json");
    serving.told(r#"{"say":"doing","busy":true}"#);
    std::thread::sleep(std::time::Duration::from_millis(300));

    let status = serving.asked(r#"{"call":"status","from":"demo/main/socat"}"#);
    assert_eq!(status["result"][0]["busy"], true, "{status}");
    assert!(serving.let_go());
}
