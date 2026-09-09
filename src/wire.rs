//! What one instance says to another: every magi both listens and dials, and both halves are
//! these types. Three transports, one encoding — argv answers one JSON object on stdout, a pipe
//! carries newline-delimited JSON both ways, and a socket carries four bytes of big-endian
//! length then JSON.
//!
//! ```text
//! -> {"call":"status","args":[]}
//! <- {"ok":true,"n":1,"result":[{"busy":false,…}]}
//! ```
//!
//! Two shapes. A call is answered, and carries the revision it is written in:
//!
//! ```text
//! -> {"call":"status","args":[]}
//! <- {"ok":true,"family":1,"n":1,"result":[{"busy":false}]}
//! ```
//!
//! An event is not:
//!
//! ```text
//! {"event":"listening","at":"…"}
//! ```
//!
//! `result` is a list and `n` says how long it is: a sibling that unpacks a list reads a bare
//! value as nothing at all. A reader refuses a `family` it does not know and tolerates one it
//! predates. A refused call is a reply, `{"ok":false,"error":…}`, not a dropped connection. The
//! tag key is `event` everywhere, in both directions; `scripts/gate-wire.sh` refuses any other.

use serde::{Deserialize, Serialize};

/// One call, as it arrives: `call` and `args` are the family shape, and `from` and `token` are
/// melchior's own, both optional so a sibling poking the socket still gets an answer to `verbs`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Call {
    pub call: String,
    #[serde(default)]
    pub args: Vec<serde_json::Value>,
    /// Who is calling, as `project/role/id`, taken at face value for everything but `stop`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// The secret handed down at spawn, for the one verb that needs proof.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

/// Which revision of the family wire this speaks, carried on every reply rather than negotiated,
/// and bumped only when a consumer that does not know about a change would misread a reply.
pub const FAMILY: u16 = 1;

/// The revision of the registrar surface a plugin file is written against — the registrar names,
/// the fields each declaration owes, what a callback is handed — reported on `verbs` beside
/// `family`, and bumped only when something already published stops working. See EXTENDING.md.
pub const SURFACE: u16 = 1;

/// One reply, as it goes back. Built through [`Reply::of`] and [`Reply::refused`] rather than by
/// hand, so the `n`/`result` invariant holds in one place.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Reply {
    pub ok: bool,
    /// See [`FAMILY`]. Defaulted on the way in, so a reply from a peer built before this existed
    /// reads as `0` rather than failing to parse.
    #[serde(default = "family")]
    pub family: u16,
    /// How many values came back. Always `result.len()`.
    #[serde(default)]
    pub n: usize,
    #[serde(default)]
    pub result: Vec<serde_json::Value>,
    /// Why not, when `ok` is false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn family() -> u16 {
    FAMILY
}

impl Reply {
    #[must_use]
    pub fn of(value: serde_json::Value) -> Self {
        Self {
            ok: true,
            family: FAMILY,
            n: 1,
            result: vec![value],
            error: None,
        }
    }

    /// An answer of none, for a verb that does something rather than reporting something.
    #[must_use]
    pub fn done() -> Self {
        Self {
            ok: true,
            family: FAMILY,
            n: 0,
            result: Vec::new(),
            error: None,
        }
    }

    /// A refusal, which is still a reply.
    #[must_use]
    pub fn refused(why: impl Into<String>) -> Self {
        Self {
            ok: false,
            family: FAMILY,
            n: 0,
            result: Vec::new(),
            error: Some(why.into()),
        }
    }
}

/// The verbs an instance answers on the socket; [`CLI_VERBS`] is what the command line answers,
/// and the two are not the same set. `mint`, `minted` and `stop` are listed here and refused to
/// every caller but the session itself: a verb that is answered and unlisted breaks "advertised
/// equals dispatched" from the side nobody checks.
pub const VERBS: &[(&str, &str)] = &[
    ("verbs", "what this instance answers"),
    (
        "client",
        "the Lua client library for this surface, as source",
    ),
    ("identity", "its project, role, id and who started it"),
    ("needs", "what a coordinator may tell it, as declarations"),
    (
        "kin",
        "how the caller stands to it: parent, child, sibling, kin, main, cousin",
    ),
    (
        "status",
        "whether it is working, for how long, and what is waiting",
    ),
    ("inbox", "messages it has been sent and not yet acted on"),
    ("tell", "put a message of any sort in its inbox"),
    (
        "role",
        "say what it is for: its own, or its parent's word for it",
    ),
    (
        "adopt",
        "ask it to become this session's parent — a person there has to accept",
    ),
    (
        "adopted",
        "tell it a parent has taken it on, and hand over what that parent lends",
    ),
    (
        "mint",
        "name a child and mint the secret that will stop it — this session's own",
    ),
    (
        "minted",
        "the secrets this session minted for what it started, by id — this session's own",
    ),
    (
        "stop",
        "end it — only from the session that started it, with the secret it was given",
    ),
];

