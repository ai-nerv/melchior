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

/// Take out what the harness has read: by id when it names them, else the oldest beyond the
/// `waiting` it says are still unread.
pub fn consumed(inbox: &mut Vec<Message>, read: Option<&[String]>, waiting: Option<usize>) {
    if let Some(read) = read {
        inbox.retain(|held| !read.contains(&held.id));
    } else if let Some(waiting) = waiting {
        let gone = inbox.len().saturating_sub(waiting);
        inbox.drain(..gone);
    }
}

/// What a session hears from one sender in a window, held here because a sender's own count is
/// the sender's to keep or to skip.
static HEARD: std::sync::Mutex<std::collections::BTreeMap<String, Vec<u64>>> =
    std::sync::Mutex::new(std::collections::BTreeMap::new());

/// Whether one more message from `sender` at `now` is within the rate a sender is held to.
///
/// # Errors
/// The refusal, saying the cap.
pub fn heard_from(sender: &str, now: u64) -> Result<(), String> {
    use crate::directory::sending::{IN_A_WINDOW, WINDOW};
    let Ok(mut heard) = HEARD.lock() else {
        return Ok(());
    };
    heard.retain(|_, times| {
        times.retain(|at| now.saturating_sub(*at) < WINDOW);
        !times.is_empty()
    });
    let times = heard.entry(sender.to_owned()).or_default();
    if times.len() >= IN_A_WINDOW {
        return Err(format!(
            "`{sender}` has sent this session {IN_A_WINDOW} messages in the last minute, which \
             is the most it takes from one sender"
        ));
    }
    times.push(now);
    Ok(())
}

/// One page of the inbox from `skip` on, as rows, never more than `page` bytes: a row that would
/// not fit even alone is cut and marked `truncated`.
#[must_use]
pub fn paged(inbox: &[Message], skip: usize, page: usize) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    let mut size = 0;
    for held in inbox.iter().skip(skip) {
        let mut row = serde_json::json!(held);
        let mut bytes = row.to_string().len();
        if bytes > page {
            let cut: String = held.text.chars().take(page / 8).collect();
            row = serde_json::json!(Message {
                text: cut,
                ..held.clone()
            });
            if let Some(fields) = row.as_object_mut() {
                fields.insert("truncated".to_owned(), serde_json::Value::Bool(true));
            }
            bytes = row.to_string().len();
        }
        if size + bytes > page && !out.is_empty() {
            break;
        }
        size += bytes;
        out.push(row);
    }
    out
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
            phase: None,
            cause: None,
            spent: Vec::new(),
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

/// Server-side limits and the one inbox, consumed by id.
#[cfg(test)]
mod limits {
    use super::*;

    fn message(id: &str) -> Message {
        Message {
            id: id.to_owned(),
            ..Message::new("magi/main/beta-nu", id)
        }
    }

    #[test]
    fn what_the_harness_read_is_taken_out_by_id_or_else_the_oldest_go() {
        let mut inbox = vec![message("a"), message("b"), message("c")];
        consumed(&mut inbox, Some(&["b".to_owned()]), Some(0));
        let left: Vec<&str> = inbox.iter().map(|held| held.id.as_str()).collect();
        assert_eq!(left, ["a", "c"], "by id, and `waiting` is not also applied");
        consumed(&mut inbox, None, Some(1));
        assert_eq!(
            inbox[0].id, "c",
            "the newest unread is kept, not the oldest"
        );
        consumed(&mut inbox, None, None);
        assert_eq!(inbox.len(), 1);
    }

    #[test]
    fn one_sender_is_held_to_the_rate_a_sender_keeps_to() {
        let sender = format!("flood-{}", std::process::id());
        let now = crate::wire::now_ms();
        for _ in 0..crate::directory::sending::IN_A_WINDOW {
            heard_from(&sender, now).expect("within the rate");
        }
        assert!(heard_from(&sender, now).is_err(), "one past the rate");
        let later = now + crate::directory::sending::WINDOW;
        assert!(heard_from(&sender, later).is_ok(), "the window moves on");
    }

    #[test]
    fn an_inbox_is_read_a_page_at_a_time_and_a_huge_row_is_cut_and_marked() {
        let mut inbox: Vec<Message> = (0..10).map(|n| message(&format!("m{n}"))).collect();
        inbox[3].text = "z".repeat(4000);
        let first = paged(&inbox, 0, 600);
        assert!(!first.is_empty() && first.len() < 10, "{}", first.len());
        let mut seen = first.len();
        let mut marked = first.iter().any(|row| row["truncated"] == true);
        while seen < inbox.len() {
            let page = paged(&inbox, seen, 600);
            assert!(!page.is_empty());
            marked |= page.iter().any(|row| row["truncated"] == true);
            seen += page.len();
        }
        assert!(marked, "the oversized row was not marked");
        assert!(paged(&inbox, seen, 600).is_empty());
    }
}
