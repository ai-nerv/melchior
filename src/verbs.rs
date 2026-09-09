//! The tool the model calls to reach other instances.
//!
//! This is the interface. Naming `$main/delta` in a prompt does not send anything — it tells the
//! model that instance exists and that this tool reaches it, and the model decides what to do.
//! Whether "tell $gamma to stop" means relay a sentence, ask a question first, or finish reading
//! a file before doing either is the model's judgement, and a harness that acted on the sentence
//! itself would be making that judgement badly and invisibly.
//!
//! # One tool, many verbs
//!
//! One entry in the tool list rather than eight, because a model given `agent_list`,
//! `agent_send`, `agent_status`… spends its attention choosing between names that differ by a
//! suffix. `verb` says which; `help` lists them all, and is the first thing the briefing points
//! at.
//!
//! # Stopping is the one that needs authority
//!
//! Everything here can be done to anything listening except `stop`. A session is stopped by the
//! session that started it and by nothing else — and "is" is not something a caller gets to
//! claim. A child is handed a secret in [`crate::inherited::TOKEN`] at spawn, and a `stop` that
//! cannot quote it back is refused however convincing the name on it was.
//!
//! # Two walls, and what the model is shown
//!
//! `list` shows what this session can actually reach, never everything that exists. A model
//! told about a cousin it will then be refused spends the turn planning around a wall it was
//! never going to get through; see [`crate::policy`] for where the walls are.

mod claiming;
pub mod doing;
mod fanning;
pub mod saying;
mod standing;
pub mod tasking;

pub use standing::Standing;

use crate::directory::TOOL;
use serde_json::{Value, json};

/// What a verb produced, for a host to turn into whatever a tool result looks like there.
///
/// Not a tool trait, and that is the point of the file. This crate does not know what a tool is
/// — a harness does — and the moment it implemented one, it would depend on that harness and
/// could not leave. So the vocabulary is [`described`] as data, the work is [`answer`], and the
/// forty lines that make the two into a tool live on the other side of the boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    /// What to tell the model.
    pub said: String,
    /// Whether it failed. A verb that ran and reported a problem is still an answer.
    pub failed: bool,
}

impl Answer {
    /// It worked.
    #[must_use]
    pub fn said(said: impl Into<String>) -> Self {
        Self {
            said: said.into(),
            failed: false,
        }
    }

    /// It did not, and this is why.
    #[must_use]
    pub fn refused(why: impl Into<String>) -> Self {
        Self {
            said: why.into(),
            failed: true,
        }
    }
}

/// The tool this crate offers, as a host needs to declare it.
///
/// Handed over as data rather than as an implementation, the way aeon publishes its descriptors
/// to magi: the vocabulary is written once, here, and a harness registers what comes back. Two
/// copies of nineteen verb descriptions would disagree the first time one was edited.
#[must_use]
pub fn described() -> Value {
    json!({
        "name": TOOL,
        "description": "Talk to other magi instances. `verb: \"help\"` lists everything this \
                        can do. Instances are named `id`, `role/id` or `project/role/id`; a \
                        bare id means one in this project. Use `list` to find out who is there \
                        and what may be done to each, rather than assuming a name.",
        "parameters": parameters(),
    })
}

/// The JSON Schema for a call.
#[must_use]
pub fn parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "verb": {
                "type": "string",
                "enum": VERBS.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
                "description": "what to do",
            },
            "who": {
                "type": "string",
                "description": "which instance, as `iota-mu`, `review/iota-mu` or \
                                `magi/review/iota-mu`. Not needed by `help`, `list`, `crew` \
                                or `inbox`.",
            },
            "message": {
                "type": "string",
                "description": "what to say. Needed by send, ask, reply, announce, \
                                attention, trouble and handoff.",
            },
            "about": {
                "type": "string",
                "description": "what is being named: the id of the message being answered \
                                (`reply` — `inbox` lists them), the piece of work being taken \
                                or let go (`claim`, `release`), or the handle `ask` gave back \
                                (`task`).",
            },
            "role": {
                "type": "string",
                "description": "for `role` and `assign`, what to call the role — one word, \
                                like `reviewer`. `message` then says what it does, in a \
                                sentence, which is what a coordinator reads to pick somebody.",
            },
            "sort": {
                "type": "string",
                "enum": ["note", "question", "answer", "attention", "claim", "release",
                         "handoff", "trouble"],
                "description": "for `send` only, what kind of message it is. The other \
                                verbs each mean one already. Defaults to note.",
            },
        },
        "required": ["verb"],
    })
}

