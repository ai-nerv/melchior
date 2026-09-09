//! What the command line answers when it is asked for something it does not have.
//!
//! Against the real binary rather than the function behind it: the rule FAMILY.md states is about
//! what a caller sees — stdout, the reply shape, exit 0 — and an argument parser breaks it above
//! the level any unit test reaches.

use std::process::Command;

/// Run the binary and hand back what it put on stdout and what it exited with.
fn ran(args: &[&str]) -> (String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_melchior"))
        .args(args)
        .output()
        .expect("melchior runs");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

fn parsed(said: &str) -> serde_json::Value {
    serde_json::from_str(said).unwrap_or_else(|why| panic!("not the reply shape: {why}: {said}"))
}

#[test]
fn an_unknown_verb_is_a_reply_on_stdout_at_exit_zero() {
    let (said, status) = ran(&["definitely-not-a-verb-xyzzy"]);
    assert_eq!(status, 0, "a refusal is a reply: {said}");
    let reply = parsed(&said);
    assert_eq!(reply["ok"], serde_json::json!(false));
    assert_eq!(reply["family"], serde_json::json!(melchior::wire::FAMILY));
    assert_eq!(reply["n"], serde_json::json!(0));
    assert!(reply["result"].is_array(), "result is a list: {said}");
    assert_eq!(reply["fault"], serde_json::json!("refused"));
    assert!(
        reply["error"]
            .as_str()
            .unwrap_or_default()
            .contains("definitely-not-a-verb-xyzzy"),
        "it names what was asked for: {said}"
    );
}

#[test]
fn an_unknown_verb_answers_in_cbor_too() {
    let out = Command::new(env!("CARGO_BIN_EXE_melchior"))
        .args(["definitely-not-a-verb-xyzzy", "--cbor"])
        .output()
        .expect("melchior runs");
    assert_eq!(out.status.code(), Some(0));
    let reply: serde_json::Value =
        ciborium::from_reader(out.stdout.as_slice()).expect("a cbor map");
    assert_eq!(reply["ok"], serde_json::json!(false));
    assert_eq!(reply["fault"], serde_json::json!("refused"));
}

#[test]
fn the_usage_a_person_asks_for_is_not_a_refusal() {
    let (said, status) = ran(&["--help"]);
    assert_eq!(status, 0);
    assert!(said.starts_with("usage: melchior"), "{said}");
}

#[test]
fn verbs_carries_the_surface_a_plugin_writes_against() {
    let (said, _) = ran(&["verbs"]);
    let reply = parsed(&said);
    assert_eq!(reply["surface"], serde_json::json!(melchior::wire::SURFACE));
    assert!(
        reply["result"][0]["verb"].is_string(),
        "a row is a value, not the list: {said}"
    );
}

#[test]
fn client_is_source_bare_and_framed_when_an_encoding_is_named() {
    let (source, status) = ran(&["client"]);
    assert_eq!(status, 0);
    assert!(source.starts_with("--"), "bare, it is Lua source");

    let (framed, _) = ran(&["client", "--json"]);
    let reply = parsed(&framed);
    assert_eq!(reply["n"], serde_json::json!(1));
    assert_eq!(reply["result"][0].as_str(), Some(source.as_str()));
}
