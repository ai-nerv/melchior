//! What became of something this session asked another to do. Split from [`super`] under THE
//! RULE, which caps a file at 800 lines.
//!
//! The seven states are A2A's vocabulary minus `auth_required`, and none of its transport. There
//! is no task table: `ask` hands back the id the far end minted for the message plus the id of the
//! agent it went to, and every state is read out of those two facts at the moment somebody asks —
//! the inbox says what came back, the socket says whether anybody is still there, and `status`
//! says whether they are working. The id in the handle is the same id `inbox` tells the far end to
//! quote in `about` when it replies.

use super::{Answer, Standing};
use crate::directory;
use crate::identity::Identity;
use crate::wire::Sort;

/// What separates the two halves of a handle. Not a slash, which would read as an address, and
/// neither an id nor a message id can hold this character, so the split is unambiguous.
const BETWEEN: char = '@';

/// Where a piece of work has got to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// It is in their inbox and they have not started.
    Submitted,
    /// They are mid-turn.
    Working,
    /// They have asked something back and cannot go on until it is answered.
    InputRequired,
    /// They answered.
    Completed,
    /// They said it could not be done, or they are no longer there.
    Failed,
    /// They handed it back without doing it.
    Canceled,
    /// They would not take it at all, so no handle was ever made.
    Rejected,
}

impl State {
    /// The word, as A2A spells it.
    #[must_use]
    pub fn named(self) -> &'static str {
        match self {
            Self::Submitted => "submitted",
            Self::Working => "working",
            Self::InputRequired => "input_required",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
            Self::Rejected => "rejected",
        }
    }

    /// What it means, and what to do next: every one says where to look.
    #[must_use]
    pub fn means(self) -> &'static str {
        match self {
            Self::Submitted => {
                "it is in their inbox and they have not begun. Nothing is owed here: carry on, \
                 and the answer will arrive in this session's inbox"
            }
            Self::Working => "they are mid-turn on it. Ask again later rather than asking twice",
            Self::InputRequired => {
                "they have asked something back and cannot go on until it is answered. It is in \
                 this session's inbox; `reply` to it"
            }
            Self::Completed => "they answered, and the answer is in this session's inbox",
            Self::Failed => {
                "either they said it could not be done, or nothing is listening as them any more \
                 — a session that crashed leaves its socket behind. `crew` says who is there"
            }
            Self::Canceled => {
                "they handed it back without doing it. It is nobody's now, so do it here or hand \
                 it to somebody else"
            }
            Self::Rejected => {
                "they would not take it, so no task was made and there is nothing to ask after"
            }
        }
    }
}

/// The handle `ask` hands back: who it went to, and what they called the message.
#[must_use]
pub fn handle(who: &str, message: &str) -> String {
    format!("{who}{BETWEEN}{message}")
}

/// Read one back, or `None` for something that is not a handle.
#[must_use]
pub fn read(handle: &str) -> Option<(&str, &str)> {
    let (who, message) = handle.trim().split_once(BETWEEN)?;
    (!who.is_empty() && !message.is_empty()).then_some((who, message))
}

/// What to say when a question got as far as an inbox.
#[must_use]
pub fn submitted(who: &Identity, message: &str) -> String {
    format!(
        "task `{}`, state: {} — the question is in `{}`'s inbox and its answer will arrive in \
         this session's, not in this turn. `task` with that handle in `about` says where it has \
         got to.",
        handle(&who.id, message),
        State::Submitted.named(),
        who.full()
    )
}

/// What to add to a refusal, for the verb that would have made a handle. The one state reported at
/// submission rather than polled for: a question the far end would not take never became a task.
#[must_use]
pub fn rejected(verb: &str) -> &'static str {
    if verb == "ask" {
        " state: rejected — no task was made, so there is nothing to ask after."
    } else {
        ""
    }
}

/// Where a handle has got to, worked out from what can be seen.
pub fn reported(arguments: &serde_json::Value, standing: &Standing) -> Answer {
    let about = arguments
        .get("about")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let Some((who, message)) = read(about) else {
        return Answer::refused(format!(
            "`{about}` is not a task handle. `ask` hands one back, and it reads \
             `id{BETWEEN}message-id`."
        ));
    };
    let me = standing.identity();
    let them = Identity {
        id: who.to_owned(),
        ..me.clone()
    };
    let state = stands(message, &them, standing);
    Answer::said(format!(
        "task `{about}` — {}: {}.",
        state.named(),
        state.means()
    ))
}

