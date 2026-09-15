//! A stranger can fetch the vocabulary and the library that speaks it, and nothing else.
//!
//! Split from [`super`] under THE RULE, which caps a file at 800 lines.

use super::tests::{alone, listening, named};
use std::time::Duration;

/// One hand-written call, the way anything that is not magi would send it.
fn asked(at: &std::path::Path, verb: &str) -> serde_json::Value {
    use std::io::{Read, Write};
    let mut sock = std::os::unix::net::UnixStream::connect(at).expect("connected");
    sock.set_read_timeout(Some(Duration::from_secs(5)))
        .expect("a timeout");
    let body = format!(r#"{{"call":"{verb}"}}"#);
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

#[tokio::test]
async fn the_client_library_comes_back_over_the_wire() {
    // `agent lua-api` prints the same source, which is enough for a host that can shell out
    // and useless to one that cannot: a sandboxed VM with no `io.popen` has no way to run
    // it. So a sibling that speaks the framing can fetch the right vocabulary using the
    // wrong one, in code, with nothing written to disk.
    let it = alone("handed");
    let them = named(&it, "theta-mu");
    let _bound = listening(&them).await;
    let at = crate::directory::listening_at(&them);

    let reply = tokio::task::spawn_blocking(move || asked(&at, "client"))
        .await
        .expect("the thread finished");

    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["n"], 1, "one value, in a list");
    let source = reply["result"][0].as_str().expect("source");
    assert_eq!(source, crate::CLIENT, "and it is the file this crate ships");
}

#[tokio::test]
async fn verbs_and_client_are_the_two_a_stranger_may_have() {
    // Everything else is *about this session*, and somebody who will not say who they are
    // has no standing to ask. These two are about the surface: what it speaks, and the
    // library that speaks it. Neither says anything about who is answering.
    let it = alone("stranger");
    let them = named(&it, "iota-nu");
    let _bound = listening(&them).await;
    let at = crate::directory::listening_at(&them);

    let (open, closed) = tokio::task::spawn_blocking(move || {
        (
            [
                asked(&at, "verbs")["ok"].clone(),
                asked(&at, "client")["ok"].clone(),
            ],
            [
                asked(&at, "identity")["ok"].clone(),
                asked(&at, "status")["ok"].clone(),
                asked(&at, "inbox")["ok"].clone(),
                asked(&at, "stop")["ok"].clone(),
            ],
        )
    })
    .await
    .expect("the thread finished");

    assert!(open.iter().all(|ok| *ok == true), "{open:?}");
    assert!(closed.iter().all(|ok| *ok == false), "{closed:?}");
}

#[tokio::test]
async fn every_verb_it_lists_is_one_a_client_could_call() {
    // `verbs` promising something nothing answers is worse than not listing it: a client
    // written from that list fails in somebody else's program.
    let it = alone("listed");
    let them = named(&it, "kappa-nu");
    let _bound = listening(&them).await;
    let at = crate::directory::listening_at(&them);

    let listed = tokio::task::spawn_blocking(move || asked(&at, "verbs"))
        .await
        .expect("the thread finished");
    let named_verbs: Vec<String> = listed["result"]
        .as_array()
        .expect("a list")
        .iter()
        .filter_map(|entry| entry["verb"].as_str().map(ToOwned::to_owned))
        .collect();

    let known: Vec<&str> = crate::wire::VERBS.iter().map(|(verb, _)| *verb).collect();
    assert_eq!(named_verbs, known);
}
