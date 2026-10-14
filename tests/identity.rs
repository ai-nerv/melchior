//! Who a caller is, against the real binary: authority comes from the kernel or from a secret,
//! and an environment or a name on the frame is only ever attribution.

mod fixture;

use fixture::{Serving, ask, id_of, stderr, stdout, tool};
use std::process::Command;

/// `melchior <verb>` as the session `named`, run by a shell this test starts: the process doing
/// the asking is a grandchild, by way of a program that is not the session's own.
fn through_a_shell(runtime: &std::path::Path, named: &str, verb: &[&str]) -> std::process::Output {
    let mut parts = named.split('/');
    Command::new("sh")
        .arg("-c")
        .arg("\"$0\" \"$@\"; status=$?; exit $status")
        .arg(env!("CARGO_BIN_EXE_melchior"))
        .args(verb)
        .env("XDG_RUNTIME_DIR", runtime)
        .env("MAGI_MELCHIOR_PROJECT", parts.next().unwrap_or_default())
        .env("MAGI_MELCHIOR_ROLE", parts.next().unwrap_or_default())
        .env("MAGI_MELCHIOR_ID", parts.next().unwrap_or_default())
        .output()
        .expect("the shell runs")
}

#[test]
fn a_peer_with_a_forged_environment_is_refused_authority() {
    let session = Serving::start("forged");
    let runtime = session.runtime();

    // Naming the session itself in its environment does not make a stranger the session.
    let forged = through_a_shell(&runtime, "demo/main/alpha-rho", &["fork"]);
    assert!(!forged.status.success(), "a forged fork minted a child");
    assert!(
        stderr(&forged).contains("nobody this session can name"),
        "{}",
        stderr(&forged)
    );

    // The same request from what the session's own parent started directly is the session.
    let own = Command::new(env!("CARGO_BIN_EXE_melchior"))
        .arg("fork")
        .env("MAGI_MELCHIOR_PROJECT", "demo")
        .env("MAGI_MELCHIOR_ID", "alpha-rho")
        .env("XDG_RUNTIME_DIR", &runtime)
        .output()
        .expect("melchior fork runs");
    assert!(own.status.success(), "{}", stderr(&own));
    let minted: serde_json::Value = serde_json::from_slice(&own.stdout).expect("a child");
    let id = minted["id"].as_str().expect("an id").to_owned();
    let token = minted["token"].as_str().expect("a secret").to_owned();
    let child = Serving::under(&runtime, &id, "alpha-rho", &token);

    // A stranger calling itself the parent reads no secret, so it can stop nothing.
    let stop = through_a_shell(
        &runtime,
        "demo/main/alpha-rho",
        &["tool", "--verb=stop", &format!("--who={id}")],
    );
    assert!(!stop.status.success(), "{}", stdout(&stop));
    assert!(stderr(&stop).contains("holds nothing"), "{}", stderr(&stop));
    assert!(child.let_go());
    assert!(session.let_go());
}