/// What the tool can be asked to do.
///
/// Extensive on purpose. The narrow version — send, and stop — makes a model that wants to know
/// whether a sibling is even alive send it a message and wait to see what happens.
const VERBS: &[(&str, &str)] = &[
    // Knowing where you are. A subagent that does not know it is one cannot behave like one.
    ("help", "list these verbs and what each takes"),
    (
        "whoami",
        "this session's own name, who started it, and what it started",
    ),
    (
        "list",
        "every instance listening, and how each relates to this one",
    ),
    (
        "crew",
        "everyone in this session: the root that started it and everything under it",
    ),
    (
        "about",
        "who an instance is: project, role, id, and who started it",
    ),
    // What an agent is for. Descriptive and nothing else: no verb, no wall and no reach reads a
    // role, which is the only reason it is safe to let a session choose its own.
    (
        "role",
        "say what this session is for, so the crew list can route work to it",
    ),
    (
        "assign",
        "say what a subagent this session started is for — its role, not its orders",
    ),
    (
        "status",
        "whether an instance is working, for how long, and what is waiting for it",
    ),
    (
        "verbs",
        "what an instance says it can answer, asked of it rather than assumed",
    ),
    // Saying things. Both return at once: nothing here holds the turn open, and `ask` differs
    // from `send` only in the sort it stamps on the message and in what the far end owes back.
    ("send", "put a note in an instance's inbox and carry on"),
    (
        "ask",
        "put a question in an instance's inbox; its answer arrives in this session's inbox \
         later, not in this turn",
    ),
    (
        "reply",
        "answer a question that was asked of this session, quoting its id",
    ),
    (
        "announce",
        "put the same note in the inbox of everyone else in this session's run",
    ),
    (
        "task",
        "where something this session asked for has got to, by the handle `ask` gave back",
    ),
    (
        "adopt",
        "ask an instance to become this session's parent -- a person there has to accept",
    ),
    // Asking for something. The difference between these and `send` is what the far end does
    // when it arrives, which is why they are verbs rather than a wording choice.
    (
        "attention",
        "tell an instance you need it — the one message allowed to interrupt a turn",
    ),
    (
        "trouble",
        "tell everyone else in this session's run that something is wrong and this session \
         cannot go on — it interrupts them",
    ),
    (
        "handoff",
        "give a piece of work to an instance: it is theirs now, not copied",
    ),
    // Not treading on each other. A file per claim, and advisory: melchior records one, it does
    // not enforce it — nothing here knows what the work is.
    (
        "claim",
        "record a piece of work as this session's, naming it in `about`, so others can see it \
         is taken before they start it",
    ),
    (
        "release",
        "let a claimed piece of work go, naming it in `about` exactly as `claim` did",
    ),
    ("claims", "what every instance has said it is working on"),
    // Reading what came back. The inbox and nothing else: a `history` verb sat here, fell
    // through to `tell` and sent an empty message, and the store it wanted is balthasar's.
    // Growing a second one here is the dependency this family exists to prevent.
    (
        "inbox",
        "what has been sent to this session and not yet acted on",
    ),
    // Lifetime.
    (
        "stop",
        "end an instance this session started — refused for any it did not",
    ),
];

/// Which verbs need an instance named, and which do not.
///
/// A table rather than a condition per verb, because the third one written by hand disagreed
/// with the schema and the model was told `whoami` needed a `who`.
const ALONE: &[&str] = &[
    "help", "whoami", "list", "crew", "inbox", "claims", "claim", "release", "announce", "trouble",
    "reply", "role", "task",
];

/// Which verbs name a role, and what goes wrong when one does not.
///
/// A table for the same reason [`ALONE`] and [`SPEAKS`] are. `assign` without a name is a parent
/// telling a child it is for something and not saying what, which reaches the far end as a
/// refusal the model has to guess its way out of.
const NAMES_A_ROLE: &[&str] = &["role", "assign"];

