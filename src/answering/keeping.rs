//! What an inbox keeps, and what it lets go of.
//!
//! The bound is the half of the cycle guard that a sender cannot lie to:
//! [`crate::directory::sending::HOPS`] is counted and refused at the sender, so an inbox that
//! cannot grow past [`AT_MOST`] is what stops a ring whose members all report `hops: 0`.

use crate::wire::Message;

/// How many messages an inbox holds; a model shown its inbox is shown all of it, so this bounds
/// a turn.
pub const AT_MOST: usize = 50;

/// Put a message in an inbox, dropping the oldest message that is not a cry for help — and only
/// then the oldest of all — to make room.
pub fn kept(inbox: &mut Vec<Message>, message: Message) {
    inbox.push(message);
    while inbox.len() > AT_MOST {
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
        // The id is minted where the message lands, and is the one the far end quotes in `about`.
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
            adopted_token: None,
        };
        let them = Whom {
            project: "magi".to_owned(),
            id: "beta-nu".to_owned(),
            parent: None,
            session: None,
            root: None,
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
