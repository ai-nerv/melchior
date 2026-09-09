//! The tool the model calls to reach other instances. This is the interface: naming `$main/delta`
//! in a prompt sends nothing — it tells the model that instance exists and that this tool reaches
//! it, and the model decides what to do.
//!
//! One entry in the tool list rather than eight, so a model does not spend its attention choosing
//! between names that differ by a suffix: `verb` says which, and `help` lists them all.
//!
//! Everything here can be done to anything listening except `stop`. A child is handed a secret in
//! [`crate::inherited::TOKEN`] at spawn, and a `stop` that cannot quote it back is refused however
//! convincing the name on it was. `list` shows what this session can actually reach rather than
//! everything that exists; see [`crate::policy`] for where the walls are.

mod claiming;
pub mod doing;
mod fanning;
pub mod saying;
mod standing;
pub mod tasking;

pub use standing::Standing;

use crate::directory::TOOL;
use serde_json::{Value, json};

/// What a verb produced, for a host to turn into whatever a tool result looks like there. Not a
/// tool trait: this crate would then depend on a harness, so the vocabulary is [`described`] as
/// data and the work is [`answer`].
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

/// The tool this crate offers, as a host needs to declare it. The vocabulary is written once,
/// here, and a harness registers what comes back rather than keeping a second copy of it.
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

/// What the tool can be asked to do. These names and descriptions are the wire contract magi
/// parses, so an edit here is an edit to what every harness sees. Public because `melchior verbs`
/// advertises them under `door: "tool"`: a verb that is answered and unlisted breaks "advertised
/// equals dispatched" from the side nobody checks.
pub const VERBS: &[(&str, &str)] = &[
    // Knowing where you are.
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
    // What an agent is for. No verb, wall or reach reads a role, which is why a session may
    // choose its own.
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
    // Saying things. Nothing here holds the turn open.
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
    // Asking for something. What separates these from `send` is what the far end does on arrival.
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
    // Not treading on each other. A file per claim, advisory and never enforced.
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
    // Reading what came back: the inbox and nothing else. A conversation store is balthasar's.
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

/// Which verbs need an instance named, and which do not. A table rather than a condition per verb,
/// so it cannot disagree with the schema.
const ALONE: &[&str] = &[
    "help", "whoami", "list", "crew", "inbox", "claims", "claim", "release", "announce", "trouble",
    "reply", "role", "task",
];

/// Which verbs name a role, refused here rather than at the far end.
const NAMES_A_ROLE: &[&str] = &["role", "assign"];

/// Which verbs need something said. Also the list charged against what a session may send in a
/// window — see [`doing::perform`] — because it is exactly the verbs that put something in
/// somebody else's inbox. `claim` is not one: a claim is a file, not a message.
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
const QUOTES: &[(&str, &str)] = &[
    ("reply", "the id of the message being answered"),
    ("claim", "a name for the piece of work being taken"),
    ("release", "the name the work was claimed under"),
    ("task", "the handle `ask` gave back"),
];

/// Answer one call. `standing` is what the host knows about this session, handed in rather than
/// reached for, so this stays a function of its arguments.
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
        // No `who`: the message being quoted already says who to answer.
        "reply" => match answering(arguments, standing) {
            Ok(who) => match doing::decide(verb, &who, arguments, standing) {
                Ok(wanted) => doing::perform(&wanted, standing),
                Err(refused) => refused,
            },
            Err(refused) => refused,
        },
        // Aimed at this session, so it takes no `who`; `assign` is the one for somebody else. It
        // still goes over the socket rather than writing the note here, which would be a second
        // writer of what the session holds.
        "role" => {
            let me = standing.identity().id;
            match doing::decide(verb, &me, arguments, standing) {
                Ok(wanted) => doing::perform(&wanted, standing),
                Err(refused) => refused,
            }
        }
        // One decision, many peers, each meeting the same wall a `send` would.
        "announce" | "trouble" => fanning::fanned(verb, arguments, standing),
        // Files, not messages: nothing is sent and nothing is dialled.
        "claim" => claiming::take(arguments, standing),
        "release" => claiming::let_go(arguments, standing),
        "claims" => claiming::held(standing),
        // Derived, never stored.
        "task" => tasking::reported(arguments, standing),
        // The floor: a verb added to `ALONE` and not to the dispatch lands here silently, because
        // this is a match arm and not a missing function. The test below is what notices.
        _ if ALONE.contains(&verb) => Answer::said(format!(
            "`{verb}` is understood but not yet carried out: the socket call it makes is \
                 not wired into the turn loop."
        )),
        _ => match arguments.get("who").and_then(Value::as_str) {
            // Decided first, dialled second: everything worth refusing is refused before the dial.
            Some(who) => match doing::decide(verb, who, arguments, standing) {
                Ok(wanted) => doing::perform(&wanted, standing),
                Err(refused) => refused,
            },
            None => Answer::refused(format!("`{verb}` needs `who` — which instance to reach.")),
        },
    }
}

/// Who a `reply` goes to: whoever sent the message it quotes, looked up rather than asked for,
/// because the id is the authority on who asked. An `about` that names nothing is refused with
/// what the inbox actually holds.
fn answering(arguments: &Value, standing: &Standing) -> Result<String, Answer> {
    let about = arguments
        .get("about")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if let Some(message) = standing.inbox.iter().find(|held| held.id == about) {
        return Ok(message.from.clone());
    }
    // Named explicitly and the id is not one of ours: a session may be answering something it was
    // told about out of band, and the wall is still passed before anything is sent.
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
        // Refused before the round trip.
        let out = call(json!({"verb": "stop", "who": "beta-nu"}), standing());
        assert!(out.failed);
        assert!(out.said.contains("started"), "{}", out.said);
    }

    #[test]
    fn stopping_something_this_session_started_gets_past_both_gates() {
        // Nothing listens in a test, so reaching the socket is the evidence both gates passed.
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
        // The authority is what this session remembers doing, never the far end's own account.
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
#[cfg(test)]
#[path = "verbs/surface.rs"]
mod surface;

/// `reply` finds who to answer from the message it quotes.
#[cfg(test)]
#[path = "verbs/replying.rs"]
mod replying;

/// What an agent is for is said by the agent or by its parent, and read as a claim.
#[cfg(test)]
#[path = "verbs/naming.rs"]
mod naming;
