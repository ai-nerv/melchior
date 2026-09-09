//! Naming a child, and the secret that makes `stop` refusable.
//!
//! Split from [`super`] under THE RULE. These are the half of the subagent lattice that was
//! never produced: `parent()` and `token()` read `MAGI_MELCHIOR_PARENT` and
//! `MAGI_MELCHIOR_TOKEN`, `announce` writes the note that makes the tree readable off the
//! directory, `children` reads it back and `stop` refuses anything a session did not start —
//! every piece correct, and every piece inert, because nothing anywhere minted a secret or
//! handed a name down. A harness that spawned a child got a *main*: no parent, outside every
//! wall the policy draws, and unstoppable by the thing that started it.

use super::tests::{alone, listening, named};
use std::time::Duration;

/// One hand-written call carrying a `from`, so the policy can place the caller.
fn asked_as(at: &std::path::Path, verb: &str, from: &str) -> serde_json::Value {
    use std::io::{Read, Write};
    let mut sock = std::os::unix::net::UnixStream::connect(at).expect("connected");
    sock.set_read_timeout(Some(Duration::from_secs(5)))
        .expect("a timeout");
    let body = format!(r#"{{"call":"{verb}","from":"{from}"}}"#);
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

/// A session names a child and mints its secret, and remembers having done so.
///
/// **The half of the subagent lattice that was never produced.** `parent()` and `token()`
/// read `MAGI_MELCHIOR_PARENT` and `MAGI_MELCHIOR_TOKEN`, `announce` writes the note that
/// makes the tree readable off the directory, `children` reads it back and `stop` refuses
/// anything a session did not start — every piece correct, and every piece inert, because
/// nothing anywhere minted a secret or handed a name down. A harness that spawned a child
/// got a *main*: no parent, outside every wall the policy draws, and unstoppable by the
/// thing that started it.
#[tokio::test]
async fn a_session_names_a_child_and_keeps_the_secret_it_minted() {
    let it = alone("minting");
    let me = named(&it, "alpha-nu");
    let _bound = listening(&me).await;
    let at = crate::directory::listening_at(&me);
    let full = me.full();

    let (at2, full2) = (at.clone(), full.clone());
    let reply = tokio::task::spawn_blocking(move || asked_as(&at2, "mint", &full2))
        .await
        .expect("the thread finished");
    assert_eq!(reply["ok"], true, "{reply}");

    let child = &reply["result"][0];
    let id = child["id"].as_str().expect("a name").to_owned();
    let token = child["token"].as_str().expect("a secret").to_owned();
    assert_ne!(id, me.id, "a child is not its parent");
    assert_eq!(child["parent"], me.id, "and it knows whose it is");
    assert_eq!(token.len(), 32, "sixteen bytes as hex: {token}");

    // The environment a harness starts it with, named here so the harness does not have to
    // know which variables melchior reads.
    let env = &child["environment"];
    assert_eq!(env[crate::inherited::PARENT], me.id);
    assert_eq!(env[crate::inherited::TOKEN], token);
    assert_eq!(env[crate::inherited::ID], id);
    assert_eq!(
        env[crate::inherited::SESSION],
        me.id,
        "a root hands down its own id as the run: {env}"
    );

    // Kept, which is what makes `stop` refusable rather than a guess. Read back over the
    // socket because that is where it lives: never on the directory, where a sibling would
    // have authority over a session it did not start.
    let held = tokio::task::spawn_blocking(move || asked_as(&at, "minted", &full))
        .await
        .expect("the thread finished");
    assert_eq!(held["result"][0][&id], token, "{held}");
}

/// A run is handed down, not started afresh at every hop.
///
/// The failure without it: a coordinator that spawns a coordinator gives its grandchildren a
/// run named after their own parent, and one job comes out as a tree of runs that share no
/// roster and no memory directory. The child inherits what the *note beside this socket* says,
/// which is what everybody else reads too.
#[tokio::test]
async fn a_child_inherits_the_run_rather_than_starting_one() {
    let it = alone("minting-run");
    let me = named(&it, "delta-rho");
    let _bound = listening(&me).await;
    let at = crate::directory::listening_at(&me);
    std::fs::write(crate::directory::sessions::session_at(&me), "alpha-rho").expect("the note");

    let full = me.full();
    let reply = tokio::task::spawn_blocking(move || asked_as(&at, "mint", &full))
        .await
        .expect("the thread finished");
    let child = &reply["result"][0];
    assert_eq!(child["session"], "alpha-rho", "{reply}");
    assert_eq!(
        child["environment"][crate::inherited::SESSION],
        "alpha-rho",
        "the child was handed a run of its own: {reply}"
    );
}

/// A secret is not something a sibling may read.
#[tokio::test]
async fn what_was_minted_is_refused_to_anybody_who_could_not_stop_this() {
    let it = alone("minting-wall");
    let me = named(&it, "beta-rho");
    let _bound = listening(&me).await;
    let at = crate::directory::listening_at(&me);

    // A main in the same project: it may ask this session things and tell it things, and it
    // may not end it — so it may not hold what would let it.
    let stranger = named(&it, "gamma-pi").full();
    let reply = tokio::task::spawn_blocking(move || asked_as(&at, "minted", &stranger))
        .await
        .expect("the thread finished");
    assert_eq!(reply["ok"], false, "a sibling reads no secrets: {reply}");
}
