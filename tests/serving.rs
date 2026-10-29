//! `melchior serve`, driven the way a harness drives it.
//!
//! Everything else about the layer is tested against itself. This runs the real binary, with a
//! real pipe and a real socket, because the two things it has to get right are only true at
//! that level: what crosses the pipe, and that nothing outlives the parent.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

mod fixture;

use fixture::{Serving, ask, id_of, left_in, stderr, stdout, tool};
use std::process::Command;

#[test]
fn run_identity_changes_across_recycled_server_processes() {
    let runtime = melchior::scratch::Scratch::new("melchior-t", "run-recycle");
    let mut runs = std::collections::BTreeSet::new();
    for _ in 0..32 {
        let serving = Serving::spawned(
            &runtime,
            &[
                ("MAGI_MELCHIOR_ID", "alpha-rho"),
                ("MAGI_MELCHIOR_SESSION", ""),
                ("MAGI_SESSION", ""),
            ],
            false,
        );
        let note = serving.at().with_extension("session");
        let run = std::fs::read_to_string(&note).expect("run note");
        let (agent, nonce) = run.rsplit_once('-').expect("run suffix");
        assert_eq!(agent, "alpha-rho");
        assert_eq!(nonce.len(), 39);
        assert!(nonce.bytes().all(|c| c.is_ascii_digit()));
        assert!(runs.insert(run), "new process reused a previous run");
        assert!(serving.let_go());
        assert!(!note.exists());
    }
}

#[test]
fn run_identity_preserves_inherited_legacy_and_nonce_ids() {
    let runtime = melchior::scratch::Scratch::new("melchior-t", "run-inherit");
    for run in [
        "alpha-rho-1700000000",
        "alpha-rho-340282366920938463463374607431768211455",
    ] {
        let serving = Serving::spawned(
            &runtime,
            &[
                ("MAGI_MELCHIOR_ID", "psi-eta"),
                ("MAGI_MELCHIOR_SESSION", run),
            ],
            false,
        );
        let note = serving.at().with_extension("session");
        assert_eq!(std::fs::read_to_string(&note).expect("inherited run"), run);
        assert!(serving.let_go());
        assert!(!note.exists());
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
    // From a process whose environment names `socat`: who a message is from is read there, once,
    // and never off the frame.
    let mut serving = Serving::start("message");
    let sent = tool(
        &serving.runtime(),
        "demo/main/socat",
        &[
            "--verb=attention",
            "--who=alpha-rho",
            "--message=build is green",
        ],
    );
    assert!(sent.status.success(), "{}", stderr(&sent));

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

    serving.told(r#"{"event":"doing","busy":true,"working_for":7,"waiting":0,"spent":[{"model":"a/b","cost_micros":150}]}"#);
    // A moment for the reader thread; the next call is a fresh connection either way.
    std::thread::sleep(std::time::Duration::from_millis(300));

    let status = serving.asked(r#"{"call":"status","from":"demo/main/socat"}"#);
    assert_eq!(status["result"][0]["busy"], true, "{status}");
    assert_eq!(status["result"][0]["working_for"], 7);
    assert_eq!(
        status["result"][0]["spent"][0]["cost_micros"], 150,
        "{status}"
    );
    assert!(serving.let_go());
}

#[test]
fn a_listing_answers_with_its_rows_rather_than_one_row_that_is_the_listing() {
    // The failure FAMILY.md names by name: `"result":[[…]]` with `"n":1`, invisible from the
    // sending side, and read by a coordinator going row by row as an array where a declaration
    // belonged. Over a real socket, in a runtime the client's VM may dial.
    let room = melchior::scratch::Project::new("melchior-t", "rows");
    let serving = Serving::beside(&room.home(), "alpha-rho");
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

    // The client this session serves: N rows reach Lua as N return values, gathered or not.
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
/// `melchior fork` names a child and mints its secret; the harness starts it with the environment
/// it was handed and gives the secret to its `serve` on its stdin; the child comes up a child, and
/// the session that started it is the one that can end it.
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
    assert!(
        !minted["environment"].to_string().contains(&token),
        "the secret rides in the child's environment: {minted}"
    );

    // The harness spawns, with exactly what it was handed.
    let child = Serving::under(&runtime, &id, "alpha-rho", &token);

    // **A child, not a main.** This is the whole property: it knows whose it is, and it says so
    // to anybody who asks — read off the note it wrote, never from what it claims about itself.
    let said = ask(
        &child.at(),
        r#"{"call":"identity","from":"demo/main/alpha-rho"}"#,
    );
    assert_eq!(said["result"][0]["parent"], "alpha-rho", "{said}");
    assert_eq!(said["result"][0]["main"], false, "{said}");

    // And its parent may end it, because its parent is the only party that holds the secret: a
    // process this test started names itself `alpha-rho` and asks `alpha-rho`'s own socket for it.
    let wrong = ask(
        &child.at(),
        r#"{"call":"stop","from":"demo/main/alpha-rho","token":"not-the-secret"}"#,
    );
    assert_eq!(
        wrong["ok"], false,
        "a guess does not end a session: {wrong}"
    );

    let right = tool(
        &runtime,
        "demo/main/alpha-rho",
        &["--verb=stop", &format!("--who={id}")],
    );
    assert!(
        right.status.success(),
        "the secret does: {}",
        stderr(&right)
    );
    let mut child = child;
    child.heard("stopped");

    let _ = child.child.kill();
    let _ = parent.child.kill();
}

#[test]
fn an_empty_runtime_directory_is_unset_and_the_fallback_is_this_user_s_alone() {
    use std::io::BufRead;
    use std::os::unix::fs::MetadataExt;
    let tmp = melchior::scratch::Scratch::new("melchior-t", "empty");
    let mut child = Command::new(env!("CARGO_BIN_EXE_melchior"))
        .arg("serve")
        .env("MAGI_MELCHIOR_PROJECT", "demo")
        .env("MAGI_MELCHIOR_ID", "alpha-rho")
        .env("XDG_RUNTIME_DIR", "")
        .env("TMPDIR", &*tmp)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("melchior serve");
    let mut first = String::new();
    std::io::BufReader::new(child.stdout.take().expect("stdout"))
        .read_line(&mut first)
        .expect("it says where");
    drop(child.stdin.take());
    let _ = child.wait();

    let said: serde_json::Value = serde_json::from_str(&first).expect("one JSON object");
    let base = tmp.join(format!("melchior-{}", melchior::places::uid()));
    let at = said["at"].as_str().expect("where");
    assert!(at.starts_with(&*base.to_string_lossy()), "{at}");
    let mode = std::fs::metadata(&base).expect("the fallback").mode();
    assert_eq!(mode & 0o777, 0o700, "{mode:o}");
}
