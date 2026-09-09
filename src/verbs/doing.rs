//! Working out what a verb means, and then doing it.
//!
//! Split from [`super`] because the two halves fail differently. Deciding is pure — a name that
//! does not parse, a verb that needs a `message`, a wall this session is on the wrong side of —
//! and every one of those is worth a test that binds nothing. Doing it is a socket, and what
//! goes wrong there is that nobody is listening.
//!
//! The order matters. Everything decidable is decided *before* anything is dialled, so a model
//! that asked for something it may not have gets told what it may do instead of spending the
//! round trip finding out. The far end checks again — a caller is not to be trusted with its own
//! permissions — but by then the turn has already been paid for.

use super::Answer;
use super::{NAMES_A_ROLE, SPEAKS, Standing, TOOL, VERBS, tasking};
use crate::asking;
use crate::directory::{Address, sending};
use crate::policy;
use crate::policy::Reach;
use crate::wire::{Reply, Sort};
use serde_json::Value;

/// A call this tool decided to make.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wanted {
    /// Which verb the model asked for.
    pub verb: String,
    /// Which instance, resolved against this session.
    pub who: crate::identity::Identity,
    /// What kind of message it carries, for the verbs that carry one.
    pub sort: Sort,
    /// What to say, for the verbs that say something.
    pub message: Option<String>,
    /// The message being answered or released, for the verbs that quote one.
    pub about: Option<String>,
    /// What to call the role, for `role` and `assign`. The sentence is in `message`.
    pub role: Option<String>,
    /// How many times this piece of work has been handed on, for `handoff`.
    ///
    /// Worked out here and carried on the frame, because no session can see a cycle from inside
    /// it — see [`crate::directory::sending::HOPS`].
    pub hops: u32,
    /// The secret the far end was started with, for the one verb that has to prove itself.
    pub token: Option<String>,
}

/// What sort of message a verb sends.
///
/// The verb *is* the sort, for everything but `send` — which takes one, because "put this in
/// their inbox" is the general case and the others are it with a meaning attached.
fn sorted(verb: &str, arguments: &Value) -> Sort {
    match verb {
        "ask" => Sort::Question,
        "reply" => Sort::Answer,
        "attention" => Sort::Attention,
        "trouble" => Sort::Trouble,
        "handoff" => Sort::Handoff,
        // The same sort a note has, and that is the difference between it and `trouble`: one is
        // read when the far end next looks, the other reaches somebody mid-turn. A fan-out that
        // interrupted a whole run for a status update would be a fan-out nobody leaves on.
        "announce" => Sort::Note,
        _ => arguments
            .get("sort")
            .and_then(Value::as_str)
            .and_then(Sort::read)
            .unwrap_or(Sort::Note),
    }
}