/// The state of one handle: what came back first, and only then whether anybody is there. A task
/// answered by an agent that has since ended is `completed`, not `failed`.
fn stands(message: &str, them: &Identity, standing: &Standing) -> State {
    if let Some(back) = standing
        .inbox
        .iter()
        .find(|held| held.about.as_deref() == Some(message))
    {
        return match back.sort {
            Sort::Answer => State::Completed,
            Sort::Question => State::InputRequired,
            Sort::Trouble => State::Failed,
            Sort::Release => State::Canceled,
            // They said something about it that was not an answer. They have it and are on it.
            _ => State::Working,
        };
    }
    let me = standing.identity();
    let Ok(mut held) = directory::dial(them, &me) else {
        // A socket file outlives its process, so this is the ordinary answer for an agent that
        // ended mid-task rather than an error.
        return State::Failed;
    };
    // Refused rather than answered is not the task failing: `status` is behind the same wall
    // everything else is, so what is known then is what was known at submission.
    match held.call("status", Vec::new()) {
        Ok(reply) if reply.ok => reply
            .result
            .first()
            .and_then(|said| said.get("busy"))
            .and_then(serde_json::Value::as_bool)
            .map_or(State::Submitted, |busy| {
                if busy {
                    State::Working
                } else {
                    State::Submitted
                }
            }),
        _ => State::Submitted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::Message;

    fn standing() -> Standing {
        Standing {
            me: "magi/main/alpha-rho".to_owned(),
            parent: None,
            forked: Vec::new(),
            minted: std::collections::BTreeMap::new(),
            inbox: Vec::new(),
        }
    }

    fn back(sort: Sort, about: &str) -> Message {
        Message::sent(
            "magi/main/beta-nu",
            "here you are",
            sort,
            Some(about.to_owned()),
        )
    }

    #[test]
    fn a_handle_reads_back_as_the_two_facts_it_was_made_of() {
        let made = handle("beta-nu", "magi-main-alpha-rho-18f2c");
        assert_eq!(read(&made), Some(("beta-nu", "magi-main-alpha-rho-18f2c")));
        assert_eq!(read("magi-main-alpha-rho-18f2c"), None);
        assert_eq!(read(""), None);
        assert_eq!(read("beta-nu@"), None);
    }

    #[test]
    fn what_came_back_decides_the_state() {
        let id = "magi-main-alpha-rho-18f2c";
        for (sort, state) in [
            (Sort::Answer, State::Completed),
            (Sort::Question, State::InputRequired),
            (Sort::Trouble, State::Failed),
            (Sort::Release, State::Canceled),
            (Sort::Note, State::Working),
        ] {
            let mut standing = standing();
            standing.inbox.push(back(sort, id));
            let them = Identity {
                project: "magi".to_owned(),
                role: "main".to_owned(),
                id: "beta-nu".to_owned(),
            };
            assert_eq!(stands(id, &them, &standing), state, "{sort:?}");
        }
    }

    #[test]
    fn an_answer_from_an_agent_that_has_since_gone_is_still_an_answer() {
        let id = "magi-main-alpha-rho-18f2c";
        let mut standing = standing();
        standing.inbox.push(back(Sort::Answer, id));
        let said = reported(
            &serde_json::json!({"about": handle("beta-nu", id)}),
            &standing,
        );
        assert!(!said.failed, "{}", said.said);
        assert!(said.said.contains("completed"), "{}", said.said);
    }

    #[test]
    fn a_task_nobody_is_listening_for_has_failed_rather_than_waiting() {
        let said = reported(
            &serde_json::json!({"about": handle("nobody-nowhere", "magi-main-alpha-rho-1")}),
            &standing(),
        );
        assert!(said.said.contains("failed"), "{}", said.said);
        assert!(
            said.said.contains("crew"),
            "and where to look: {}",
            said.said
        );
    }

    #[test]
    fn something_that_is_not_a_handle_says_what_one_looks_like() {
        let said = reported(&serde_json::json!({"about": "the-parser"}), &standing());
        assert!(said.failed);
        assert!(said.said.contains("ask"), "{}", said.said);
    }

    #[test]
    fn every_state_says_what_to_do_next_and_none_of_them_needs_authenticating() {
        for state in [
            State::Submitted,
            State::Working,
            State::InputRequired,
            State::Completed,
            State::Failed,
            State::Canceled,
            State::Rejected,
        ] {
            assert!(!state.named().is_empty());
            assert!(state.means().len() > 20, "{} says nothing", state.named());
            assert_ne!(state.named(), "auth_required");
        }
    }
}
