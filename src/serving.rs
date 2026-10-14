//! Listening, so another instance can reach this one.
//!
//! Nothing here decides anything: it frames bytes, works out where the caller sits in the tree,
//! and hands both to [`crate::answering::answer`], which is where the permission check and the
//! vocabulary live.

use crate::answering::{About, Then, answer};
use crate::directory::inside;
use crate::framing;
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

/// The same, vouching only for what `origin` started: `None` vouches for nobody.
pub async fn serve_from(
    path: &Path,
    serving: Serving,
    origin: Option<crate::kin::Origin>,
) -> std::io::Result<()> {
    accept_from(listening_on(path).await?, serving, origin).await
}

/// Take the socket, and hand it back before anything is served on it, so a caller can say "I am
/// reachable" at the moment it becomes true.
pub async fn listening_on(path: &Path) -> std::io::Result<tokio::net::UnixListener> {
    if !inside(path) {
        return Err(std::io::Error::other("that is not an instance socket"));
    }
    bind(path).await
}

/// Accept callers on a socket already taken, and serve them until the process ends.
pub async fn accept(listener: tokio::net::UnixListener, serving: Serving) -> std::io::Result<()> {
    // The session that started this one, as the kernel names it, read once.
    accept_from(listener, serving, crate::kin::Origin::here()).await
}

/// [`accept`], vouching for what `origin` started.
pub async fn accept_from(
    listener: tokio::net::UnixListener,
    serving: Serving,
    origin: Option<crate::kin::Origin>,
) -> std::io::Result<()> {
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
        let pinned = callers::pinned(&stream, origin.as_ref());
        tokio::spawn(async move {
            let _permit = permit;
            let _ = talk(stream, pinned, serving).await;
        });
    }
}

/// One connection, for as many calls as it cares to make. It keeps serving after replying: a
/// client that holds its connection dies on its second call against a server that closes.
async fn talk(
    stream: tokio::net::UnixStream,
    pinned: callers::Pinned,
    serving: Serving,
) -> std::io::Result<()> {
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
                // A malformed or oversized frame still gets an answer, then the connection ends: a
                // body that will not be read cannot be skipped and stay in step.
                Ok(Err(why)) => {
                    let _ = framing::write(&mut writer, &unread(&why)).await;
                    return Ok(());
                }
                Err(_) => return Ok(()),
            };
        let mut about = serving.about.borrow().clone();
        // This session's own parent changes from outside — another session accepts it as a child
        // and writes the note — so it is re-read per call rather than trusted from startup.
        about.parent = crate::directory::parent_of(&about.me);
        // Who the connection is was pinned when it was accepted; where it sits is read per call.
        let caller = callers::caller_of(&call, &pinned, &about);
        let began = std::time::Instant::now();
        let (reply, then) = answer(&call, &about, caller.as_ref());
        // A `status` answered is the roster's two-second poll: a change in it is logged as `roster`.
        if call.call != "status" || !reply.ok {
            crate::noted!(
                "serve: {} from {} → {} in {}ms",
                call.call,
                caller.as_ref().map_or_else(
                    || "nobody".to_owned(),
                    |them| format!("{}/{}", them.project, them.id)
                ),
                if reply.ok {
                    "ok".to_owned()
                } else {
                    format!(
                        "refused: {}",
                        crate::noted::short(reply.error.as_deref().unwrap_or_default())
                    )
                },
                began.elapsed().as_millis()
            );
        }
        {
            use tokio::io::AsyncWriteExt;
            writer.write_all(&written(wire, &reply)?).await?;
            writer.flush().await?;
        }
        match then {
            Then::Nothing => {}
            Then::Keep(message) => {
                crate::noted!("serve: message from {} ({:?})", message.from, message.sort);
                let _ = serving.arrived.send(message).await;
            }
            Then::Adopted {
                by,
                handover,
                secret,
            } => {
                crate::noted!("serve: taken on by {by}");
                let _ = serving.adopted.send((by, handover, secret)).await;
            }
            Then::Ask(request) => {
                crate::noted!("serve: {} asks to be taken on", request.from);
                let _ = serving.asked.send(request).await;
            }
            Then::Minted { id, token } => {
                crate::noted!("serve: minted {id}");
                let _ = serving.minted.send((id, token)).await;
            }
            Then::Named(role) => {
                crate::noted!("serve: named {}", role.name);
                let _ = serving.named.send(role).await;
            }
            Then::Stop => {
                crate::noted!("serve: stopped");
                let _ = serving.stopped.send(()).await;
                return Ok(());
            }
        }
    }
}

/// What a frame that was not read, or a reply that could not be written, is answered with: over
/// the cap is `failed`, anything else that will not decode is `refused`.
fn unread(why: &std::io::Error) -> Reply {
    if framing::too_large(why) {
        Reply::no(why.to_string(), crate::wire::Fault::Failed)
    } else {
        Reply::refused(why.to_string())
    }
}

/// A reply as it goes out: one over the cap is replaced by the refusal that says so, and the
/// connection stays in step.
pub(crate) fn written(wire: framing::Wire, reply: &Reply) -> std::io::Result<Vec<u8>> {
    match framing::framed(wire, reply) {
        Err(why) if framing::too_large(&why) => framing::framed(wire, &unread(&why)),
        framed => framed,
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

pub mod callers;

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

    /// A session nothing is vouched to: every dial from this process is a stranger's.
    pub(super) async fn listening(me: &Identity) -> Bound {
        listening_from(me, None).await
    }

    /// A session this test process speaks for, the way a harness speaks for its own `serve`.
    pub(super) async fn listening_as_own(me: &Identity) -> Bound {
        listening_from(me, crate::kin::Origin::here()).await
    }

    async fn listening_from(me: &Identity, origin: Option<crate::kin::Origin>) -> Bound {
        let (about, about_rx) = tokio::sync::watch::channel(About {
            me: me.clone(),
            parent: None,
            token: None,
            busy: false,
            working_for: 0,
            inbox: Vec::new(),
            minted: std::collections::BTreeMap::new(),
            phase: None,
            cause: None,
            spent: Vec::new(),
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
            let _ = serve_from(
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
                origin,
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
    async fn a_message_is_from_whom_the_kernel_names_never_whom_the_call_names() {
        // This process is what the session started, as far as the kernel can tell, and it says
        // `alpha-rho` on every frame: the message is the session's own, never alpha-rho's.
        let it = alone("arrives");
        let them = named(&it, "beta-nu");
        let me = named(&it, "alpha-rho");
        let mut bound = listening_as_own(&them).await;

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
        assert_eq!(message.from, them.full());
        assert_ne!(
            message.from,
            me.full(),
            "the call's own `from` was believed"
        );
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
        let _bound = listening_as_own(&them).await;

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
    async fn a_session_refuses_a_stop_from_itself_at_the_wall() {
        // Two gates stand between a caller and a stop, and this is the outer one: whatever it
        // claims, a process this session speaks for is the session, and none may stop itself.
        let it = alone("wall");
        let them = named(&it, "epsilon-tau");
        let me = named(&it, "zeta-nu");
        let _bound = listening_as_own(&them).await;

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
mod parting;
