//! `melchior serve`, driven the way a harness drives it.
//!
//! Everything else about the layer is tested against itself. This runs the real binary, with a
//! real pipe and a real socket, because the two things it has to get right are only true at
//! that level: what crosses the pipe, and that nothing outlives the parent.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};

/// A running `melchior serve`, and the runtime directory it is alone in.
struct Serving {
    child: Child,
    out: BufReader<std::process::ChildStdout>,
    runtime: std::path::PathBuf,
    /// Where it said it was listening, rather than where a test guessed it would be.
    at: std::path::PathBuf,
    /// What it said it is called, for addressing it and for checking who a message came from.
    named: String,
}

impl Serving {
    /// Start one alone in its own runtime directory.
    fn start(name: &str) -> Self {
        // Short, because a unix socket path is capped at about a hundred bytes and a temp
        // directory under a long prefix silently exhausts it.
        let runtime =
            std::path::PathBuf::from(format!("/tmp/melchior-t-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&runtime);
        Self::beside(&runtime, "alpha-rho")
    }

    /// Start one in a runtime directory somebody else may already be in.
    ///
    /// What a conversation needs: two sessions that can see each other. `start` gives each its
    /// own directory — the project wall doing its job — which means two of them are, by
    /// construction, unable to say anything to one another.
    fn beside(runtime: &std::path::Path, id: &str) -> Self {
        std::fs::create_dir_all(runtime).expect("mkdir");
        let mut child = Command::new(env!("CARGO_BIN_EXE_melchior"))
            .arg("serve")
            .env("MAGI_MELCHIOR_PROJECT", "demo")
            .env("MAGI_MELCHIOR_ID", id)
            .env("XDG_RUNTIME_DIR", runtime)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("melchior serve");

        let mut out = BufReader::new(child.stdout.take().expect("stdout"));
        let mut first = String::new();
        out.read_line(&mut first).expect("it says something");
        let said: serde_json::Value = serde_json::from_str(&first).expect("one JSON object");
        assert_eq!(said["heard"], "listening", "{first}");
        let at = std::path::PathBuf::from(said["at"].as_str().expect("it says where"));
        let named = said["as"].as_str().expect("it says who").to_owned();
        Self {
            child,
            out,
            runtime: runtime.to_path_buf(),
            at,
            named,
        }
    }

    /// What it calls itself, as `project/role/id`.
    fn named(&self) -> String {
        self.named.clone()
    }

    /// The runtime directory it listens in, for putting a second session beside it.
    fn runtime(&self) -> std::path::PathBuf {
        self.runtime.clone()
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
        panic!("melchior serve outlived the parent that started it");
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

/// Two sessions holding a conversation: `ask`, then `reply`, through the tool a model calls.
///
/// The verb the whole thing turns on. `reply` was a stub for as long as the layer existed, so
/// two agents could open a conversation and never continue one — and the way it presented was
/// the model saying "reply is not wired, sending instead" and falling back to a note, which
/// wakes nobody. One exchange, then silence, and nothing in the harness to point at.
#[test]
fn an_ask_and_a_reply_are_a_conversation() {
    let mut asker = Serving::start("asker");
    let answerer = Serving::beside(&asker.runtime(), "beta-nu");
    let (me, them) = (asker.named(), answerer.named());
    let runtime = asker.runtime();

    let asked = tool(
        &runtime,
        &me,
        &[
            "--verb=ask",
            &format!("--who={}", id_of(&them)),
            "--message=which file?",
        ],
    );
    assert!(asked.status.success(), "{}", stderr(&asked));

    // The id is the authority on who asked, so the answerer reads it back out of its own inbox
    // rather than being told: an id it invented would answer somebody who never asked.
    let listed = stdout(&tool(&runtime, &them, &["--verb=inbox"]));
    assert!(listed.contains("[question]"), "{listed}");
    let about = listed
        .split("about: \"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("the inbox names the id to quote")
        .to_owned();

    // No `who`: it goes to whoever asked, worked out from the message being quoted.
    let replied = tool(
        &runtime,
        &them,
        &[
            "--verb=reply",
            &format!("--about={about}"),
            "--message=command.rs",
        ],
    );
    assert!(replied.status.success(), "{}", stderr(&replied));

    let heard = asker.heard("message");
    assert_eq!(heard["who"], them);
    assert_eq!(heard["text"], "command.rs");
    assert_eq!(
        heard["sort"], "answer",
        "a reply that travelled as a note would reach the asker's transcript and never be read"
    );
    assert_eq!(heard["about"], about, "it must quote what it answers");
}

#[test]
fn an_id_that_names_nothing_is_refused_rather_than_sent_to_somebody() {
    // The failure worth guarding: an invented id resolving to whoever happens to be first in
    // the inbox would put an answer in front of a session that never asked.
    let answerer = Serving::start("stray");
    let refused = tool(
        &answerer.runtime(),
        &answerer.named(),
        &["--verb=reply", "--about=made-up", "--message=x"],
    );
    assert!(!refused.status.success());
    assert!(
        stderr(&refused).contains("nothing has been sent"),
        "{}",
        stderr(&refused)
    );
}

/// One `melchior tool` call, as the session `named`, in the directory it is listening in.
fn tool(runtime: &std::path::Path, named: &str, args: &[&str]) -> std::process::Output {
    let mut parts = named.split('/');
    Command::new(env!("CARGO_BIN_EXE_melchior"))
        .arg("tool")
        .args(args)
        .env("XDG_RUNTIME_DIR", runtime)
        .env("MAGI_MELCHIOR_PROJECT", parts.next().unwrap_or_default())
        .env("MAGI_MELCHIOR_ROLE", parts.next().unwrap_or_default())
        .env("MAGI_MELCHIOR_ID", parts.next().unwrap_or_default())
        .output()
        .expect("melchior tool runs")
}

fn stdout(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// The last segment, which is what a sibling is addressed by inside one project.
fn id_of(named: &str) -> &str {
    named.rsplit('/').next().unwrap_or_default()
}

#[test]
fn a_session_that_is_gone_leaves_the_roster_and_the_directory() {
    // The leftovers. `listening` promised in its own doc comment that a socket nothing answers
    // is discovered rather than trusted, and then listed filenames — so every session that
    // crashed, and every one from a build that named its socket differently, stayed in the
    // roster for good. A model was offered names nobody answered and found out one failed send
    // at a time.
    let alive = Serving::start("sweep");
    let runtime = alive.runtime();
    let project = runtime.join("melchior").join("demo");

    // A corpse of each kind: a plain file where a socket would be, and the note beside it.
    std::fs::write(project.join("zeta-mu"), b"").expect("a dead socket");
    std::fs::write(project.join("zeta-mu.parent"), b"demo/main/alpha-rho").expect("its note");

    let gone = Serving::beside(&runtime, "iota-phi");
    let seen = stdout(&tool(&runtime, &gone.named(), &["--verb=list"]));
    assert!(
        seen.contains("alpha-rho"),
        "the live one is missing: {seen}"
    );
    assert!(
        !seen.contains("zeta-mu"),
        "a session nothing answers is still being offered: {seen}"
    );
    assert!(
        !project.join("zeta-mu").exists() && !project.join("zeta-mu.parent").exists(),
        "the corpse was listed out but not swept"
    );

    // And when the last of them goes, so does the directory.
    drop(gone);
    assert!(alive.let_go());
    assert!(
        !project.exists(),
        "the project directory outlived every session in it"
    );
}