/// Everything decidable about a verb aimed at one instance.
///
/// `Err` is the refusal to hand back; `Ok` is a call worth making.
pub fn decide(
    verb: &str,
    who: &str,
    arguments: &Value,
    standing: &Standing,
) -> Result<Wanted, Answer> {
    let Some(address) = Address::read(who) else {
        return Err(Answer::refused(format!(
            "`{who}` is not a name an instance can have. Names are `id`, `role/id` or \
             `project/role/id`."
        )));
    };
    if !VERBS.iter().any(|(name, _)| *name == verb) {
        return Err(Answer::refused(format!(
            "`{verb}` is not one of {TOOL}'s verbs. Call it with `verb: \"help\"`."
        )));
    }
    let reach = match verb {
        "stop" => Reach::Stop,
        _ if SPEAKS.contains(&verb) => Reach::Tell,
        _ => Reach::Ask,
    };
    let me = standing.whom();
    let whole = address.against(&standing.identity());
    let relation = standing.stands(&whole);
    // **The same relation `stop` needs, and none of the proof.** Saying what a subagent is for is
    // not ending its life: a role grants nothing, so the secret that makes `stop` refusable would
    // be guarding a word — and demanding it would say the word mattered more than it does.
    // Refused with its own sentence rather than through the ladder, whose Stop refusal talks
    // about ending sessions and would send a model looking for a token it does not need.
    if verb == "assign" && relation != policy::Relation::Child {
        return Err(Answer::refused(format!(
            "this session did not start `{}`, so what it is for is not this session's to say. \
             An instance sets its own with `role`.",
            whole.full()
        )));
    }
    if let Some(said) = arguments.get("message").and_then(Value::as_str)
        && NAMES_A_ROLE.contains(&verb)
        && said.chars().count() > crate::directory::roles::AT_MOST
    {
        return Err(Answer::refused(format!(
            "that description is {} characters and a role may say {}. It is what a coordinator \
             reads to pick somebody, not where the work is described.",
            said.chars().count(),
            crate::directory::roles::AT_MOST
        )));
    }
    if !policy::may(&me, relation, reach) {
        return Err(Answer::refused(policy::refusal(&me, relation, reach)));
    }
    // Bounded before the dial, like everything else here, and for the plainest of the reasons: a
    // message the far end will refuse for its length has still cost a round trip by the time it
    // says so, and the model reads the refusal a turn later than it could have.
    if let Some(said) = arguments.get("message").and_then(Value::as_str)
        && !NAMES_A_ROLE.contains(&verb)
        && let Err(why) = sending::sized(said)
    {
        return Err(Answer::refused(why));
    }
    // **The count comes off what arrived and goes onto what leaves.** A handoff cycle is invisible
    // from inside any one session — each of A, B and C sees one arrival and one departure — so
    // the only place the ring can be counted is on the work itself.
    let hops = if verb == "handoff" {
        let deep = sending::hopped(&standing.inbox);
        if let Some(why) = sending::too_far(deep) {
            return Err(Answer::refused(why));
        }
        deep
    } else {
        0
    };
    // By verb rather than by reach, so that a second verb rated `Stop` one day cannot pick a
    // secret up on the way past. One line sends one, and it can be read.
    let token = if verb == "stop" {
        // Held only for what this session started. Refused here rather than at the far end,
        // where the answer would be "that is not the secret" — true, and no help at all to a
        // model that never had one.
        let Some(secret) = standing.minted.get(&whole.id).cloned() else {
            return Err(Answer::refused(format!(
                "this session did not start `{}`, so it holds nothing that could stop it",
                whole.full()
            )));
        };
        Some(secret)
    } else {
        None
    };
    Ok(Wanted {
        verb: verb.to_owned(),
        who: whole,
        sort: sorted(verb, arguments),
        message: arguments
            .get("message")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        about: arguments
            .get("about")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        role: arguments
            .get("role")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        hops,
        token,
    })
}

/// Make the call, having first charged it against what this session may send.
///
/// **Counted here rather than in [`decide`]**, and it is the one gate that is not pure: the record
/// is a file, because `melchior tool` is one process per call and nothing it counted in memory
/// would survive the exit. It is still before the socket opens, which is the property that
/// matters — a refusal a model reads without paying for a round trip.
///
/// Only the verbs that put something in an inbox are charged. Asking a peer what it is doing is
/// not a message, and rating it as one would have a coordinator run out of window reading its own
/// crew.
pub fn perform(wanted: &Wanted, standing: &Standing) -> Answer {
    if SPEAKS.contains(&wanted.verb.as_str())
        && let Err(why) = sending::allow(
            &standing.identity(),
            &format!(
                "{}\u{0}{}\u{0}{}",
                wanted.verb,
                wanted.who.id,
                wanted.message.as_deref().unwrap_or_default()
            ),
        )
    {
        return Answer::refused(why);
    }
    carried(wanted, standing)
}