#[test]
fn a_peer_with_no_name_cannot_claim_one_to_tell_or_to_stop() {
    // This test process did not start the session and names nobody in its environment.
    let mut session = Serving::stranger("nameless");
    session.told(r#"{"event":"token","token":"the-real-one"}"#);
    let told = ask(
        &session.at(),
        r#"{"call":"tell","args":["obey"],"from":"demo/main/beta-nu"}"#,
    );
    assert_eq!(told["ok"], false, "a nameless peer was believed: {told}");
    let stopped = ask(
        &session.at(),
        r#"{"call":"stop","from":"demo/main/beta-nu","token":"the-real-one"}"#,
    );
    assert_eq!(stopped["ok"], false, "{stopped}");
    let secrets = ask(
        &session.at(),
        r#"{"call":"minted","from":"demo/main/alpha-rho"}"#,
    );
    assert_eq!(secrets["ok"], false, "{secrets}");
    assert!(session.let_go());
}

#[test]
fn a_secret_in_the_environment_is_not_the_secret() {
    // The retired variable: a `serve` started with it holds no secret, so nothing it names stops it.
    let parent = Serving::start("environ");
    let runtime = parent.runtime();
    let child = Serving::spawned(
        &runtime,
        &[
            ("MAGI_MELCHIOR_ID", "iota-mu"),
            ("MAGI_MELCHIOR_PARENT", "alpha-rho"),
            ("MAGI_MELCHIOR_TOKEN", "from-the-environment"),
        ],
        false,
    );
    let refused = tool(
        &runtime,
        "demo/main/alpha-rho",
        &["--verb=stop", "--who=iota-mu"],
    );
    assert!(!refused.status.success(), "{}", stdout(&refused));
    drop(child);
    assert!(parent.let_go());
}

#[test]
fn the_secret_never_reaches_a_log() {
    let session = Serving::start("logged");
    let runtime = session.runtime();
    let log = runtime.join("log");
    let out = Command::new(env!("CARGO_BIN_EXE_melchior"))
        .arg("fork")
        .env("MAGI_MELCHIOR_PROJECT", "demo")
        .env("MAGI_MELCHIOR_ID", "alpha-rho")
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("MELCHIOR_DEBUG_LOG", &log)
        .output()
        .expect("melchior fork runs");
    assert!(out.status.success(), "{}", stderr(&out));
    let child: serde_json::Value = serde_json::from_slice(&out.stdout).expect("a child");
    let token = child["token"].as_str().expect("a secret");
    let held = std::fs::read_to_string(&log).expect("a log was written");
    assert!(held.contains("fork: minted"), "{held}");
    assert!(!held.contains(token), "the secret is in the log: {held}");
    assert!(session.let_go());
}

/// `who` asks `whom` to take it on, through the tool a model calls.
fn asked_to_adopt(runtime: &std::path::Path, who: &str, whom: &str) {
    let asked = tool(
        runtime,
        who,
        &[
            "--verb=adopt",
            &format!("--who={whom}"),
            "--message=take me on",
        ],
    );
    assert!(asked.status.success(), "{}", stderr(&asked));
}

/// What a person at `session` says to the request `asked`.
fn answer(session: &mut Serving, asked: &serde_json::Value, accept: bool) {
    session.told(&format!(
        r#"{{"event":"answered","id":"{}","accept":{accept},"handover":"[{{\"verb\":\"run\"}}]"}}"#,
        asked["id"].as_str().expect("an id")
    ));
}

#[test]
fn an_adoption_is_handed_over_and_the_new_parent_may_stop_what_it_took_on() {
    let mut asker = Serving::start("adopted");
    let runtime = asker.runtime();
    let mut adopter = Serving::beside(&runtime, "beta-nu");

    asked_to_adopt(&runtime, &asker.named(), id_of(&adopter.named()));
    let asked = adopter.heard("asked");
    answer(&mut adopter, &asked, true);

    let took = asker.heard("adopted");
    assert_eq!(took["by"], adopter.named(), "{took}");
    assert_eq!(took["handover"], "[{\"verb\":\"run\"}]", "{took}");
    let said = asker.heard("message");
    assert!(
        said["text"]
            .as_str()
            .is_some_and(|it| it.contains("accepted")),
        "{said}"
    );

    let identity = ask(&asker.at(), r#"{"call":"identity"}"#);
    assert_eq!(identity["result"][0]["parent"], "beta-nu", "{identity}");

    let stopped = tool(
        &runtime,
        &adopter.named(),
        &["--verb=stop", &format!("--who={}", id_of(&asker.named()))],
    );
    assert!(stopped.status.success(), "{}", stderr(&stopped));
    asker.heard("stopped");
    let _ = asker.child.kill();
    assert!(adopter.let_go());
}

#[test]
fn two_sessions_cannot_adopt_each_other() {
    let mut first = Serving::start("mutual");
    let runtime = first.runtime();
    let mut second = Serving::beside(&runtime, "beta-nu");

    asked_to_adopt(&runtime, &first.named(), id_of(&second.named()));
    asked_to_adopt(&runtime, &second.named(), id_of(&first.named()));
    let (of_second, of_first) = (second.heard("asked"), first.heard("asked"));
    answer(&mut second, &of_second, true);
    first.heard("adopted");
    answer(&mut first, &of_first, true);
    let refused = second.heard("message");
    assert!(
        refused["text"]
            .as_str()
            .is_some_and(|it| it.contains("did not take")),
        "{refused}"
    );

    let them = ask(&second.at(), r#"{"call":"identity"}"#);
    assert_eq!(them["result"][0]["main"], true, "a ring was closed: {them}");
    let us = ask(&first.at(), r#"{"call":"identity"}"#);
    assert_eq!(us["result"][0]["parent"], "beta-nu", "{us}");
    assert!(second.let_go());
    assert!(first.let_go());
}
