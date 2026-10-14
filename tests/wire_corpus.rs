//! The family's wire corpus: every server case sent to a real `melchior serve`, and every client
//! case served to both of melchior's clients, [`melchior::asking::Held`] and the shipped Lua
//! library, each answered or taken as `cases.json` says.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

mod fixture;

use fixture::Serving;
use melchior::asking::Held;
use melchior::framing::{self, Wire};
use melchior::mind::lua::engine::Engine;
use melchior::scratch::{Project, Scratch};
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wire-corpus")
}

fn cases() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(corpus().join("cases.json")).expect("cases"))
        .expect("parse")
}

fn bytes_of(case: &serde_json::Value) -> (String, Vec<u8>) {
    let file = case["file"].as_str().expect("file").to_owned();
    let bytes = std::fs::read(corpus().join(&file)).expect("case");
    (file, bytes)
}

/// One whole frame, or `None` when the far end closed first.
fn reply(stream: &mut UnixStream) -> Option<Vec<u8>> {
    let mut header = [0_u8; 4];
    stream.read_exact(&mut header).ok()?;
    let mut body = vec![0_u8; u32::from_be_bytes(header) as usize];
    stream.read_exact(&mut body).ok()?;
    Some(body)
}

fn decoded(body: &[u8]) -> (serde_json::Value, &'static str) {
    let value: serde_json::Value = Wire::read_any(body).expect("a reply in either encoding");
    let encoding = match Wire::of(body) {
        Wire::Cbor => "cbor",
        Wire::Json => "json",
    };
    (value, encoding)
}

fn send(stream: &mut UnixStream, bytes: &[u8], bytewise: bool) {
    if !bytewise {
        stream.write_all(bytes).expect("write");
        return;
    }
    for byte in bytes {
        stream.write_all(&[*byte]).expect("write");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Every server case against `serving`, with a `verbs` after each that says it stays open.
fn served_as_the_corpus_says(serving: &Serving) {
    let verbs = std::fs::read(corpus().join("server/verbs-json.frame")).expect("verbs");
    for case in cases()["server"].as_array().expect("server cases") {
        let (file, bytes) = bytes_of(case);
        let mut stream = UnixStream::connect(serving.at()).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("a deadline");
        send(&mut stream, &bytes, case["split"] == "bytewise");
        let body = reply(&mut stream).unwrap_or_else(|| panic!("{file}: no answer"));
        let (said, encoding) = decoded(&body);
        let expect = &case["expect"];
        assert_eq!(said["ok"], expect["ok"], "{file}: {said}");
        if let Some(wanted) = expect["encoding"].as_str() {
            assert_eq!(encoding, wanted, "{file}");
        }
        if let Some(fault) = expect["fault"].as_str() {
            assert_eq!(said["fault"], fault, "{file}: {said}");
        }
        if let Some(part) = expect["error_contains"].as_str() {
            let error = said["error"].as_str().unwrap_or_default();
            assert!(error.contains(part), "{file}: {said}");
        }
        match case["then"].as_str() {
            Some("open") => {
                send(&mut stream, &verbs, false);
                let next = reply(&mut stream).unwrap_or_else(|| panic!("{file}: closed"));
                assert_eq!(decoded(&next).0["ok"], true, "{file}: out of step after");
            }
            Some("closed") => assert!(reply(&mut stream).is_none(), "{file}: left open"),
            _ => {}
        }
    }
}

#[test]
fn every_server_case_is_answered_to_the_session_s_own() {
    let serving = Serving::start("wc-own");
    served_as_the_corpus_says(&serving);
    assert!(serving.let_go());
}

#[test]
fn every_server_case_is_answered_to_a_stranger() {
    let serving = Serving::stranger("wc-far");
    served_as_the_corpus_says(&serving);
    assert!(serving.let_go());
}

#[test]
fn the_corpus_names_the_cap_this_program_reads_at() {
    assert_eq!(cases()["cap"].as_u64(), u64::try_from(framing::MOST).ok());
}

/// Answer the first call on `listening` with `case`, then hang up or hold the line until the
/// client lets go.
fn fake(listening: UnixListener, case: Vec<u8>, eof_after: bool) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let Ok((mut socket, _)) = listening.accept() else {
            return;
        };
        let _ = socket.set_read_timeout(Some(Duration::from_secs(5)));
        if reply(&mut socket).is_none() {
            return;
        }
        let _ = socket.write_all(&case);
        if eof_after {
            return;
        }
        let _ = socket.read(&mut [0_u8; 1]);
    })
}