/// Which verbs need something said.
///
/// This is also the list charged against what a session may send in a window — see
/// [`doing::perform`] — because it is exactly the list of verbs that put something in somebody
/// else's inbox. `claim` is no longer on it: a claim is a file the crew reads, not a message
/// anybody is sent.
const SPEAKS: &[&str] = &[
    "send",
    "ask",
    "reply",
    "announce",
    "attention",
    "trouble",
    "handoff",
];

/// Which verbs quote something by id, and what that id is.
///
/// A table for the same reason [`ALONE`] and [`SPEAKS`] are. `release` was in none of them, so
/// it named nothing and said nothing: it reached the far end as a `release` with an empty body,
/// which tells whoever reads it that something was let go but not what.
const QUOTES: &[(&str, &str)] = &[
    ("reply", "the id of the message being answered"),
    ("claim", "a name for the piece of work being taken"),
    ("release", "the name the work was claimed under"),
    ("task", "the handle `ask` gave back"),
];

/// Answer one call.
///
/// `standing` is what the host knows about this session: its name, who started it, what it
/// started, and what has arrived. Handed in rather than reached for, so this stays a function
/// of its arguments and the host keeps the state.
#[must_use]
pub fn answer(arguments: &Value, standing: &Standing) -> Answer {
    let verb = arguments.get("verb").and_then(Value::as_str).unwrap_or("");
    if verb.is_empty() {
        return Answer::refused(format!(
            "{TOOL} needs a verb. Call it with `verb: \"help\"` to see them."
        ));
    }
    if !VERBS.iter().any(|(name, _)| *name == verb) {
        return Answer::refused(format!(
            "`{verb}` is not one of {TOOL}'s verbs. Call it with `verb: \"help\"`."
        ));
    }
    // What a verb needs is a table, not a condition per verb: the third one written by hand
    // disagreed with the schema and told the model `whoami` wanted a `who`.
    let said = arguments.get("message").and_then(Value::as_str);
    if SPEAKS.contains(&verb) && said.is_none_or(str::is_empty) {
        return Answer::refused(format!("`{verb}` needs `message` — what to say."));
    }
    if let Some((_, what)) = QUOTES.iter().find(|(name, _)| *name == verb)
        && arguments
            .get("about")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
    {
        return Answer::refused(format!(
            "`{verb}` needs `about` — {what}. `inbox` lists them."
        ));
    }
    if NAMES_A_ROLE.contains(&verb)
        && arguments
            .get("role")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
    {
        return Answer::refused(format!(
            "`{verb}` needs `role` — one word to call it. `message` says what it does, in a \
             sentence, and that is what a coordinator reads."
        ));
    }
    match verb {
        "help" => Answer::said(saying::help(standing)),
        "whoami" => Answer::said(saying::whoami(standing)),
        "list" => Answer::said(saying::list(standing)),
        "crew" => Answer::said(saying::crew(standing)),
        "inbox" => Answer::said(saying::inbox(standing)),
        // Answered without a `who`, because the message being quoted already says who to answer.
        // Asking for both would have a model look up something it just handed over, and the two
        // could disagree — an answer addressed to somebody who never asked.
        "reply" => match answering(arguments, standing) {
            Ok(who) => match doing::decide(verb, &who, arguments, standing) {
                Ok(wanted) => doing::perform(&wanted, standing),
                Err(refused) => refused,
            },
            Err(refused) => refused,
        },
        // Aimed at this session and nowhere else, so it takes no `who`: the verb for saying what
        // *somebody else* is for is `assign`, and it is a different verb because it needs a
        // different relation. It still goes through the socket rather than writing the note here
        // — the session holds the answer every other agent reads, and a tool process that wrote
        // it directly would be a second writer with no reason to agree.
        "role" => {
            let me = standing.identity().id;
            match doing::decide(verb, &me, arguments, standing) {
                Ok(wanted) => doing::perform(&wanted, standing),
                Err(refused) => refused,
            }
        }
        // One decision, many peers, and each of them meets the same wall a `send` would. Fanning
        // out is not a way round a refusal.
        "announce" | "trouble" => fanning::fanned(verb, arguments, standing),
        // Files, not messages. The record is how the rest of the crew finds out, so nothing is
        // sent and nothing is dialled — see [`crate::directory::claims`].
        "claim" => claiming::take(arguments, standing),
        "release" => claiming::let_go(arguments, standing),
        "claims" => claiming::held(standing),
        // Derived, never stored. A task table would need a reaper and would be wrong between
        // reaps, and the one thing a coordinator must not be told is that a dead agent is busy.
        "task" => tasking::reported(arguments, standing),
        // Kept as the floor rather than deleted with the last stub: a verb added to `ALONE` and
        // not to the dispatch lands here silently, because this is a match arm and not a missing
        // function. The test below is what notices.
        _ if ALONE.contains(&verb) => Answer::said(format!(
            "`{verb}` is understood but not yet carried out: the socket call it makes is \
                 not wired into the turn loop."
        )),
        _ => match arguments.get("who").and_then(Value::as_str) {
            // Decided first, dialled second. Everything worth refusing is refused before
            // the round trip, so a model that asked for something it may not have is told
            // what it may do instead of paying for the answer.
            Some(who) => match doing::decide(verb, who, arguments, standing) {
                Ok(wanted) => doing::perform(&wanted, standing),
                Err(refused) => refused,
            },
            None => Answer::refused(format!("`{verb}` needs `who` — which instance to reach.")),
        },
    }
}

