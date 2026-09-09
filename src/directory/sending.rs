//! How much one session may say, and how often.
//!
//! Split from [`super`] under THE RULE, which caps a file at 800 lines.
//!
//! # Why there are caps at all
//!
//! MAST's two commonest multi-agent failures are step repetition, at 15.7%, and being unaware of
//! termination, at 12.4%. Both look identical from inside one session: an agent doing the right
//! thing, again. Nothing here can tell a deliberate second attempt from the ten-thousandth pass
//! of a loop by *reading* it, so what these numbers do is bound the damage rather than diagnose
//! the cause — a loop that cannot send more than [`IN_A_WINDOW`] messages a minute, and cannot
//! send the same one twice, runs out of ways to be a loop.
//!
//! # The record is a note beside the socket
//!
//! `melchior tool` is one process per call. Nothing it counts survives the exit, so the count is
//! on disk, in `<id>.sent`, beside `<id>.parent` and for the same reason — it is written by the
//! session it belongs to and swept when that session goes.
//!
//! Two tool processes writing it at once lose an update, and the cap then counts one send short.
//! That is the right way for this to be wrong: a rate limit that occasionally forgives is a rate
//! limit, and one that occasionally refuses a message somebody meant is a bug report.
//!
//! # And only for a session that is really there
//!
//! Nothing is recorded unless the session's own socket answers. A note written for a session that
//! never bound would never be swept — nothing would ever dial it, so nothing would ever notice it
//! was a corpse — and the tool process of a session that is not listening has no peers to flood.

use super::{answers, home, safe, socket};
use crate::identity::Identity;
use crate::wire::{Message, Sort};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

/// How much one message may say, in characters.
///
/// Two thousand is a long paragraph — about three hundred words. The number comes from the far
/// end rather than from this one: an inbox holds fifty messages and a model reads the whole of it
/// in one turn, so an uncapped message is one peer deciding how much of somebody else's context
/// it gets. Fifty at this size is already a hundred thousand characters, which is the pathological
/// case and survivable; fifty uncapped is not a number at all.
///
/// It is also the honest bound on what this is *for*. melchior moves a message, not a file: work
/// that needs more than a paragraph to hand over needs a path in the paragraph.
pub const AT_MOST: usize = 2_000;

/// How many messages one session may send in a window.
///
/// The same number as [`AT_ONCE`], because they bound the same thing — how many peers one
/// decision may touch. A session that addressed a whole crew one at a time gets through with
/// nothing to spare and a session in a loop is thousands past it before the first window closes.
pub const IN_A_WINDOW: usize = 24;

/// How long that window is, in milliseconds.
///
/// A minute, because that is roughly one model turn on a loaded machine: the cap should be felt
/// by a session sending on every pass of a loop and not by one that sent a burst, thought, and
/// sent another.
pub const WINDOW: u64 = 60_000;

/// How many peers one fan-out may reach.
///
/// A run bigger than this is one where `announce` is the wrong verb: twenty-four agents each
/// being told the same thing is twenty-four turns spent reading it, and a coordinator that wants
/// more than that wants a roster it addresses in parts.
pub const AT_ONCE: usize = 24;

/// How many times one piece of work may be handed on.
///
/// **This is the cycle guard, and it is the number that has to ride the message.** No single
/// session can see a cycle: A hands to B, B to C, C to A, and each of the three sees one handoff
/// arrive and one leave. A counter held locally is a counter that resets at every hop.
///
/// Eight because a genuine pipeline is three or four stages — plan, do, review, and back once —
/// so eight lets an honest chain run round twice before it is stopped, while the cycles MAST
/// measures are two and three agents long and hit this in seconds.
pub const HOPS: u32 = 8;

/// What is wrong with the length of a message, if anything.
///
/// # Errors
/// When it says more than [`AT_MOST`] characters.
pub fn sized(said: &str) -> Result<(), String> {
    let held = said.chars().count();
    if held > AT_MOST {
        return Err(format!(
            "that message is {held} characters and one may be {AT_MOST}. An inbox holds fifty of \
             these and a model reads all of them: say what the work is and where it is, not what \
             is in it."
        ));
    }
    Ok(())
}