/// The same, already charged.
///
/// What a fan-out calls, because an announce to a crew of twelve is one thing a model decided to
/// say rather than twelve: charged per recipient it would trip its own cap on the first call.
pub(super) fn carried(wanted: &Wanted, standing: &Standing) -> Answer {
    let me = standing.identity();
    let mut held = match crate::directory::dial(&wanted.who, &me) {
        Ok(held) => held,
        // The common failure, and worth its own sentence: a socket file outlives the process
        // that made it, so a name found in the directory is not a promise that anything is
        // behind it. "nothing is listening" is actionable; "connection refused" is not.
        Err(why) => {
            return Answer::refused(format!(
                "nothing is listening as `{}` ({why}). Use `list` to see who is actually there.",
                wanted.who.full()
            ));
        }
    };
    let reply = match said(&mut held, wanted) {
        Ok(reply) => reply,
        Err(why) => {
            return Answer::refused(format!("`{}` did not answer: {why}", wanted.who.full()));
        }
    };
    if !reply.ok {
        // The far end's own words. It knows things this side does not — that it was never
        // started by anybody, that the secret was wrong — and repeating them beats a summary.
        return Answer::refused(format!(
            "`{}` refused: {}{}",
            wanted.who.full(),
            reply.error.unwrap_or_else(|| "no reason given".to_owned()),
            tasking::rejected(&wanted.verb)
        ));
    }
    Answer::said(landed(wanted, &reply))
}

/// One verb, over an open connection.
fn said(held: &mut asking::Held, wanted: &Wanted) -> std::io::Result<Reply> {
    match wanted.verb.as_str() {
        "about" => held.call("identity", Vec::new()),
        "status" => held.call("status", Vec::new()),
        "verbs" => held.call("verbs", Vec::new()),
        "stop" => held.call_with(
            "stop",
            Vec::new(),
            wanted.token.as_deref().unwrap_or_default(),
        ),
        // Both are the same call: an agent naming itself, and its parent naming it. What differs
        // is who may make it, and that was settled above and is settled again at the far end.
        "role" | "assign" => held.call(
            "role",
            vec![
                serde_json::json!(wanted.role.clone().unwrap_or_default()),
                serde_json::json!(wanted.message),
            ],
        ),
        // The reason travels with it: somebody is about to be asked to take responsibility for
        // a session, and "why" is the whole of what they have to go on.
        "adopt" => held.call(
            "adopt",
            vec![serde_json::json!(
                wanted.message.clone().unwrap_or_default()
            )],
        ),
        // Everything else is a message. The sort is what makes them different verbs rather than
        // a wording choice: `attention` and `note` travel identically and mean entirely
        // different things to whoever reads them.
        _ => held.call(
            "tell",
            vec![
                serde_json::json!(wanted.message.clone().unwrap_or_default()),
                serde_json::json!(name_of(wanted.sort)),
                serde_json::json!(wanted.about),
                serde_json::json!(wanted.hops),
            ],
        ),
    }
}

/// What to tell the model when it worked.
fn landed(wanted: &Wanted, reply: &Reply) -> String {
    let who = wanted.who.full();
    match wanted.verb.as_str() {
        // A report is whatever came back, verbatim. The model asked a question about another
        // session; summarising the answer here would be this file deciding what mattered.
        "about" | "status" | "verbs" => reply
            .result
            .first()
            .map_or_else(|| "nothing".to_owned(), ToString::to_string),
        "stop" => format!("`{who}` was told to stop."),
        // Said plainly, because the tempting reading is the wrong one. A role is what `crew`
        // shows a coordinator so it can pick somebody; it is not an instruction, and nothing
        // about what this session or that one may do has changed.
        "role" | "assign" => format!(
            "`{}` is now `{}`. That is what `crew` will show; it grants nothing and changes \
             nothing about what may be reached.",
            wanted.who.id,
            wanted.role.as_deref().unwrap_or_default()
        ),
        // A handle and a state, because "it is in their inbox" is a fact with no follow-up. The
        // handle is the id the far end minted for the message, which is also the id it will quote
        // in `about` when it answers — so `task` can say where this got to without anything
        // being written down anywhere.
        "ask" => reply
            .result
            .first()
            .and_then(|said| said.get("id"))
            .and_then(serde_json::Value::as_str)
            .map_or_else(
                || {
                    format!(
                        "The question is in `{who}`'s inbox. It answered without naming the \
                         message, so there is no handle to ask after — its answer will still \
                         arrive in this session's inbox."
                    )
                },
                |id| tasking::submitted(&wanted.who, id),
            ),
        "attention" | "trouble" => {
            format!("`{who}` has it, marked so it can interrupt whatever they are doing.")
        }
        // Said carefully, because the model must not carry on as though it now had a parent.
        // Nothing has changed yet and nothing may change: a person has to say yes first, and
        // the answer arrives later as a message.
        "adopt" => format!(
            "Asked `{who}` to take this session on. Nothing has changed yet — somebody there \
             has to accept, and their answer will arrive in this session's inbox."
        ),
        _ => format!("In `{who}`'s inbox."),
    }
}

