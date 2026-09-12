//! Listening, so another instance can reach this one.
//!
//! Nothing here decides anything: it frames bytes, works out where the caller sits in the tree,
//! and hands both to [`crate::answering::answer`], which is where the permission check and the
//! vocabulary live.

use crate::answering::{About, Then, answer};
use crate::directory::{inside, whom};
use crate::framing;
use crate::policy::Whom;
use crate::wire::{Call, Message, Reply};
use std::path::Path;
use tokio::sync::mpsc;

/// How many callers may be connected at once, bounded because the socket is reachable by
/// anything running as this user.
const CALLERS: usize = 8;

/// How long a connection may sit idle before it is dropped.
const IDLE: std::time::Duration = std::time::Duration::from_secs(300);

/// What the socket needs from the session, and what it sends back to it.
pub struct Serving {
    /// Told what this instance is doing, whenever a call needs to know.
    pub about: tokio::sync::watch::Receiver<About>,
    /// Messages that arrived, on their way to the inbox.
    pub arrived: mpsc::Sender<Message>,
    /// Requests that arrived, on their way to the person at the keyboard. Separate from
    /// `arrived` because a request waits for an answer only a person can give.
    pub asked: mpsc::Sender<crate::wire::Request>,
    /// What a parent handed over when it took this session on (for the harness), and the stop
    /// secret it minted (for the loop's own record, never the harness).
    pub adopted: mpsc::Sender<(String, Option<String>, Option<String>)>,
    /// Somebody with the right to stop this instance did.
    pub stopped: mpsc::Sender<()>,
    /// A child was named and its secret minted, on its way to this session's own record of it.
    /// Carried up rather than written here: the loop that owns `About` is its only writer.
    pub minted: mpsc::Sender<(String, String)>,
    /// This session has been told what it is for, on its way to the note every peer reads. Up
    /// rather than written here, so one file has one writer.
    pub named: mpsc::Sender<crate::directory::roles::Role>,
}

/// Listen on `path` until the process ends. A stale socket is cleared first: `bind` fails with
/// `EADDRINUSE` on a path whose maker is long gone.
pub async fn serve(path: &Path, serving: Serving) -> std::io::Result<()> {
    accept(listening_on(path).await?, serving).await
}

/// Take the socket, and hand it back before anything is served on it.
///
/// Split from [`accept`] so a caller can say "I am reachable" at the moment it becomes true.
/// Announcing before the bind announces a future, and a parent that started sending on the
/// strength of it met its own session as "nothing is listening".
pub async fn listening_on(path: &Path) -> std::io::Result<tokio::net::UnixListener> {
    if !inside(path) {
        // Belt and braces: the path is built from a project name, which is the working
        // directory's and can be anything.
        return Err(std::io::Error::other("that is not an instance socket"));
    }
    bind(path).await
}

/// Accept callers on a socket already taken, and serve them until the process ends.
pub async fn accept(listener: tokio::net::UnixListener, serving: Serving) -> std::io::Result<()> {
    let held = std::sync::Arc::new(tokio::sync::Semaphore::new(CALLERS));

    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        // Another user's process gets nothing at all, whatever it says about itself.
        if !ours(&stream) {
            continue;
        }
        // Refused rather than queued: a caller told "busy" can ask again, while one held in a
        // queue waits on a session that may be blocked for a whole turn.
        let Ok(permit) = std::sync::Arc::clone(&held).try_acquire_owned() else {
            continue;
        };
        let serving = Serving {
            about: serving.about.clone(),
            arrived: serving.arrived.clone(),
            asked: serving.asked.clone(),
            adopted: serving.adopted.clone(),
            stopped: serving.stopped.clone(),
            minted: serving.minted.clone(),
            named: serving.named.clone(),
        };
        tokio::spawn(async move {
            let _permit = permit;
            let _ = talk(stream, serving).await;
        });
    }
}