/// How many hops an outgoing handoff carries, given what this session was handed.
///
/// **The deepest handoff in the inbox, plus one.** Not the one being passed on, because nothing
/// says which that is: a model handing work on has read several messages and the tool call names
/// a recipient, not a provenance. Taking the deepest can only over-count, and over-counting stops
/// a chain sooner — which is the safe direction for a guard whose failure mode is a loop that
/// never stops.
#[must_use]
pub fn hopped(inbox: &[Message]) -> u32 {
    inbox
        .iter()
        .filter(|held| held.sort == Sort::Handoff)
        .map(|held| held.hops)
        .max()
        .unwrap_or(0)
        .saturating_add(1)
}

/// Whether a handoff has been passed on too many times.
#[must_use]
pub fn too_far(hops: u32) -> Option<String> {
    if hops <= HOPS {
        return None;
    }
    Some(format!(
        "this piece of work has been handed on {} times and {HOPS} is the limit. A chain that \
         long is a cycle: nobody in it can see the whole ring, which is why the count travels \
         with the work. Do it here, or say in `trouble` why it cannot be done.",
        hops - 1
    ))
}

/// Where the note recording what this session has sent is put.
#[must_use]
pub fn sent_at(project: &str, id: &str) -> PathBuf {
    home(project).join(format!("{}.sent", safe(id)))
}

/// Record one message and say whether it may go.
///
/// `what` is everything that makes two sends the same send: the verb, who it is aimed at, and
/// what it says. Two calls that agree on all three inside one window are a model repeating
/// itself, and the second is refused rather than dropped — silence would leave it believing the
/// message landed, and believing that is how the third one gets sent.
///
/// # Errors
/// When the window is full, or when this exact message has already gone in it.
pub fn allow(me: &Identity, what: &str) -> Result<(), String> {
    // Nothing to record for a session that is not listening: the note would outlive every sweep
    // that could take it down. See the module header.
    if !answers(&socket(&me.project, &me.id)) {
        return Ok(());
    }
    let path = sent_at(&me.project, &me.id);
    let now = crate::wire::now_ms();
    let mark = marked(what);
    let mut held: Vec<(u64, u64)> = read(&path)
        .into_iter()
        .filter(|(at, _)| now.saturating_sub(*at) < WINDOW)
        .collect();
    if let Some((at, _)) = held.iter().find(|(_, said)| *said == mark) {
        return Err(format!(
            "this session sent that same message {}s ago and nothing has changed since. Sending \
             it again is the shape a loop has from the inside — read `inbox` for what came back, \
             or say something different.",
            now.saturating_sub(*at) / 1_000
        ));
    }
    if held.len() >= IN_A_WINDOW {
        return Err(format!(
            "this session has sent {IN_A_WINDOW} messages in the last minute, which is the cap. \
             Nothing is refusing what you are asking for — this is the guard against a handoff \
             that has come back round. Read `inbox`, and use `announce` where you meant to tell \
             the whole crew."
        ));
    }
    held.push((now, mark));
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let written: String = held
        .iter()
        .map(|(at, said)| format!("{at} {said}\n"))
        .collect();
    let _ = std::fs::write(path, written);
    Ok(())
}

/// Take the note back down.
pub fn ended(me: &Identity) {
    let _ = std::fs::remove_file(sent_at(&me.project, &me.id));
}

/// The same, for a corpse being swept by somebody else.
pub fn forget_in(project: &str, id: &str) {
    let _ = std::fs::remove_file(sent_at(project, id));
}

/// What has been sent, as the note holds it.
fn read(path: &std::path::Path) -> Vec<(u64, u64)> {
    let Ok(said) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    said.lines()
        .filter_map(|line| {
            let (at, mark) = line.trim().split_once(' ')?;
            Some((at.parse().ok()?, mark.parse().ok()?))
        })
        .collect()
}

/// One message, as a number two calls can be compared by.
///
/// Hashed rather than kept whole so the note stays a note: fifty sends of a two-thousand-character
/// message would otherwise put a hundred kilobytes in the runtime directory, per session, to
/// answer a question that only needs "was this the same".
fn marked(what: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    what.hash(&mut hasher);
    hasher.finish()
}