/// Who a `reply` goes to: whoever sent the message it quotes.
///
/// Looked up rather than asked for. A model that has just read its inbox has the id in hand, and
/// making it also name the sender is an invitation to answer the wrong session — the id is the
/// authority on who asked, so it is the only thing this takes.
///
/// An `about` that names nothing is refused with what the inbox actually holds. It is the likely
/// mistake: an id invented, or one from a message already acted on, and "no such message" alone
/// leaves a model with nowhere to go.
fn answering(arguments: &Value, standing: &Standing) -> Result<String, Answer> {
    let about = arguments
        .get("about")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if let Some(message) = standing.inbox.iter().find(|held| held.id == about) {
        return Ok(message.from.clone());
    }
    // Named explicitly, and the id is not one of ours: let it through rather than refuse. A
    // session may be answering something it was told about out of band, and the wall still has
    // to be passed before anything is sent.
    if let Some(who) = arguments
        .get("who")
        .and_then(Value::as_str)
        .filter(|who| !who.is_empty())
    {
        return Ok(who.to_owned());
    }
    if standing.inbox.is_empty() {
        return Err(Answer::refused(format!(
            "nothing has been sent to this session, so there is no `{about}` to answer. \
             `send` or `ask` reaches an instance that has not written first."
        )));
    }
    let held: Vec<String> = standing
        .inbox
        .iter()
        .map(|message| format!("`{}` from {}", message.id, message.from))
        .collect();
    Err(Answer::refused(format!(
        "`{about}` is not a message in this session's inbox. It holds: {}.",
        held.join(", ")
    )))
}