/// One connection, for as many calls as it cares to make. It keeps serving after replying: a
/// client that holds its connection dies on its second call against a server that closes.
async fn talk(stream: tokio::net::UnixStream, serving: Serving) -> std::io::Result<()> {
    // Which of the user's sessions the caller is, from the kernel — the one thing it cannot forge.
    let peer = stream.peer_cred().ok().and_then(|cred| cred.pid());
    let (mut reader, mut writer) = stream.into_split();

    loop {
        // Read with the encoding it arrived in, so the reply goes back the same way.
        let (call, wire): (Call, framing::Wire) =
            match tokio::time::timeout(IDLE, framing::read_wire(&mut reader)).await {
                Ok(Ok(pair)) => pair,
                // Hanging up is not a mistake. A caller that has said everything closes its
                // write half and reads on, so answering the `UnexpectedEof` with a refusal sends
                // it a second frame saying something untrue.
                Ok(Err(why)) if why.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
                // A malformed frame still gets an answer, then the connection ends: a stream that
                // has lost its framing cannot be resynchronised.
                Ok(Err(why)) => {
                    let _ = framing::write(&mut writer, &Reply::refused(why.to_string())).await;
                    return Ok(());
                }
                Err(_) => return Ok(()),
            };
        let mut about = serving.about.borrow().clone();
        // This session's own parent changes from outside — another session accepts it as a child
        // and writes the note — so it is re-read per call rather than trusted from startup.
        about.parent = crate::directory::parent_of(&about.me);
        // And the caller's, per call rather than per connection: a session may fork mid-call.
        let caller = caller_of(&call, peer, &about);
        let (reply, then) = answer(&call, &about, caller.as_ref());
        framing::write_as(&mut writer, wire, &reply).await?;
        match then {
            Then::Nothing => {}
            Then::Keep(message) => {
                let _ = serving.arrived.send(message).await;
            }
            Then::Adopted {
                by,
                handover,
                secret,
            } => {
                let _ = serving.adopted.send((by, handover, secret)).await;
            }
            Then::Ask(request) => {
                let _ = serving.asked.send(request).await;
            }
            Then::Minted { id, token } => {
                let _ = serving.minted.send((id, token)).await;
            }
            Then::Named(role) => {
                let _ = serving.named.send(role).await;
            }
            Then::Stop => {
                let _ = serving.stopped.send(()).await;
                return Ok(());
            }
        }
    }
}

/// Whether the far end is this user at all, from `SO_PEERCRED` rather than anything the caller
/// sent. It is also all the kernel can settle: every session in a project runs as one user.
fn ours(stream: &tokio::net::UnixStream) -> bool {
    stream
        .peer_cred()
        .is_ok_and(|cred| cred.uid() == rustix::process::getuid().as_raw())
}

/// Listen at `path`, clearing what a crash left behind: a socket file outlives the process that
/// made it, so `bind` fails with `EADDRINUSE` on a path nothing has answered. Connected to before
/// removed, because a path that answers belongs to a running session.
async fn bind(path: &Path) -> std::io::Result<tokio::net::UnixListener> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    if tokio::fs::metadata(path).await.is_ok()
        && tokio::net::UnixStream::connect(path).await.is_err()
    {
        tokio::fs::remove_file(path).await?;
    }
    tokio::net::UnixListener::bind(path)
}

/// Where a caller sits in the tree, from the name it gave. The name is the caller's; the place is
/// looked up in the project directory, so a session cannot claim to be somebody's child and be
/// believed. A name from another project resolves to a stranger, and `None` — a caller that said
/// nothing at all — is a different mistake with a different answer.
fn placed(from: Option<&str>, about: &About) -> Option<Whom> {
    // Parsed as a whole name, never split at the first slash: `magi/review/iota-mu` cut that way
    // is a session called `review/iota-mu`, which is nobody. The role is dropped on purpose.
    let them = crate::identity::Identity::read(from?)?;
    Some(place(&them.project, &them.id, about))
}

/// Where a session by this project and id sits in the tree. Another project's is placed with no
/// parent and no run: its directory is not one this session lists.
fn place(project: &str, id: &str, about: &About) -> Whom {
    if project != about.me.project {
        return Whom {
            project: project.to_owned(),
            id: id.to_owned(),
            parent: None,
            session: None,
            root: None,
        };
    }
    whom(project, id)
}

