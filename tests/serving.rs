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

    /// Start one as a child: with a parent named and a secret handed down.
    ///
    /// The environment a harness would use after `melchior fork` — which is the whole point of
    /// this pair of variables and the whole of what was never produced.
    fn under(runtime: &std::path::Path, id: &str, parent: &str, token: &str) -> Self {
        std::fs::create_dir_all(runtime).expect("mkdir");
        let mut child = Command::new(env!("CARGO_BIN_EXE_melchior"))
            .arg("serve")
            .env("MAGI_MELCHIOR_PROJECT", "demo")
            .env("MAGI_MELCHIOR_ID", id)
            .env("MAGI_MELCHIOR_PARENT", parent)
            .env("MAGI_MELCHIOR_TOKEN", token)
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
        assert_eq!(said["event"], "listening", "{first}");
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
        assert_eq!(said["event"], "listening", "{first}");
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
            if said["event"] == kind {
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
    assert_eq!(heard["event"], "message");
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

    serving.told(r#"{"event":"doing","busy":true,"working_for":7,"waiting":0}"#);
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
    serving.told(r#"{"event":"doing","busy":true}"#);
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

/// The subagent lattice, end to end, against the real binaries.
///
/// **Every piece of this existed and none of it was connected.** `parent()` and `token()` read
/// `MAGI_MELCHIOR_PARENT` and `MAGI_MELCHIOR_TOKEN`; `announce` writes the note that makes the
/// tree readable off the directory; `children` reads it back; `stop` refuses anything a session
/// did not start. All of it correct, and all of it inert, because nothing minted a secret or
/// handed a name down — so a harness that spawned a child got a *main*: no parent, outside every
/// wall the policy draws, and unstoppable by the thing that started it.
#[test]
fn a_forked_session_comes_up_as_a_child_and_its_parent_may_end_it() {
    let mut parent = Serving::start("fork");
    let runtime = parent.runtime();

    // What a harness asks for before it spawns. Over argv, to this session's own socket.
    let out = Command::new(env!("CARGO_BIN_EXE_melchior"))
        .arg("fork")
        .env("MAGI_MELCHIOR_PROJECT", "demo")
        .env("MAGI_MELCHIOR_ID", "alpha-rho")
        .env("XDG_RUNTIME_DIR", &runtime)
        .output()
        .expect("melchior fork runs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let minted: serde_json::Value = serde_json::from_slice(&out.stdout).expect("one JSON object");
    let id = minted["id"].as_str().expect("a name").to_owned();
    let token = minted["token"].as_str().expect("a secret").to_owned();

    // The harness spawns, with exactly what it was handed.
    let mut child = Serving::under(&runtime, &id, "alpha-rho", &token);

    // **A child, not a main.** This is the whole property: it knows whose it is, and it says so
    // to anybody who asks — read off the note it wrote, never from what it claims about itself.
    let said = ask(
        &child.at(),
        r#"{"call":"identity","from":"demo/main/alpha-rho"}"#,
    );
    assert_eq!(said["result"][0]["parent"], "alpha-rho", "{said}");
    assert_eq!(said["result"][0]["main"], false, "{said}");

    // And its parent may end it, because its parent is the only party that knows the secret.
    let wrong = ask(
        &child.at(),
        r#"{"call":"stop","from":"demo/main/alpha-rho","token":"not-the-secret"}"#,
    );
    assert_eq!(
        wrong["ok"], false,
        "a guess does not end a session: {wrong}"
    );

    let right = ask(
        &child.at(),
        &format!(r#"{{"call":"stop","from":"demo/main/alpha-rho","token":"{token}"}}"#),
    );
    assert_eq!(right["ok"], true, "the secret does: {right}");

    let _ = child.child.kill();
    let _ = parent.child.kill();
    let _ = std::fs::remove_dir_all(&runtime);
}

/// One hand-written call, framed the way the family frames everything.
fn ask(at: &std::path::Path, body: &str) -> serde_json::Value {
    let mut sock = std::os::unix::net::UnixStream::connect(at).expect("connected");
    sock.set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .expect("a timeout");
    let mut frame = u32::try_from(body.len())
        .expect("fits")
        .to_be_bytes()
        .to_vec();
    frame.extend_from_slice(body.as_bytes());
    sock.write_all(&frame).expect("wrote");
    let mut header = [0_u8; 4];
    sock.read_exact(&mut header).expect("read a header");
    let mut answer = vec![0_u8; u32::from_be_bytes(header) as usize];
    sock.read_exact(&mut answer).expect("read a body");
    serde_json::from_slice(&answer).expect("it is JSON")
}

/// A parent that starts one `melchior serve` and then does nothing at all.
///
/// A shell rather than this process: the parent has to be something the test can kill outright,
/// and killing the test runner is not available. Its stdin is a pipe *this* process holds the
/// other end of, handed down explicitly — a background job in a non-interactive shell is given
/// `/dev/null` otherwise, and stdin staying open is the whole point of the exercise.
struct Killable {
    shell: Child,
    /// The write end of the pipe `serve` is reading, kept open on purpose. While this is held,
    /// end of file cannot be what stops it, so the kernel is the only explanation left.
    held: Option<std::process::ChildStdin>,
    served: u32,
    runtime: std::path::PathBuf,
}

impl Killable {
    fn start(name: &str) -> Self {
        // Short, for the same reason `Serving::start` is: a unix socket path is capped at about
        // a hundred bytes and a long prefix silently exhausts it.
        let runtime =
            std::path::PathBuf::from(format!("/tmp/melchior-k-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&runtime);
        std::fs::create_dir_all(&runtime).expect("mkdir");
        let pids = runtime.join("pid");
        let script = format!(
            "exec 3<&0; XDG_RUNTIME_DIR={runtime} {binary} serve --project killed <&3 \
             >/dev/null 2>&1 & echo $! > {pids}; wait",
            runtime = runtime.display(),
            binary = env!("CARGO_BIN_EXE_melchior"),
            pids = pids.display(),
        );
        let mut shell = Command::new("sh")
            .arg("-c")
            .arg(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start the caller");
        // Taken out before anything waits on the shell: `Child::wait` drops its own stdin, and
        // dropping this one would close the pipe and end `serve` for the ordinary reason.
        let held = shell.stdin.take();
        let served = read_pid(&pids).expect("the caller said which melchior it started");
        Self {
            shell,
            held,
            served,
            runtime,
        }
    }

    /// End the caller the way a crash would: with nothing running inside it.
    fn killed(&mut self) {
        let _ = self.shell.kill();
        let _ = self.shell.wait();
    }

    /// Leave nothing running and nothing on disk, whatever the assertions are about to do.
    fn cleared(mut self) {
        let _ = self.shell.kill();
        let _ = self.shell.wait();
        drop(self.held.take());
        let _ = Command::new("kill")
            .arg("-9")
            .arg(self.served.to_string())
            // Already gone is the passing case, and its complaint reads like a failure.
            .stderr(Stdio::null())
            .status();
        let _ = std::fs::remove_dir_all(&self.runtime);
    }
}

/// The pid the shell wrote down, once it has written it.
fn read_pid(at: &std::path::Path) -> Option<u32> {
    for _ in 0..250 {
        if let Ok(text) = std::fs::read_to_string(at)
            && let Ok(pid) = text.trim().parse::<u32>()
        {
            return Some(pid);
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    None
}

/// Whether a process exists and is not merely a corpse waiting to be reaped.
///
/// The state field rather than the directory's existence: every process here is started by a
/// shell that is about to be killed, so a zombie is the expected shape of "gone".
fn alive(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .is_ok_and(|stat| stat.split_whitespace().nth(2) != Some("Z"))
}

/// Wait for `pid` to go away, and say whether it did.
fn gone_within(pid: u32, patience: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + patience;
    while std::time::Instant::now() < deadline {
        if !alive(pid) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    !alive(pid)
}

#[test]
fn nothing_outlives_a_parent_that_was_killed_outright() {
    // The case `nothing_outlives_the_parent` cannot reach. That one lets go of the pipe, which
    // is a parent with a way out; this one is the panic, the `kill -9` and the OOM, where
    // nothing in the parent runs at all and the pipe is never closed by anybody. The write end
    // is still held here while the assertion runs, so end of file is not available as an
    // explanation and `PR_SET_PDEATHSIG` is the only thing left that could have done it.
    let mut caller = Killable::start("killed");
    let served = caller.served;
    assert!(
        alive(served),
        "it is up while the parent that started it is"
    );

    caller.killed();
    let went = gone_within(served, std::time::Duration::from_secs(10));

    caller.cleared();
    assert!(
        went,
        "melchior serve must not outlive the process that started it"
    );
}
