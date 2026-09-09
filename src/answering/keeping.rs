//! What an inbox keeps, and what it lets go of.
//!
//! Split from [`super`] under THE RULE, which caps a file at 800 lines.
//!
//! # The bound is the second line of defence, and it is the one that cannot be lied to
//!
//! [`crate::directory::sending::HOPS`] stops a handoff cycle by counting, and counting works only
//! while everybody in the ring reports honestly — the refusal is at the sender, so a session that
//! sent `hops: 0` on every pass would never refuse itself. This is the half that does not care:
//! an inbox that cannot grow past [`AT_MOST`] gives a loop nowhere to put its next message, and
//! the loop dies of a full queue whether or not the counter was right.
//!
//! MAST names being unaware of termination at 12.4% and step repetition at 15.7%. Neither is
//! something one session can see. A queue that fills is something every session can.
//!
//! # Which one goes
//!
//! The oldest that is not a cry for help, and only then the oldest of all. A flood is made of
//! notes and handoffs — that is what a loop produces — and dropping the oldest of *those* means
//! an `attention` or a `trouble` sits in the inbox until somebody reads it, however much noise
//! arrives afterwards. Dropping the plain oldest would let a loop evict the one message that says
//! the run is in trouble, which is exactly the message the loop makes somebody send.

use crate::wire::Message;

/// How many messages an inbox holds.
///
/// Fifty, and it is a number about reading rather than about memory. A model shown its inbox is
/// shown all of it, so this bounds a turn: fifty messages at
/// [`crate::directory::sending::AT_MOST`] characters is the pathological case and survivable,
/// where unbounded is not a case at all. It is also comfortably more than any real conversation
/// between agents — a run where fifty things are waiting has stopped being a conversation.
pub const AT_MOST: usize = 50;

/// Put a message in an inbox, and make room for it if there is none.
pub fn kept(inbox: &mut Vec<Message>, message: Message) {
    inbox.push(message);
    while inbox.len() > AT_MOST {
        // The oldest that can wait, and only then the oldest of all.
        let at = inbox
            .iter()
            .position(|held| !held.sort.interrupts())
            .unwrap_or(0);
        inbox.remove(at);
    }
}

/// A full inbox drops the noise, and the far end says which message it kept.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::answering::{About, Then, answer};
    use crate::identity::Identity;
    use crate::policy::Whom;
    use crate::wire::{Call, Sort};

    #[test]
    fn an_inbox_stops_growing_and_the_loop_dies_of_a_full_queue() {
        // The half of the cycle guard that a lying sender cannot get round: the hop count is
        // refused at the sender, so a session that reported zero every pass would never refuse
        // itself. This does not ask.
        let mut inbox = Vec::new();
        for at in 0..AT_MOST * 3 {
            kept(
                &mut inbox,
                Message::new("magi/main/beta-nu", &format!("round {at}")),
            );
        }
        assert_eq!(
            inbox.len(),
            AT_MOST,
            "an unbounded inbox is a loop's memory"
        );
        assert!(
            inbox.last().is_some_and(|held| held.text.ends_with("149")),
            "it kept the beginning and dropped the news"
        );
    }

    #[test]
    fn a_flood_of_notes_cannot_evict_a_cry_for_help() {
        // The message a loop makes somebody send is the one the loop must not be able to bury.
        let mut inbox = vec![Message::sent(
            "magi/main/gamma-xi",
            "I am stuck and cannot go on",
            Sort::Trouble,
            None,
        )];
        for at in 0..AT_MOST * 2 {
            kept(
                &mut inbox,
                Message::new("magi/main/beta-nu", &format!("round {at}")),
            );
        }
        assert_eq!(inbox.len(), AT_MOST);
        assert!(
            inbox.iter().any(|held| held.sort == Sort::Trouble),
            "the trouble was dropped to make room for noise"
        );
    }

    #[test]
    fn the_far_end_says_which_message_it_made_of_what_was_sent() {
        // The id is minted where the message lands, so a sender that wanted to ask about it later
        // had no way to name it. It is also the id the far end quotes in `about` when it answers,
        // which is what makes a task handle correlate with nothing else added to the wire.
        let about = About {
            me: Identity {
                project: "magi".to_owned(),
                role: "main".to_owned(),
                id: "alpha-rho".to_owned(),
            },
            parent: None,
            token: None,
            busy: false,
            working_for: 0,
            inbox: Vec::new(),
            minted: std::collections::BTreeMap::new(),
        };
        let them = Whom {
            project: "magi".to_owned(),
            id: "beta-nu".to_owned(),
            parent: None,
            session: None,
        };
        let call = Call {
            call: "tell".to_owned(),
            args: vec![
                serde_json::json!("which parser?"),
                serde_json::json!("question"),
                serde_json::Value::Null,
                serde_json::json!(3),
            ],
            ..Call::default()
        };
        let (reply, then) = answer(&call, &about, Some(&them));
        assert!(reply.ok, "{reply:?}");
        let Then::Keep(message) = then else {
            panic!("it was not kept: {then:?}");
        };
        assert_eq!(reply.result[0]["id"], message.id);
        assert_eq!(message.hops, 3, "the count did not survive the call");
    }
}