/// What a message is for. One inbox, sorted by what each thing is, rather than a channel per
/// kind; the sort travels on the wire in lowercase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Sort {
    #[default]
    Note,
    /// A question, which expects an [`Sort::Answer`] quoting its id.
    Question,
    /// An answer to a question, with `about` naming it.
    Answer,
    /// "I need you." The one that is allowed to interrupt.
    Attention,
    Claim,
    Release,
    /// A piece of work moved, not copied.
    Handoff,
    /// Something is wrong and the sender cannot go on.
    Trouble,
}

impl Sort {
    #[must_use]
    pub fn read(name: &str) -> Option<Self> {
        Some(match name {
            "note" => Self::Note,
            "question" => Self::Question,
            "answer" => Self::Answer,
            "attention" => Self::Attention,
            "claim" => Self::Claim,
            "release" => Self::Release,
            "handoff" => Self::Handoff,
            "trouble" => Self::Trouble,
            _ => return None,
        })
    }

    /// Whether this is meant to reach somebody who is mid-turn. Everything else waits.
    #[must_use]
    pub fn interrupts(self) -> bool {
        matches!(self, Self::Attention | Self::Trouble)
    }

    #[must_use]
    pub fn expects_an_answer(self) -> bool {
        matches!(self, Self::Question)
    }
}

/// A message from one instance to another.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Message {
    pub id: String,
    /// Who sent it, as `project/role/id`.
    pub from: String,
    #[serde(default)]
    pub sort: Sort,
    pub text: String,
    /// The message this one is about, for an answer or a release.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    /// How many times this piece of work has been handed on, for a [`Sort::Handoff`]. It rides
    /// the message because a count kept locally would restart at zero on every pass and never
    /// close a ring; zero for everything else, and defaulted on the way in.
    #[serde(default, skip_serializing_if = "is_first")]
    pub hops: u32,
    /// When, in milliseconds since the epoch.
    pub at: u64,
}

fn is_first(hops: &u32) -> bool {
    *hops == 0
}

impl Message {
    /// A plain note from `from`.
    #[must_use]
    pub fn new(from: &str, text: &str) -> Self {
        Self::sent(from, text, Sort::Note, None)
    }

    /// A message of any sort, stamped now.
    #[must_use]
    pub fn sent(from: &str, text: &str, sort: Sort, about: Option<String>) -> Self {
        let at = now_ms();
        Self {
            id: format!("{}-{at:x}", &from.replace('/', "-")),
            from: from.to_owned(),
            sort,
            text: text.to_owned(),
            about,
            hops: 0,
            at,
        }
    }

    /// The same, having been handed on this many times.
    #[must_use]
    pub fn carried(self, hops: u32) -> Self {
        Self { hops, ..self }
    }
}

#[must_use]
pub fn now_ms() -> u64 {
    u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_millis()),
    )
    .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reply_carries_a_list_and_says_how_long_it_is() {
        let reply = Reply::of(serde_json::json!({"busy": false}));
        let json = serde_json::to_value(&reply).expect("encodes");
        assert!(json["result"].is_array(), "result must be a list: {json}");
        assert_eq!(json["n"], 1);
    }

    #[test]
    fn n_is_always_the_length_of_the_result() {
        for reply in [
            Reply::of(serde_json::json!("one")),
            Reply::done(),
            Reply::refused("no such call: nope"),
        ] {
            assert_eq!(reply.n, reply.result.len(), "{reply:?}");
        }
    }

    #[test]
    fn a_refusal_is_a_reply_and_says_why() {
        let reply = Reply::refused("no such call: nope");
        assert!(!reply.ok);
        assert_eq!(reply.error.as_deref(), Some("no such call: nope"));
        let json = serde_json::to_value(&reply).expect("encodes");
        assert_eq!(json["ok"], false);
    }

    #[test]
    fn a_successful_reply_carries_no_error_field_at_all() {
        let json = serde_json::to_value(Reply::done()).expect("encodes");
        assert!(json.get("error").is_none(), "{json}");
    }

    #[test]
    fn a_call_with_no_arguments_still_reads() {
        let call: Call = serde_json::from_str(r#"{"call":"status"}"#).expect("reads");
        assert_eq!(call.call, "status");
        assert!(call.args.is_empty());
    }

    #[test]
    fn a_reply_survives_the_round_trip() {
        let sent = Reply::of(serde_json::json!({"id": "gamma"}));
        let text = serde_json::to_string(&sent).expect("encodes");
        let back: Reply = serde_json::from_str(&text).expect("decodes");
        assert_eq!(sent, back);
    }

    #[test]
    fn every_verb_is_named_once_and_described() {
        let mut names: Vec<&str> = VERBS.iter().map(|(name, _)| *name).collect();
        names.sort_unstable();
        let held = names.len();
        names.dedup();
        assert_eq!(names.len(), held, "a verb is listed twice");
        assert!(names.contains(&"verbs"), "it must say what it speaks");
        for (name, said) in VERBS {
            assert!(!said.is_empty(), "{name} has no description");
        }
    }

    #[test]
    fn nothing_that_runs_a_command_is_in_the_first_cut() {
        for (name, _) in VERBS {
            assert!(
                !["run", "shell", "exec", "eval", "tool"].contains(name),
                "{name} does not belong on a socket"
            );
        }
    }

    #[test]
    fn a_hop_count_survives_the_wire_and_a_note_does_not_carry_one() {
        let handed =
            Message::sent("magi/main/beta-nu", "yours now", Sort::Handoff, None).carried(3);
        let text = serde_json::to_string(&handed).expect("encodes");
        assert_eq!(
            serde_json::from_str::<Message>(&text)
                .expect("decodes")
                .hops,
            3
        );
        let older = r#"{"id":"x","from":"magi/main/beta-nu","sort":"handoff","text":"y","at":1}"#;
        assert_eq!(
            serde_json::from_str::<Message>(older)
                .expect("decodes")
                .hops,
            0
        );
        let note = serde_json::to_value(Message::new("magi/main/beta-nu", "fyi")).expect("encodes");
        assert!(note.get("hops").is_none(), "{note}");
    }

    #[test]
    fn a_message_remembers_who_sent_it() {
        let message = Message::new("magi/main/alpha-rho", "stop what you are doing");
        assert_eq!(message.from, "magi/main/alpha-rho");
        assert!(message.at > 0, "and when it was sent");
    }
}