fn me() -> melchior::Identity {
    melchior::Identity {
        project: "corpus".to_owned(),
        role: "main".to_owned(),
        id: "alpha-rho".to_owned(),
    }
}

#[test]
fn every_client_case_is_taken_by_held_as_it_says() {
    for case in cases()["client"].as_array().expect("client cases") {
        let (file, bytes) = bytes_of(case);
        let room = Scratch::new("mwc", "held");
        let at = room.join("s");
        let server = fake(
            UnixListener::bind(&at).expect("bind"),
            bytes,
            case["eof_after"].as_bool().unwrap_or(false),
        );
        let mut held = Held::within(&at, &me(), Duration::from_secs(2)).expect("dialled");
        let first = held.call("verbs", Vec::new());
        match case["expect"].as_str().expect("expect") {
            "accepted" => {
                let reply = first.unwrap_or_else(|why| panic!("{file}: {why}"));
                assert!(reply.ok, "{file}: {reply:?}");
                assert_eq!(reply.result, [serde_json::json!("x")], "{file}");
            }
            "failed" => {
                let reply = first.unwrap_or_else(|why| panic!("{file}: {why}"));
                assert!(!reply.ok, "{file}");
                assert_eq!(reply.fault, Some(melchior::wire::Fault::Failed), "{file}");
            }
            "refused" => match first {
                Err(why) => assert!(why.to_string().contains("newer"), "{file}: {why}"),
                Ok(reply) => assert!(!reply.ok, "{file}: {reply:?}"),
            },
            kind @ ("error_and_close" | "error_and_close_without_allocating") => {
                let why = first.expect_err(&file);
                if kind.ends_with("without_allocating") {
                    assert!(framing::too_large(&why), "{file}: {why}");
                }
                let after = held.call("verbs", Vec::new()).expect_err(&file);
                assert!(after.to_string().contains("closed"), "{file}: {after}");
            }
            other => panic!("{file}: no such expectation '{other}'"),
        }
        drop(held);
        server.join().expect("the fake ends");
    }
}

/// What the shipped Lua client made of `case`: its call's two values, then a following call's
/// reason, joined by `|`.
fn lua_took(name: &str, case: Vec<u8>, eof_after: bool) -> String {
    let room = Project::new("mwc-t", name);
    let at = room.home().join("s");
    let server = fake(UnixListener::bind(&at).expect("bind"), case, eof_after);
    let mut engine = Engine::new();
    let probe = format!(
        "local client = load([=====[\n{source}]=====])(melchior.stream)\n\
         local s = assert(client.connect({{ path = {at:?}, timeout_ms = 2000 }}))\n\
         local a, b = s:call('verbs')\n\
         local _, after = s:call('verbs')\n\
         melchior.answer = tostring(a) .. '|' .. tostring(b) .. '|' .. tostring(after)\n",
        source = melchior::CLIENT,
        at = at.display().to_string(),
    );
    engine
        .run(&probe, "corpus.lua")
        .unwrap_or_else(|why| panic!("{name}: the client raised: {why}"));
    engine.harvest();
    let answer = engine
        .config()
        .string("answer")
        .unwrap_or_default()
        .to_owned();
    drop(engine);
    server.join().expect("the fake ends");
    answer
}

#[test]
fn every_client_case_is_taken_by_the_lua_client_as_it_says() {
    for case in cases()["client"].as_array().expect("client cases") {
        let (file, bytes) = bytes_of(case);
        let name = file.replace(['/', '.'], "-");
        let took = lua_took(&name, bytes, case["eof_after"].as_bool().unwrap_or(false));
        let parts: Vec<&str> = took.split('|').collect();
        match case["expect"].as_str().expect("expect") {
            "accepted" => assert_eq!(parts[0], "x", "{file}: {took}"),
            "failed" => assert_eq!(parts[..2], ["nil", "no"], "{file}: {took}"),
            "refused" => {
                assert_eq!(parts[0], "nil", "{file}: {took}");
                assert!(parts[1].contains("newer"), "{file}: {took}");
            }
            kind @ ("error_and_close" | "error_and_close_without_allocating") => {
                assert_eq!(parts[0], "nil", "{file}: {took}");
                assert!(parts[1].contains("closed"), "{file}: {took}");
                assert_eq!(parts[2], "this connection is closed", "{file}: {took}");
                if kind.ends_with("without_allocating") {
                    assert!(parts[1].contains("over the family's cap"), "{file}: {took}");
                }
            }
            other => panic!("{file}: no such expectation '{other}'"),
        }
    }
}