/// The wire name of a sort.
fn name_of(sort: Sort) -> String {
    serde_json::to_value(sort)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| "note".to_owned())
}

/// Deciding is settled before anything is dialled.
#[cfg(test)]
mod tests {
    use super::*;

    fn standing() -> Standing {
        Standing {
            me: "magi/main/alpha-rho".to_owned(),
            parent: None,
            forked: Vec::new(),
            minted: std::collections::BTreeMap::new(),
            inbox: Vec::new(),
        }
    }

    #[test]
    fn a_verb_is_decided_without_anything_listening() {
        // The whole reason deciding and doing are separate functions. Every refusal worth
        // testing is decidable, and a test that has to bind a socket is one nobody writes.
        let wanted = decide(
            "send",
            "beta-nu",
            &serde_json::json!({"message": "hello"}),
            &standing(),
        )
        .expect("a call worth making");
        assert_eq!(wanted.who.full(), "magi/main/beta-nu");
        assert_eq!(wanted.sort, Sort::Note);
    }

    #[test]
    fn the_verb_is_the_sort_for_everything_but_send() {
        for (verb, sort) in [
            ("ask", Sort::Question),
            ("reply", Sort::Answer),
            ("attention", Sort::Attention),
            ("trouble", Sort::Trouble),
            ("handoff", Sort::Handoff),
            // A fan-out that interrupted a whole run for a status update is one nobody leaves on.
            ("announce", Sort::Note),
        ] {
            let wanted = decide(
                verb,
                "beta-nu",
                &serde_json::json!({"message": "x"}),
                &standing(),
            )
            .expect("decided");
            assert_eq!(wanted.sort, sort, "{verb}");
        }
    }

    #[test]
    fn a_handoff_carries_one_more_hop_than_the_deepest_that_arrived() {
        // The count has to ride the message because no session can see a ring: A hands to B, B to
        // C, C back to A, and each of the three sees one arrival and one departure.
        let mut standing = standing();
        let fresh = decide(
            "handoff",
            "beta-nu",
            &serde_json::json!({"message": "yours now"}),
            &standing,
        )
        .expect("decided");
        assert_eq!(fresh.hops, 1, "a first handoff started somewhere else");

        let mut handed =
            crate::wire::Message::sent("magi/main/gamma-xi", "yours now", Sort::Handoff, None);
        handed.hops = 4;
        standing.inbox.push(handed);
        let on = decide(
            "handoff",
            "beta-nu",
            &serde_json::json!({"message": "yours now"}),
            &standing,
        )
        .expect("decided");
        assert_eq!(on.hops, 5);
        // And nothing else carries one, so a note cannot age a chain it was never part of.
        let note = decide(
            "send",
            "beta-nu",
            &serde_json::json!({"message": "fyi"}),
            &standing,
        )
        .expect("decided");
        assert_eq!(note.hops, 0);
    }

    #[test]
    fn a_handoff_that_has_come_back_round_is_refused_before_the_dial() {
        // Risk 6. MAST measures step repetition at 15.7% and unaware-of-termination at 12.4%, and
        // a cycle looks like both from inside. What stops it is the number, and the number is
        // refused here rather than at whoever it would have been handed to next.
        let mut standing = standing();
        let mut arrived =
            crate::wire::Message::sent("magi/main/gamma-xi", "yours now", Sort::Handoff, None);
        arrived.hops = crate::directory::sending::HOPS;
        standing.inbox.push(arrived);
        let refused = decide(
            "handoff",
            "beta-nu",
            &serde_json::json!({"message": "yours now"}),
            &standing,
        )
        .expect_err("a ring closed");
        assert!(refused.failed);
        assert!(refused.said.contains("cycle"), "{}", refused.said);
        assert!(
            refused.said.contains("trouble"),
            "and no way out of it: {}",
            refused.said
        );
    }