#[cfg(test)]
mod client_tests {
    use super::*;

    /// The verbs the shipped Lua stub attaches to a session, with comment lines dropped first: a
    /// split on quotes would otherwise read the prose as verbs.
    fn surface() -> Vec<String> {
        let source = crate::CLIENT;
        let from = source
            .find("local SURFACE = {")
            .expect("the stub names its surface");
        let body = &source[from..];
        let to = body.find("\n}").expect("and closes it");
        body[..to]
            .lines()
            .map(str::trim)
            .filter(|line| !line.starts_with("--") && !line.starts_with("local"))
            .flat_map(|line| line.split('"').skip(1).step_by(2))
            .map(ToOwned::to_owned)
            .collect()
    }

    #[test]
    fn the_stub_speaks_every_verb_this_answers() {
        let stub = surface();
        for (verb, _) in VERBS {
            assert!(
                stub.iter().any(|named| named == verb),
                "the client library does not offer `{verb}`: {stub:?}"
            );
        }
    }

    #[test]
    fn the_stub_claims_nothing_this_does_not_answer() {
        let known: Vec<&str> = VERBS.iter().map(|(verb, _)| *verb).collect();
        for verb in surface() {
            assert!(
                known.contains(&verb.as_str()),
                "the client library offers `{verb}`, which nothing answers: {known:?}"
            );
        }
    }

    #[test]
    fn the_stub_is_shipped_whole_and_returns_its_module() {
        assert!(crate::CLIENT.len() > 4_000, "that is not the whole file");
        assert!(crate::CLIENT.contains("local M = { _NAME = \"melchior\""));
        assert!(crate::CLIENT.trim_end().ends_with("return M"));
    }

    #[test]
    fn the_stub_puts_a_caller_on_every_call() {
        assert!(crate::CLIENT.contains("from = self.from"));
    }
}

/// One session asking another to become its parent: a request rather than a message, answered by
/// a person, and it only ever runs downhill — a session asks to become a child, it never claims
/// to be somebody else's parent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub id: String,
    /// Who is asking, as `project/role/id`.
    pub from: String,
    pub why: String,
    /// When, in milliseconds since the epoch.
    pub at: u64,
}

impl Request {
    #[must_use]
    pub fn made(from: &str, why: &str) -> Self {
        let at = now_ms();
        Self {
            id: format!("{}-{at:x}", from.replace('/', "-")),
            from: from.to_owned(),
            why: why.to_owned(),
            at,
        }
    }
}

/// What the command line answers, as against [`VERBS`], which is the socket.
pub const CLI_VERBS: &[(&str, &str)] = &[
    ("verbs", "what this program answers, on each of its doors"),
    (
        "client",
        "the Lua client library for its surface, as source",
    ),
    ("needs", "what a coordinator may tell it, as declarations"),
    ("configure", "take that configuration, as Lua on stdin"),
    ("models", "what this machine could talk to"),
    ("ask", "run a turn; an Ask on stdin"),
    ("serve", "bind this session's socket and answer for it"),
    ("fork", "start a child session, named and vouched for"),
    ("auth", "sign in to a provider that takes more than a key"),
    ("tool", "the agent surface, as a harness calls it"),
    ("brief", "what this session should know about its siblings"),
    (
        "acknowledge",
        "clear the installed packages, so their declarations may run",
    ),
];
