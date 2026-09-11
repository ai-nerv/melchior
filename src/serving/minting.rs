//! Naming a child, the secret that makes `stop` refusable, and who may ask for either.

use super::tests::{alone, listening, named};
use crate::answering::{About, Then, answer};
use crate::identity::Identity;
use crate::wire::Call;
use std::time::Duration;

/// This session, with nothing minted yet. A root: no parent, no token.
fn about(me: &Identity) -> About {
    About {
        me: me.clone(),
        parent: None,
        token: None,
        busy: false,
        working_for: 0,
        inbox: Vec::new(),
        minted: std::collections::BTreeMap::new(),
    }
}

/// A `mint` or `minted` as this session's own process makes it. `serving::caller_of` derives the
/// caller of an authority verb from the kernel — the connecting process's `/proc/environ` — and a
/// session's own fork process carries the session's id there, so the caller it hands `answer` is
/// this session. Placing the caller here as `about.whom()` is exactly that.
fn as_myself(verb: &str, about: &About) -> (crate::wire::Reply, Then) {
    let mine = about.whom();
    answer(
        &Call {
            call: verb.to_owned(),
            ..Call::default()
        },
        about,
        Some(&mine),
    )
}

/// A hand-written call over a real socket, carrying whatever `from` it likes. What a process that
/// is not this session sends when it tries to pass for it.
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

/// A session names a child and mints its secret, and hands the child what it needs to come up.
#[test]
fn a_session_names_a_child_and_keeps_the_secret_it_minted() {
    let it = alone("minting");
    let me = named(&it, "alpha-nu");
    let about = about(&me);

    let (reply, then) = as_myself("mint", &about);
    assert!(reply.ok, "{reply:?}");

    let child = &reply.result[0];
    let id = child["id"].as_str().expect("a name").to_owned();
    let token = child["token"].as_str().expect("a secret").to_owned();
    assert_ne!(id, me.id, "a child is not its parent");
    assert_eq!(child["parent"], me.id, "and it knows whose it is");
    assert_eq!(token.len(), 32, "sixteen bytes as hex: {token}");

    // The environment a harness starts it with, so it need not know which variables melchior reads.
    let env = &child["environment"];
    assert_eq!(env[crate::inherited::PARENT], me.id);
    assert_eq!(env[crate::inherited::TOKEN], token);
    assert_eq!(env[crate::inherited::ID], id);
    assert_eq!(
        env[crate::inherited::SESSION],
        me.id,
        "a root hands down its own id as the run: {env}"
    );

    // Carried up to the session, which is what records it — never written to the directory.
    match then {
        Then::Minted {
            id: minted,
            token: secret,
        } => {
            assert_eq!(minted, id);
            assert_eq!(secret, token);
        }
        other => panic!("mint did not carry the secret up: {other:?}"),
    }
}

/// A tree of agents has a bottom: a session already at the depth limit mints no child.
#[test]
fn a_session_at_the_depth_limit_starts_no_child() {
    use crate::directory::MAX_DEPTH;
    let it = alone("minting-depth");
    let me = named(&it, "deep-one");
    // A chain of parent notes above `me`, one short of the limit, so `me` sits at the last level
    // that may still spawn; a note per ancestor is all `depth_of` reads.
    let mut child = me.id.clone();
    for rung in 0..MAX_DEPTH {
        let parent = format!("rung-{rung}");
        crate::directory::wrote(&crate::directory::kin_at(&named(&it, &child)), &parent)
            .expect("a parent note");
        child = parent;
    }
    let (reply, then) = as_myself("mint", &about(&me));
    assert!(
        !reply.ok,
        "a session at the limit minted a child anyway: {reply:?}"
    );
    assert!(matches!(then, Then::Nothing), "and nothing was minted");
}

/// A run is handed down, not started afresh at every hop.
#[test]
fn a_child_inherits_the_run_rather_than_starting_one() {
    let it = alone("minting-run");
    let me = named(&it, "delta-rho");
    // The note `whom` reads to learn which run this session belongs to.
    std::fs::write(crate::directory::sessions::session_at(&me), "alpha-rho").expect("the note");
    let about = about(&me);

    let (reply, _) = as_myself("mint", &about);
    let child = &reply.result[0];
    assert_eq!(child["session"], "alpha-rho", "{reply:?}");
    assert_eq!(
        child["environment"][crate::inherited::SESSION],
        "alpha-rho",
        "the child was handed the run it belongs to: {reply:?}"
    );
}

/// A secret is not something a sibling may read — proven the way it matters, over a real socket
/// with a forged `from`. The connecting process is not this session in the kernel's eyes, whatever
/// name it puts on the wire, so `minted` is refused however it claims to be the session itself.
#[tokio::test]
async fn a_forged_from_reads_no_secrets() {
    let it = alone("minting-wall");
    let me = named(&it, "beta-rho");
    let _bound = listening(&me).await;
    let at = crate::directory::listening_at(&me);

    for claim in [me.full(), named(&it, "gamma-pi").full()] {
        let (at, asked) = (at.clone(), claim.clone());
        let reply = tokio::task::spawn_blocking(move || asked_as(&at, "minted", &asked))
            .await
            .expect("the thread finished");
        assert_eq!(
            reply["ok"], false,
            "a socket peer the kernel did not place as this session read its secrets claiming \
             `{claim}`: {reply}"
        );
    }
}
