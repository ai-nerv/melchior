//! Hanging up is not a mistake; a bad frame is, and an oversized one is answered before the close.

use super::tests::{alone, listening, named};
use std::io::{Read, Write};
use std::time::Duration;

/// Send `bytes`, close the write half, and read everything that comes back — the shape of a
/// one-shot `printf … | socat - UNIX-CONNECT:…`.
fn one_shot(at: &std::path::Path, bytes: &[u8]) -> Vec<u8> {
    let mut sock = std::os::unix::net::UnixStream::connect(at).expect("connected");
    sock.set_read_timeout(Some(Duration::from_secs(5)))
        .expect("a timeout");
    sock.write_all(bytes).expect("wrote");
    sock.shutdown(std::net::Shutdown::Write).expect("hung up");
    let mut back = Vec::new();
    let _ = sock.read_to_end(&mut back);
    back
}

/// One call, framed by hand.
fn framed(body: &str) -> Vec<u8> {
    let mut out = u32::try_from(body.len())
        .expect("fits")
        .to_be_bytes()
        .to_vec();
    out.extend_from_slice(body.as_bytes());
    out
}

/// Every frame in `back`, decoded.
fn frames(back: &[u8]) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 4 <= back.len() {
        let len = u32::from_be_bytes([back[at], back[at + 1], back[at + 2], back[at + 3]]) as usize;
        let body = &back[at + 4..(at + 4 + len).min(back.len())];
        out.push(serde_json::from_slice(body).expect("a reply"));
        at += 4 + len;
    }
    out
}

#[tokio::test]
async fn a_one_shot_gets_one_frame_and_nothing_after_it() {
    // magi's own client holds its connection and reads exactly one reply per call, so it
    // never saw the second frame; a sibling parsing until EOF chokes on it.
    let it = alone("parting");
    let them = named(&it, "mu-rho");
    let _bound = listening(&them).await;
    let at = crate::directory::listening_at(&them);

    let back = tokio::task::spawn_blocking(move || one_shot(&at, &framed(r#"{"call":"verbs"}"#)))
        .await
        .expect("the thread finished");

    let said = u32::from_be_bytes([back[0], back[1], back[2], back[3]]) as usize;
    assert_eq!(
        back.len(),
        said + 4,
        "a second frame followed the answer: {}",
        String::from_utf8_lossy(&back[said + 4..])
    );
}

#[tokio::test]
async fn a_frame_that_is_actually_broken_is_still_answered() {
    // A refusal a caller can read beats a dropped connection: "expected value at line 1" says
    // what to fix where "connection reset" does not.
    let it = alone("broken");
    let them = named(&it, "nu-rho");
    let _bound = listening(&them).await;
    let at = crate::directory::listening_at(&them);

    let back = tokio::task::spawn_blocking(move || one_shot(&at, &framed("not json")))
        .await
        .expect("the thread finished");

    assert!(!back.is_empty(), "a broken frame got no answer at all");
    let reply: serde_json::Value =
        serde_json::from_slice(&back[4..]).expect("the refusal is a reply");
    assert_eq!(reply["ok"], false, "{reply}");
}

#[tokio::test]
async fn a_caller_that_says_nothing_at_all_is_not_answered() {
    // Connecting and hanging up is what a liveness check does — see `asking::answers`.
    let it = alone("silent");
    let them = named(&it, "xi-rho");
    let _bound = listening(&them).await;
    let at = crate::directory::listening_at(&them);

    let back = tokio::task::spawn_blocking(move || one_shot(&at, &[]))
        .await
        .expect("the thread finished");

    assert!(back.is_empty(), "{}", String::from_utf8_lossy(&back));
}

#[tokio::test]
async fn a_frame_over_the_cap_is_answered_failed_and_then_closed() {
    let it = alone("oversize");
    let them = named(&it, "omicron-rho");
    let _bound = listening(&them).await;
    let at = crate::directory::listening_at(&them);

    let header = u32::try_from(crate::framing::MOST + 1)
        .expect("fits")
        .to_be_bytes();
    let mut bytes = header.to_vec();
    bytes.extend_from_slice(&framed(r#"{"call":"verbs"}"#));
    let back = tokio::task::spawn_blocking(move || one_shot(&at, &bytes))
        .await
        .expect("the thread finished");

    let replies = frames(&back);
    assert_eq!(
        replies.len(),
        1,
        "one refusal and then the close: {replies:?}"
    );
    assert_eq!(replies[0]["ok"], false);
    assert_eq!(replies[0]["fault"], "failed", "{}", replies[0]);
    let why = replies[0]["error"].as_str().unwrap_or_default();
    assert!(why.contains(&crate::framing::MOST.to_string()), "{why}");
}

#[tokio::test]
async fn a_frame_of_no_bytes_is_refused_as_malformed() {
    let it = alone("empty");
    let them = named(&it, "pi-rho");
    let _bound = listening(&them).await;
    let at = crate::directory::listening_at(&them);

    let back = tokio::task::spawn_blocking(move || one_shot(&at, &0_u32.to_be_bytes()))
        .await
        .expect("the thread finished");

    let replies = frames(&back);
    assert_eq!(replies.len(), 1, "{replies:?}");
    assert_eq!(replies[0]["fault"], "refused", "{}", replies[0]);
    assert!(
        replies[0]["error"]
            .as_str()
            .unwrap_or_default()
            .contains("malformed")
    );
}

#[test]
fn a_reply_over_the_cap_is_replaced_by_the_refusal_that_says_so() {
    let huge = crate::wire::Reply::of(serde_json::json!("z".repeat(crate::framing::MOST)));
    let out = super::written(crate::framing::Wire::Json, &huge).expect("something to write");
    let replies = frames(&out);
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0]["fault"], "failed", "{}", replies[0]);
    assert!(out.len() < 1024, "the refusal is small: {}", out.len());
}