    #[test]
    fn a_message_longer_than_the_cap_never_reaches_a_socket() {
        let refused = decide(
            "send",
            "beta-nu",
            &serde_json::json!({"message": "z".repeat(crate::directory::sending::AT_MOST + 1)}),
            &standing(),
        )
        .expect_err("an uncapped message");
        assert!(
            refused
                .said
                .contains(&crate::directory::sending::AT_MOST.to_string()),
            "{}",
            refused.said
        );
    }

    #[test]
    fn send_takes_the_sort_it_was_given_and_defaults_to_a_note() {
        let given = decide(
            "send",
            "beta-nu",
            &serde_json::json!({"message": "x", "sort": "attention"}),
            &standing(),
        )
        .expect("decided");
        assert_eq!(given.sort, Sort::Attention);
        let bare = decide(
            "send",
            "beta-nu",
            &serde_json::json!({"message": "x"}),
            &standing(),
        )
        .expect("decided");
        assert_eq!(bare.sort, Sort::Note);
    }

    #[test]
    fn stopping_something_with_no_secret_is_refused_before_the_round_trip() {
        // The far end would say "that is not the secret", which is true and no help at all to a
        // model that never had one.
        let mut standing = standing();
        standing.forked.push("iota-mu".to_owned());
        let refused = decide("stop", "iota-mu", &serde_json::json!({}), &standing)
            .expect_err("nothing to stop it with");
        assert!(refused.failed);
        assert!(refused.said.contains("did not start"), "{}", refused.said);
    }

    #[test]
    fn stopping_something_this_session_started_carries_the_secret() {
        let mut standing = standing();
        standing.forked.push("iota-mu".to_owned());
        standing
            .minted
            .insert("iota-mu".to_owned(), "s3cret".to_owned());
        let wanted = decide("stop", "iota-mu", &serde_json::json!({}), &standing).expect("decided");
        assert_eq!(wanted.token.as_deref(), Some("s3cret"));
    }

    #[test]
    fn no_other_verb_ever_carries_a_secret() {
        // One line that sends one, so it can be read. A verb picking one up by accident is how
        // a secret ends up somewhere it was never meant to go.
        let mut standing = standing();
        standing
            .minted
            .insert("beta-nu".to_owned(), "s3cret".to_owned());
        for verb in ["send", "ask", "status", "about", "attention"] {
            let wanted = decide(
                verb,
                "beta-nu",
                &serde_json::json!({"message": "x"}),
                &standing,
            )
            .expect("decided");
            assert!(wanted.token.is_none(), "{verb} carried the secret");
        }
    }

    #[test]
    fn a_wall_is_met_here_rather_than_over_the_socket() {
        let mut standing = standing();
        standing.parent = Some("beta-nu".to_owned());
        let refused = decide(
            "send",
            "other/main/tau-chi",
            &serde_json::json!({"message": "x"}),
            &standing,
        )
        .expect_err("across the wall");
        assert!(refused.said.contains("projects"), "{}", refused.said);
    }

    #[test]
    fn nothing_listening_says_so_and_says_what_to_do_about_it() {
        // A socket file outlives the process that made it, so a name in the directory is not a
        // promise that anything is behind it.
        let wanted = decide(
            "status",
            "nobody-nowhere",
            &serde_json::json!({}),
            &standing(),
        )
        .expect("decided");
        let out = perform(&wanted, &standing());
        assert!(out.failed);
        assert!(out.said.contains("nothing is listening"), "{}", out.said);
        assert!(out.said.contains("list"), "and what to do: {}", out.said);
    }
}