/// The tool refuses what it should and asks for what it needs.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Identity;
    use crate::policy::Relation;
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

    fn call(arguments: Value, standing: Standing) -> Answer {
        answer(&arguments, &standing)
    }

    #[test]
    fn help_lists_every_verb() {
        // The first thing the briefing points the model at, so it has to be complete.
        let out = call(json!({"verb": "help"}), standing());
        assert!(!out.failed);
        assert!(
            out.said.contains("magi/main/alpha-rho"),
            "it never says who we are"
        );
        for (verb, _) in VERBS {
            assert!(out.said.contains(verb), "{verb} is missing from help");
        }
    }

    #[test]
    fn the_schema_offers_exactly_the_verbs_that_exist() {
        // A model told about a verb the tool does not have spends a call finding out.
        let schema = parameters();
        let offered = schema["properties"]["verb"]["enum"]
            .as_array()
            .expect("an enum")
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        let held: Vec<&str> = VERBS.iter().map(|(name, _)| *name).collect();
        assert_eq!(offered, held);
    }

    #[test]
    fn a_verb_that_needs_an_instance_says_so_when_it_has_none() {
        let out = call(json!({"verb": "status"}), standing());
        assert!(out.failed);
        assert!(out.said.contains("who"), "{}", out.said);
    }

    #[test]
    fn help_and_inbox_need_nobody() {
        for verb in ["help", "inbox"] {
            let out = call(json!({"verb": verb}), standing());
            assert!(!out.failed, "{verb}: {}", out.said);
        }
    }

    #[test]
    fn stopping_something_this_session_did_not_start_is_refused_here() {
        // Refused before the round trip, so the model is told what it may do instead of
        // spending a turn discovering it.
        let out = call(json!({"verb": "stop", "who": "beta-nu"}), standing());
        assert!(out.failed);
        assert!(out.said.contains("started"), "{}", out.said);
    }

    #[test]
    fn stopping_something_this_session_started_gets_past_both_gates() {
        // Two gates, and this proves it clears both: the relation says it is this session's
        // child, and the secret says this session is the one that started it. What stops it
        // here is the socket, because nothing is listening in a test — and that failure
        // arriving *is* the evidence, since a refusal would have come before the dial.
        let mut standing = standing();
        standing.forked.push("iota-mu".to_owned());
        standing
            .minted
            .insert("iota-mu".to_owned(), "s3cret".to_owned());
        let out = call(json!({"verb": "stop", "who": "iota-mu"}), standing);
        assert!(out.failed);
        assert!(
            out.said.contains("nothing is listening"),
            "it was refused before it got to the socket: {}",
            out.said
        );
    }

    #[test]
    fn a_child_is_what_this_session_started_and_not_what_a_name_looks_like() {
        // The authority comes from what this session remembers doing, never from the far end's
        // description of itself. A child that declined to leave its note beside its socket
        // would otherwise have made itself unstoppable by forgetting who its parent was.
        let mut standing = standing();
        standing.forked.push("iota-mu".to_owned());
        let child = Identity {
            project: "magi".to_owned(),
            role: "main".to_owned(),
            id: "iota-mu".to_owned(),
        };
        let stranger = Identity {
            project: "magi".to_owned(),
            role: "main".to_owned(),
            id: "beta-nu".to_owned(),
        };
        assert_eq!(standing.stands(&child), Relation::Child);
        assert_ne!(standing.stands(&stranger), Relation::Child);
    }

    #[test]
    fn nothing_in_another_project_is_a_child_however_it_was_recorded() {
        // The project wall wins over local memory, so a stale entry cannot reach across it.
        let mut standing = standing();
        standing.forked.push("other/iota-mu".to_owned());
        let across = Identity {
            project: "other".to_owned(),
            role: "main".to_owned(),
            id: "iota-mu".to_owned(),
        };
        assert_eq!(standing.stands(&across), Relation::Elsewhere);
    }

    #[test]
    fn sending_without_anything_to_say_is_refused() {
        for verb in ["send", "ask"] {
            let out = call(json!({"verb": verb, "who": "gamma"}), standing());
            assert!(out.failed, "{verb} sent nothing");
            assert!(out.said.contains("message"), "{}", out.said);
        }
    }

    #[test]
    fn an_unknown_verb_points_at_help() {
        let out = call(json!({"verb": "obliterate", "who": "gamma"}), standing());
        assert!(out.failed);
        assert!(out.said.contains("help"), "{}", out.said);
    }

    #[test]
    fn a_name_nothing_can_have_says_what_a_name_looks_like() {
        let out = call(json!({"verb": "status", "who": "a/b/c/d"}), standing());
        assert!(out.failed);
        assert!(out.said.contains("project/role/id"), "{}", out.said);
    }

    #[test]
    fn the_inbox_names_who_sent_what() {
        let mut standing = standing();
        standing
            .inbox
            .push(Message::new("magi/main/gamma", "the parser is fixed"));
        let out = call(json!({"verb": "inbox"}), standing);
        assert!(out.said.contains("magi/main/gamma"), "{}", out.said);
        assert!(out.said.contains("the parser is fixed"));
    }
}

/// The wider surface: what each verb needs, and what it means when it lands.
///
/// Split from this file under THE RULE, which caps a file at 800 lines.
#[cfg(test)]
#[path = "verbs/surface.rs"]
mod surface;

/// `reply` finds who to answer from the message it quotes.
///
/// Split from this file under THE RULE, which caps a file at 800 lines.
#[cfg(test)]
#[path = "verbs/replying.rs"]
mod replying;

/// What an agent is for is said by the agent or by its parent, and read as a claim.
#[cfg(test)]
#[path = "verbs/naming.rs"]
mod naming;
