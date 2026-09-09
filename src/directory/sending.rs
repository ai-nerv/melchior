//! How much one session may say, and how often.
//!
//! The record is a note beside the socket, `<id>.sent`, because `melchior tool` is one process per
//! call and nothing it counts survives the exit. Two tool processes writing it at once lose an
//! update and the cap then counts one send short, which is the forgiving direction. Nothing is
//! recorded unless the session's own socket answers: a note written for a session that never
//! bound would never be swept, because nothing would ever dial it and find a corpse.

use super::{answers, home, safe, socket};
use crate::identity::Identity;
use crate::wire::{Message, Sort};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

/// How much one message may say, in characters. An inbox holds fifty of them and a model reads
/// the whole of it in one turn.
pub const AT_MOST: usize = 2_000;

/// How many messages one session may send in a window.
pub const IN_A_WINDOW: usize = 24;

/// How long that window is, in milliseconds.
pub const WINDOW: u64 = 60_000;

/// How many peers one fan-out may reach.
pub const AT_ONCE: usize = 24;

/// How many times one piece of work may be handed on. The count rides the message because no
/// single session can see a cycle, and a counter held locally resets at every hop.
pub const HOPS: u32 = 8;

/// What is wrong with the length of a message, if anything.
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

/// How many hops an outgoing handoff carries: the deepest handoff in the inbox plus one, because
/// nothing says which arrival is being passed on and over-counting stops a chain sooner.
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

/// Record one message and say whether it may go. `what` is the verb, who it is aimed at and what
/// it says, so two calls agreeing on all three inside one window are the same send, and the
/// second is refused rather than dropped.
pub fn allow(me: &Identity, what: &str) -> Result<(), String> {
    // Nothing to record for a session that is not listening: the note would outlive every sweep.
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

/// One message, as a number two calls can be compared by, hashed so the note stays a note.
fn marked(what: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    what.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Project;

    /// A project of its own with a socket, so the record is actually kept.
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
        let (project, _bound) = listening("repeat");
        let me = me(&project);
        allow(&me, "send beta-nu the parser is done").expect("the first one goes");
        let why = allow(&me, "send beta-nu the parser is done").expect_err("the second one");
        assert!(why.contains("same message"), "{why}");
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
        // A counter this session held would start at zero every time work came back round.
        assert_eq!(hopped(&[]), 1, "a fresh handoff is the first hop");
        let handed = |hops: u32| {
            let mut message = Message::sent("magi/main/beta-nu", "yours now", Sort::Handoff, None);
            message.hops = hops;
            message
        };
        assert_eq!(hopped(&[handed(3)]), 4);
        // The deepest, not the newest.
        assert_eq!(hopped(&[handed(5), handed(2)]), 6);
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