/// Who to treat as the caller. An [`is_authority`] verb takes its caller only from the kernel, so
/// none can pass for a session that did not spawn it; any other verb falls back to the `from` it
/// sent, which is never taken over the kernel.
fn caller_of(call: &Call, peer: Option<i32>, about: &About) -> Option<Whom> {
    let verified = claimed(peer, about);
    if is_authority(&call.call) {
        verified
    } else {
        verified.or_else(|| placed(call.from.as_deref(), about))
    }
}

/// The verbs whose authority is being the session itself, so their caller comes from the kernel.
fn is_authority(verb: &str) -> bool {
    matches!(
        verb,
        "mint" | "minted" | "role" | "adopt" | "adopted" | "tool"
    )
}

/// The caller as the kernel names it: the session id in the connecting process's own environment.
/// `None` — the most restricted caller — when it names none, or the process is gone.
fn claimed(peer: Option<i32>, about: &About) -> Option<Whom> {
    let body = std::fs::read(format!("/proc/{}/environ", peer?)).ok()?;
    let project = var_in(&body, crate::inherited::PROJECT)?;
    let id = var_in(&body, crate::inherited::ID)?;
    Some(place(&project, &id, about))
}

/// One variable out of a NUL-separated `/proc/<pid>/environ` block, the older `MAGI_*` spelling too.
fn var_in(environ: &[u8], name: &str) -> Option<String> {
    let find = |key: &str| {
        let prefix = format!("{key}=");
        environ
            .split(|byte| *byte == 0)
            .filter_map(|entry| std::str::from_utf8(entry).ok())
            .find_map(|entry| entry.strip_prefix(&prefix))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    find(name).or_else(|| {
        find(&format!(
            "MAGI_{}",
            name.trim_start_matches("MAGI_MELCHIOR_")
        ))
    })
}

/// Two instances, one socket, and a message that actually arrives. The one thing that cannot be
/// settled without a socket is that the client half and the server half agree about the wire.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::asking::Held;
    use crate::identity::Identity;
    use crate::scratch::Project;
    use crate::wire::Sort;
    use std::time::Duration;

    /// A project nothing else is using, held for as long as the test is. One per test, because
    /// they run at once and a shared project directory tears down the sockets the others use.
    pub(super) fn alone(tag: &str) -> Project {
        Project::new("melchior-serve", tag)
    }

    pub(super) fn named(it: &Project, id: &str) -> Identity {
        Identity {
            project: it.to_string(),
            role: "main".to_owned(),
            id: id.to_owned(),
        }
    }

    /// What a bound session hands back, held for as long as the test needs it.
    pub(super) struct Bound {
        arrived: mpsc::Receiver<Message>,
        /// Dropping this closes the channel the socket reads the session's state from.
        _about: tokio::sync::watch::Sender<About>,
        _stopped: mpsc::Receiver<()>,
    }

    pub(super) async fn listening(me: &Identity) -> Bound {
        let (about, about_rx) = tokio::sync::watch::channel(About {
            me: me.clone(),
            parent: None,
            token: None,
            busy: false,
            working_for: 0,
            inbox: Vec::new(),
            minted: std::collections::BTreeMap::new(),
            adopted_token: None,
        });
        let (arrived_tx, arrived) = mpsc::channel(8);
        let (stopped_tx, stopped) = mpsc::channel(1);
        // Drained into `about`, the way the real loop does it.
        let (minted_tx, mut minted) = mpsc::channel::<(String, String)>(4);
        let recording = about.clone();
        tokio::spawn(async move {
            while let Some((id, token)) = minted.recv().await {
                recording.send_modify(|about| {
                    about.minted.insert(id, token);
                });
            }
        });
        let at = crate::directory::listening_at(me);
        tokio::spawn(async move {
            let _ = serve(
                &at,
                Serving {
                    asked: tokio::sync::mpsc::channel(4).0,
                    adopted: tokio::sync::mpsc::channel(4).0,
                    about: about_rx,
                    arrived: arrived_tx,
                    stopped: stopped_tx,
                    minted: minted_tx,
                    named: tokio::sync::mpsc::channel(4).0,
                },
            )
            .await;
        });
        // The bind is a few awaits away and the first dial would otherwise race it.
        let at = crate::directory::listening_at(me);
        for _ in 0..100 {
            if Held::at(&at, me).is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Bound {
            arrived,
            _about: about,
            _stopped: stopped,
        }
    }

    #[tokio::test]
    async fn a_message_sent_by_one_instance_arrives_at_another() {
        let it = alone("arrives");
        let them = named(&it, "beta-nu");
        let me = named(&it, "alpha-rho");
        let mut bound = listening(&them).await;

        // From a blocking thread, because that is where it happens for real: the client half is
        // a tool peer, which has no runtime and wants none.
        let (they, i) = (them.clone(), me.clone());
        let sent = tokio::task::spawn_blocking(move || {
            let mut held = crate::directory::dial(&they, &i).expect("it is listening");
            held.call(
                "tell",
                vec![
                    serde_json::json!("the parser is done"),
                    serde_json::json!("attention"),
                    serde_json::json!(null),
                ],
            )
            .expect("answered")
        })
        .await
        .expect("the thread finished");

        assert!(sent.ok, "{sent:?}");
        let message = tokio::time::timeout(Duration::from_secs(5), bound.arrived.recv())
            .await
            .expect("it arrived")
            .expect("the channel is open");
        assert_eq!(message.text, "the parser is done");
        // Never from an argument: a message that names its own sender is one anybody can forge.
        assert_eq!(message.from, me.full());
        assert_eq!(message.sort, Sort::Attention);
        assert!(message.sort.interrupts());
    }

    #[tokio::test]
    async fn one_connection_serves_several_calls() {
        // A client that holds its connection dies on its second call against a server that
        // closes after replying.
        let it = alone("held");
        let them = named(&it, "gamma-xi");
        let me = named(&it, "delta-pi");
        let _bound = listening(&them).await;

        let (they, i) = (them.clone(), me.clone());
        let answers = tokio::task::spawn_blocking(move || {
            let mut held = crate::directory::dial(&they, &i).expect("it is listening");
            ["verbs", "identity", "status", "identity"]
                .into_iter()
                .map(|verb| held.call(verb, Vec::new()).expect("answered"))
                .collect::<Vec<_>>()
        })
        .await
        .expect("the thread finished");

        for (at, reply) in answers.iter().enumerate() {
            assert!(reply.ok, "call {at} failed: {reply:?}");
            assert_eq!(reply.n, reply.result.len(), "call {at}");
        }
        assert_eq!(answers[1].result[0]["id"], them.id);
    }

    #[tokio::test]
    async fn a_main_refuses_a_stop_from_another_main_at_the_wall() {
        // Two gates stand between a caller and a stop, and this is the outer one: nobody started
        // this session, so no relation makes the caller its parent.
        let it = alone("wall");
        let them = named(&it, "epsilon-tau");
        let me = named(&it, "zeta-nu");
        let _bound = listening(&them).await;

        let (they, i) = (them.clone(), me.clone());
        let reply = tokio::task::spawn_blocking(move || {
            let mut held = crate::directory::dial(&they, &i).expect("it is listening");
            held.call_with("stop", Vec::new(), "guessed")
                .expect("answered")
        })
        .await
        .expect("the thread finished");

        assert!(!reply.ok, "a stop went through: {reply:?}");
        let why = reply.error.unwrap_or_default();
        assert!(
            why.contains("only the session that started one may stop it"),
            "it did not say why: {why}"
        );
    }

    #[tokio::test]
    async fn claiming_to_be_the_parent_is_not_enough_to_stop_a_child() {
        // The inner gate. Every session in a project runs as one user, so any process here can
        // call itself the parent and the directory will agree; what it cannot do is produce the
        // secret.
        let it = alone("secret");
        let them = named(&it, "iota-mu");
        let me = named(&it, "kappa-rho");
        let mut about = About {
            me: them.clone(),
            parent: Some(me.id.clone()),
            token: Some("the-real-one".to_owned()),
            busy: false,
            working_for: 0,
            inbox: Vec::new(),
            minted: std::collections::BTreeMap::new(),
            adopted_token: None,
        };
        // The note a child leaves beside its socket. Written by hand here; a session writes its
        // own.
        std::fs::write(crate::directory::kin_at(&them), &me.id).expect("the note");

        let (about_tx, about_rx) = tokio::sync::watch::channel(about.clone());
        about.busy = false;
        let (arrived_tx, _arrived) = mpsc::channel(8);
        let (stopped_tx, mut stopped) = mpsc::channel(1);
        let at = crate::directory::listening_at(&them);
        tokio::spawn(async move {
            let _ = serve(
                &at,
                Serving {
                    asked: tokio::sync::mpsc::channel(4).0,
                    adopted: tokio::sync::mpsc::channel(4).0,
                    about: about_rx,
                    arrived: arrived_tx,
                    stopped: stopped_tx,
                    minted: tokio::sync::mpsc::channel(4).0,
                    named: tokio::sync::mpsc::channel(4).0,
                },
            )
            .await;
        });
        let at = crate::directory::listening_at(&them);
        for _ in 0..100 {
            if Held::at(&at, &me).is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let _keep = about_tx;

        let (they, i) = (them.clone(), me.clone());
        let (guessed, right) = tokio::task::spawn_blocking(move || {
            let mut held = crate::directory::dial(&they, &i).expect("it is listening");
            (
                held.call_with("stop", Vec::new(), "guessed")
                    .expect("answered"),
                held.call_with("stop", Vec::new(), "the-real-one")
                    .expect("answered"),
            )
        })
        .await
        .expect("the thread finished");

        assert!(!guessed.ok, "a guess stopped it: {guessed:?}");
        assert!(
            guessed.error.unwrap_or_default().contains("secret"),
            "and it did not say what was wrong"
        );
        assert!(
            right.ok,
            "the parent could not stop its own child: {right:?}"
        );
        assert!(
            tokio::time::timeout(Duration::from_secs(5), stopped.recv())
                .await
                .expect("the session was told")
                .is_some()
        );
    }

    #[tokio::test]
    async fn a_sibling_tool_speaking_the_family_shape_is_understood() {
        // Hand-written frames, the way anything that is not magi would send them.
        let it = alone("sibling");
        let them = named(&it, "theta-mu");
        let _bound = listening(&them).await;
        let at = crate::directory::listening_at(&them);

        let asked = tokio::task::spawn_blocking(move || {
            use std::io::{Read, Write};
            let mut sock = std::os::unix::net::UnixStream::connect(&at).expect("connected");
            sock.set_read_timeout(Some(Duration::from_secs(5)))
                .expect("a timeout");
            ["verbs", "status"]
                .into_iter()
                .map(|verb| {
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
                    serde_json::from_slice::<serde_json::Value>(&answer).expect("it is JSON")
                })
                .collect::<Vec<_>>()
        })
        .await
        .expect("the thread finished");

        assert_eq!(asked[0]["ok"], true, "verbs answers anybody: {}", asked[0]);
        assert!(asked[0]["result"].is_array(), "and in the family's shape");
        // Everything else is about this session, and a stranger has no standing to ask.
        assert_eq!(asked[1]["ok"], false, "status must not: {}", asked[1]);
    }
}

/// A stranger may have the vocabulary and the library that speaks it, and nothing else.
#[cfg(test)]
mod handing;

/// A caller's own account of what it is for reaches nothing that decides anything.
#[cfg(test)]
mod roles;

/// Naming a child, and the secret that makes `stop` refusable.
#[cfg(test)]
#[path = "serving/minting.rs"]
mod minting;

/// Hanging up is not a mistake; a bad frame is.
#[cfg(test)]
mod parting {
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

    #[tokio::test]
    async fn a_one_shot_gets_one_frame_and_nothing_after_it() {
        // magi's own client holds its connection and reads exactly one reply per call, so it
        // never saw the second frame; a sibling parsing until EOF chokes on it.
        let it = alone("parting");
        let them = named(&it, "mu-rho");
        let _bound = listening(&them).await;
        let at = crate::directory::listening_at(&them);

        let back =
            tokio::task::spawn_blocking(move || one_shot(&at, &framed(r#"{"call":"verbs"}"#)))
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
}
