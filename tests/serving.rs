//! `melchior serve`, driven the way a harness drives it.
//!
//! Everything else about the layer is tested against itself. This runs the real binary, with a
//! real pipe and a real socket, because the two things it has to get right are only true at
//! that level: what crosses the pipe, and that nothing outlives the parent.

use melchior::scratch::Scratch;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};

/// A running `melchior serve`, and the runtime directory it is alone in.
struct Serving {
    child: Child,
    out: BufReader<std::process::ChildStdout>,
    /// The directory it listens in, owned when this session is the one that made it.
    ///
    /// **`Some` for exactly one session per directory.** [`Serving::beside`] and
    /// [`Serving::under`] start a second one in somebody else's, and a guard each would mean the
    /// first to finish deleting the socket the other is still answering on.
    own: Option<Scratch>,
    runtime: std::path::PathBuf,
    /// Where it said it was listening, rather than where a test guessed it would be.
    at: std::path::PathBuf,
    /// What it said it is called, for addressing it and for checking who a message came from.
    named: String,
}

impl Serving {
    /// Start one alone in its own runtime directory.
    ///
    /// **Under `$TMPDIR`, through a guard that removes it on the unwind.** This used to name
    /// `/tmp/melchior-t-<pid>-<name>` outright and remove it on the last line of the test, which
    /// is wrong twice over: a failing test kept its directory for good, and a literal path is
    /// one `gate-hermetic` cannot see — the gate runs the suite under a `TMPDIR` of its own and
    /// looks there, so these two leaked past it on every green run it ever reported.
    ///
    /// Short still matters: a unix socket path is capped at `SUN_LEN`, and the gate roots its
    /// own directory at `/tmp` for that reason.
    fn start(name: &str) -> Self {
        let own = Scratch::new("melchior-t", name);
        let mut serving = Self::beside(&own, "alpha-rho");
        serving.own = Some(own);
        serving
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
            own: None,
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
            own: None,
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

    /// End it the way the kernel does, and wait for it to go.
    ///
    /// `kill -TERM` rather than [`std::process::Child::kill`], which sends `SIGKILL` — the one
    /// death nothing inside the process can be asked to tidy up after, and so the one death this
    /// cannot be about.
    fn signalled(&mut self) -> bool {
        let _ = Command::new("kill")
            .arg("-TERM")
            .arg(self.child.id().to_string())
            .status();
        for _ in 0..200 {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        false
    }

    /// Let go of the pipe, and wait.
    ///
    /// The directory goes with `self`, on the return and on the panic alike.
    fn let_go(mut self) -> bool {
        drop(self.child.stdin.take());
        for _ in 0..100 {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return !self.at().exists();
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let _ = self.child.kill();
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
fn a_listing_answers_with_its_rows_rather_than_one_row_that_is_the_listing() {
    // The failure FAMILY.md names by name: `"result":[[…]]` with `"n":1`, invisible from the
    // sending side, and read by a coordinator going row by row as an array where a declaration
    // belonged. Over a real socket, because the command line is the only door the gate probes.
    let serving = Serving::start("rows");
    let said = r#"{"call":"tell","from":"demo/main/socat","args":["build is green"]}"#;
    serving.asked(said);
    serving.asked(&said.replace("build is green", "and the deploy went out"));

    for verb in ["verbs", "needs", "inbox"] {
        let reply = serving.asked(&format!(r#"{{"call":"{verb}","from":"demo/main/socat"}}"#));
        assert_eq!(reply["ok"], true, "{verb}: {reply}");
        let rows = reply["result"].as_array().expect("result is a list");
        assert_eq!(
            reply["n"].as_u64().expect("a count"),
            rows.len() as u64,
            "{verb} says how many came back and then sends a different number: {reply}"
        );
        assert!(rows.len() > 1, "{verb} has more than one of them: {reply}");
        assert!(
            !rows.iter().any(serde_json::Value::is_array),
            "{verb} wrapped its whole listing in one row: {reply}"
        );
    }

    // And the counts a re-wrapping would flatten to 1.
    let listed = serving.asked(r#"{"call":"verbs","from":"demo/main/socat"}"#);
    assert_eq!(listed["n"], melchior::wire::VERBS.len(), "{listed}");
    assert_eq!(listed["result"][0]["door"], "socket", "{listed}");
    let waiting = serving.asked(r#"{"call":"inbox","from":"demo/main/socat"}"#);
    assert_eq!(waiting["n"], 2, "two messages, two rows: {waiting}");

    // A record is still one row, and a map of id to secret is a record.
    let held = serving.asked(r#"{"call":"status","from":"demo/main/socat"}"#);
    assert_eq!(held["n"], 1, "a record is one row: {held}");

    // The other half of the same commit: the client this session serves. N rows reach Lua as N
    // return values, so a client that did not gather them would hand its caller one verb.
    let mut engine = melchior::mind::lua::engine::Engine::new();
    let chunk = format!(
        "local it, why = load([=====[\n{source}]=====])(melchior.stream)\
         .connect({{ path = {at:?}, timeout_ms = 5000 }})\n\
         assert(it, tostring(why))\n\
         it.from = \"demo/main/socat\"\n\
         melchior.verbs = #it.verbs()\n\
         melchior.needs = #it.needs()\n\
         local waiting = it.inbox()\n\
         melchior.inbox = #waiting\n\
         melchior.first = tostring(waiting[1] and waiting[1].text)\n",
        source = melchior::CLIENT,
        at = serving.at().display().to_string(),
    );
    engine.run(&chunk, "gather.lua").expect("the client loads");
    engine.harvest();
    let config = engine.config();
    assert_eq!(
        config.number("verbs"),
        Some(melchior::wire::VERBS.len() as f64),
        "the client gathered the listing back into one table"
    );
    assert!(config.number("needs").unwrap_or_default() > 1.0);
    assert_eq!(config.number("inbox"), Some(2.0));
    assert_eq!(config.string("first"), Some("build is green"));
    drop(engine);

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

/// What is in a project's directory, by name, whether or not the directory is still there.
fn left_in(project: &std::path::Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(project) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn a_session_ended_by_a_signal_leaves_the_directory_as_it_found_it() {
    // The exit path was written, correct, and unreachable. `serve` takes its socket and its notes
    // back down under its loop, and a signal ends a process without running any of that — so a
    // melchior that died the way melchiors actually die, on the `SIGTERM` the kernel sends when
    // the magi that started it is killed, had never once run those lines.
    let mut serving = Serving::start("signalled");
    let project = serving.runtime().join("melchior").join("demo");
    assert!(serving.at().exists(), "it never bound");

    assert!(serving.signalled(), "it did not end on SIGTERM");
    assert!(
        left_in(&project).is_empty(),
        "a signal left these behind: {:?}",
        left_in(&project)
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
    /// Kept for its `Drop`, which removes the directory however the test ends.
    _runtime: Scratch,
}

impl Killable {
    fn start(name: &str) -> Self {
        // Under `$TMPDIR`, for the same reason `Serving::start` is: this named
        // `/tmp/melchior-k-<pid>-<name>` outright, which escapes the isolated root
        // `gate-hermetic` runs the suite under and so leaked past it unremarked.
        let runtime = Scratch::new("melchior-k", name);
        let pids = runtime.join("pid");
        // `--ui`, so there is a fourth file to leave behind. Without it a session writes its
        // socket, its `.role` and its `.session`, and the case the field reported — four files
        // per session, the screen note among them — is one the fixture could not reproduce.
        let script = format!(
            "exec 3<&0; XDG_RUNTIME_DIR={runtime} {binary} serve --project killed \
             --ui {runtime}/screen.sock <&3 >/dev/null 2>&1 & echo $! > {pids}; wait",
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
        let it = Self {
            shell,
            held,
            served,
            _runtime: runtime,
        };
        let home = it.home();
        if !settled_in(&home, std::time::Duration::from_secs(10)) {
            let left = left_in(&home);
            it.cleared();
            panic!("the session never came up: {left:?}");
        }
        it
    }

    /// Where this session's socket and the notes beside it are.
    fn home(&self) -> std::path::PathBuf {
        self._runtime.join("melchior").join("killed")
    }

    /// End the caller the way a crash would: with nothing running inside it.
    fn killed(&mut self) {
        let _ = self.shell.kill();
        let _ = self.shell.wait();
    }

    /// Leave nothing running, whatever the assertions are about to do.
    ///
    /// The directory is not this function's job any more: it goes when `self` does.
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
    }
}

/// Wait for the session the shell started to come up, and say whether it did.
///
/// The pid is written the moment the shell forks, and `serve` writes its notes and binds its
/// socket well after — so listing the directory on the next line found it empty on roughly one
/// loaded run in three, and a kill in that window landed before the child had asked the kernel
/// to end it with its parent. Polled: the `listening` line belongs to the shell.
fn settled_in(project: &std::path::Path, patience: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + patience;
    loop {
        // The notes go down before the socket binds, so the socket says it is all there.
        let names = left_in(project);
        if names.iter().any(|it| !it.contains('.')) && names.iter().any(|it| it.ends_with(".ui")) {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
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

#[test]
fn a_killed_parent_leaves_none_of_its_sessions_notes_behind() {
    // The case the field reported, and the reason the exit path being correct was not enough:
    // a headless magi taken down with `kill -9`, its melchior ended by the kernel's `SIGTERM`,
    // and four files still in the directory afterwards — the socket, and the `.ui`, `.role` and
    // `.session` notes beside it. Every sibling that listed the project was then offered a name
    // that answers nothing until something else came along and swept it.
    let mut caller = Killable::start("swept");
    let home = caller.home();
    let bound = left_in(&home);
    assert!(
        bound.iter().any(|name| name.ends_with(".ui")),
        "the fixture is not the reported case: {bound:?}"
    );

    caller.killed();
    let went = gone_within(caller.served, std::time::Duration::from_secs(10));
    // Read after it is gone rather than on a timer: the unlinking is the last thing it does, so
    // the process being gone is what makes this listing an answer rather than a race.
    let left = left_in(&home);

    caller.cleared();
    assert!(went, "melchior serve outlived the parent that started it");
    assert!(
        left.is_empty(),
        "the kernel's signal ended it and these stayed: {left:?}"
    );
}
