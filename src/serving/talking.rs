//! One connection, for as many calls as it cares to make, under the family's deadlines: five
//! minutes idle between calls, thirty seconds for the rest of a frame once its first byte has
//! come, and thirty seconds to write a reply. Each call is answered off the async loop, because
//! answering one may dial every peer in the project.

use super::{Serving, callers};
use crate::answering::{About, Then, answer};
use crate::framing;
use crate::wire::{Arriving, Call, Reply};
use std::time::Duration;
use tokio::io::AsyncWriteExt;

/// How long a connection may sit idle between calls.
pub(super) const IDLE: Duration = Duration::from_secs(300);

/// How long the rest of a frame may take once it has begun, and a reply may take to be written.
pub(super) const FRAME: Duration = Duration::from_secs(30);

/// Serve one connection until it hangs up, goes quiet, or stops this session.
pub(super) async fn talk(
    stream: tokio::net::UnixStream,
    pinned: callers::Pinned,
    serving: Serving,
) -> std::io::Result<()> {
    let (mut reader, mut writer) = stream.into_split();
    loop {
        let (arriving, wire): (Arriving, framing::Wire) =
            match framing::read_wire_within(&mut reader, IDLE, FRAME).await {
                Ok(pair) => pair,
                // Hanging up, or going quiet, is not a mistake that needs a reply.
                Err(why)
                    if matches!(
                        why.kind(),
                        std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    return Ok(());
                }
                // A malformed or oversized frame still gets an answer, then the connection ends:
                // a body that will not be read cannot be skipped and stay in step.
                Err(why) => {
                    let _ = tokio::time::timeout(FRAME, framing::write(&mut writer, &unread(&why)))
                        .await;
                    return Ok(());
                }
            };
        // A call from a newer family is refused by name and the line stays in step.
        let (reply, then) = if let Some(refusal) = arriving.newer() {
            (refusal, Then::Nothing)
        } else {
            let call = arriving.call;
            let about = serving.about.borrow().clone();
            let asked = pinned.clone();
            tokio::task::spawn_blocking(move || answered(&call, about, &asked))
                .await
                .map_err(std::io::Error::other)?
        };
        let bytes = written(wire, &reply)?;
        let wrote = async {
            writer.write_all(&bytes).await?;
            writer.flush().await
        };
        match tokio::time::timeout(FRAME, wrote).await {
            Ok(Ok(())) => {}
            Ok(Err(why)) => return Err(why),
            Err(_) => return Ok(()),
        }
        if deliver(then, &serving).await {
            return Ok(());
        }
    }
}

/// Answer one call as the connection was pinned, and say so in the log.
pub(super) fn answered(call: &Call, mut about: About, pinned: &callers::Pinned) -> (Reply, Then) {
    // This session's own parent changes from outside, so it is re-read per call.
    about.parent = crate::directory::parent_of(&about.me);
    let caller = callers::caller_of(call, pinned, &about);
    let began = std::time::Instant::now();
    let (reply, then) = answer(call, &about, caller.as_ref());
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
    (reply, then)
}

/// Hand what a call asked for to the loop that owns it. True when the session was stopped.
async fn deliver(then: Then, serving: &Serving) -> bool {
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
            if serving.asked.try_send(request).is_err() {
                crate::noted!("serve: a request to be taken on was dropped: too many are waiting");
            }
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
            return true;
        }
    }
    false
}

/// The same, from a thread that is not the loop's: a call this session made to itself.
pub(super) fn deliver_here(then: Then, serving: &Serving) {
    let handle = tokio::runtime::Handle::try_current();
    match handle {
        Ok(handle) => {
            let serving = serving.clone();
            handle.spawn(async move {
                deliver(then, &serving).await;
            });
        }
        Err(_) => crate::noted!("serve: a call to itself had nowhere to go"),
    }
}

/// What a frame that was not read, or a reply that could not be written, is answered with: over
/// the cap is `failed`, anything else that will not decode is `refused`.
pub(super) fn unread(why: &std::io::Error) -> Reply {
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