/// The caps hold, and the count that matters travels with the message.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Project;

    /// A project of its own, with a socket, so the record is actually kept.
    ///
    /// A guard rather than a name: the line that removed it came after the assertions, so a
    /// failing test left the directory and its socket behind for good — see [`crate::scratch`].
    fn listening(name: &str) -> (Project, std::os::unix::net::UnixListener) {
        let project = Project::new("melchior-sent", name);
        let bound =
            std::os::unix::net::UnixListener::bind(socket(&project, "alpha-rho")).expect("bind");
        (project, bound)
    }

    fn me(project: &str) -> Identity {
        Identity {
            project: project.to_owned(),
            role: "main".to_owned(),
            id: "alpha-rho".to_owned(),
        }
    }

    #[test]
    fn the_same_message_twice_in_a_window_is_refused_and_says_so() {
        // Step repetition, MAST's commonest failure at 15.7%. Refused rather than silently
        // dropped: a session told nothing goes on believing the message landed, and believing
        // that is how the third one gets sent.
        let (project, _bound) = listening("repeat");
        let me = me(&project);
        allow(&me, "send beta-nu the parser is done").expect("the first one goes");
        let why = allow(&me, "send beta-nu the parser is done").expect_err("the second one");
        assert!(why.contains("same message"), "{why}");
        // And a different one still goes, so the guard is about repetition and not about volume.
        allow(&me, "send beta-nu the lexer is done").expect("something new");
    }

    #[test]
    fn a_session_runs_out_of_window_before_it_runs_out_of_peers() {
        let (project, _bound) = listening("burst");
        let me = me(&project);
        for at in 0..IN_A_WINDOW {
            allow(&me, &format!("send beta-nu {at}")).expect("under the cap");
        }
        let why = allow(&me, "send beta-nu one too many").expect_err("over it");
        assert!(why.contains(&IN_A_WINDOW.to_string()), "{why}");
        assert!(why.contains("announce"), "and what to do instead: {why}");
    }

    #[test]
    fn nothing_is_recorded_for_a_session_that_is_not_listening() {
        // The note is swept when the session goes, and nothing sweeps a session nothing dials.
        let project = Project::new("melchior-sent", "absent");
        let me = me(&project);
        for _ in 0..IN_A_WINDOW * 2 {
            allow(&me, "send beta-nu the same thing").expect("nothing to record");
        }
        assert!(!sent_at(&project, "alpha-rho").exists());
    }

    #[test]
    fn the_hop_count_comes_off_the_message_rather_than_out_of_this_session() {
        // The whole point of putting it in the frame. A counter this session held would start at
        // zero every time work came back round, and a cycle is exactly the case where it does.
        assert_eq!(hopped(&[]), 1, "a fresh handoff is the first hop");
        let handed = |hops: u32| {
            let mut message = Message::sent("magi/main/beta-nu", "yours now", Sort::Handoff, None);
            message.hops = hops;
            message
        };
        assert_eq!(hopped(&[handed(3)]), 4);
        // The deepest, not the newest: nothing says which arrival is being passed on, and
        // over-counting stops a chain sooner, which is the safe direction.
        assert_eq!(hopped(&[handed(5), handed(2)]), 6);
        // A note that is not a handoff carries nothing.
        assert_eq!(hopped(&[Message::new("magi/main/beta-nu", "fyi")]), 1);
    }

    #[test]
    fn a_chain_is_refused_at_the_limit_and_told_why_it_cannot_see_the_ring() {
        assert_eq!(too_far(HOPS), None, "the last honest hop was refused");
        let why = too_far(HOPS + 1).expect("one past it");
        assert!(why.contains(&HOPS.to_string()), "{why}");
        assert!(why.contains("cycle"), "{why}");
    }

    #[test]
    fn a_message_longer_than_a_paragraph_is_refused_by_number() {
        assert!(sized(&"z".repeat(AT_MOST)).is_ok());
        let why = sized(&"z".repeat(AT_MOST + 1)).expect_err("an uncapped message");
        assert!(why.contains(&AT_MOST.to_string()), "{why}");
    }

    #[test]
    fn the_note_sits_beside_the_socket_and_is_not_mistaken_for_one() {
        let path = sent_at("magi", "alpha-rho");
        assert_eq!(path.parent(), Some(home("magi").as_path()));
        assert!(
            path.file_name()
                .expect("a name")
                .to_string_lossy()
                .contains('.'),
            "{path:?} would be listed as an agent"
        );
    }
}
