//! The wider surface: what each verb needs, and what it means when it lands.
//!
//! Split from [`super`] under THE RULE, which caps a file at 800 lines.

use super::*;
use crate::scratch::Project;
use crate::wire::{Message, Sort};

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

/// A project of its own, because the verbs that need no instance now *do* things — `claim`
/// writes a file and `announce` reaches a run — and a test run against `magi` would reach
/// whatever the person at this machine has open.
///
/// One project each, and a guard around it. The two callers shared a single name and removed it
/// on their last line, so whichever finished first tore down the directory the other was still
/// writing in — and neither removed anything at all when it failed.
fn nowhere() -> (Project, Standing) {
    let project = Project::new("melchior-surface", "alone");
    let standing = Standing {
        me: format!("{project}/main/alpha-rho"),
        ..standing()
    };
    (project, standing)
}

#[test]
fn every_verb_either_needs_an_instance_or_is_listed_as_not_needing_one() {
    // The table and the dispatch drift the moment either is edited, and the drift shows up
    // as the model being told `whoami` wants a `who`.
    let (_project, standing) = nowhere();
    for (verb, _) in VERBS {
        let out = call(
            json!({"verb": verb, "message": "x", "about": "y", "role": "z"}),
            standing.clone(),
        );
        let wants_who = out.failed && out.said.contains("needs `who`");
        assert_eq!(
            wants_who,
            !ALONE.contains(verb),
            "{verb}: needs an instance = {wants_who}, listed as alone = {}",
            ALONE.contains(verb)
        );
    }
}

#[test]
fn nothing_that_needs_no_instance_is_still_a_stub() {
    // A verb added to `ALONE` and not to the dispatch joins the stubs silently — it is a
    // match arm, not a missing function, so nothing fails to compile and the model is told
    // the verb is "understood but not yet carried out" for as long as nobody tries it.
    let (_project, standing) = nowhere();
    for verb in ALONE {
        let out = call(
            json!({"verb": verb, "message": "x", "about": "y", "role": "z"}),
            standing.clone(),
        );
        assert!(
            !out.said.contains("not yet carried out"),
            "`{verb}` is listed and does nothing: {}",
            out.said
        );
    }
}

#[test]
fn a_verb_that_says_something_refuses_to_say_nothing() {
    for verb in SPEAKS {
        let out = call(json!({"verb": verb, "who": "gamma"}), standing());
        assert!(out.failed, "{verb} sent an empty message");
        assert!(out.said.contains("message"), "{verb}: {}", out.said);
    }
}

#[test]
fn a_verb_that_quotes_an_id_refuses_to_quote_nothing() {
    // `release` was in none of the three tables, so nothing asked it for anything: it went
    // out as a `release` with an empty body, telling the far end that something had been
    // let go and not what.
    for (verb, _) in QUOTES {
        let out = call(
            json!({"verb": verb, "who": "gamma", "message": "x"}),
            standing(),
        );
        assert!(out.failed, "{verb} quoted nothing");
        assert!(out.said.contains("about"), "{verb}: {}", out.said);
    }
}

#[test]
fn a_reply_has_to_say_what_it_is_answering() {
    // Without it the far end has an answer and no idea to what, which is worse than no
    // answer: it reads as an unprompted assertion.
    let out = call(json!({"verb": "reply", "message": "yes"}), standing());
    assert!(out.failed);
    assert!(out.said.contains("about"), "{}", out.said);
    assert!(out.said.contains("inbox"), "and where to find the id");
}

#[test]
fn a_root_session_is_told_it_has_nobody_to_escalate_to() {
    // A subagent that does not know it is one will not raise `attention` at a parent it
    // does not know it has. A root that thinks it has one will wait for an answer forever.
    let said = call(json!({"verb": "whoami"}), standing()).said;
    assert!(said.contains("root session"), "{said}");

    let mut child = standing();
    child.parent = Some("magi/main/root".to_owned());
    let said = call(json!({"verb": "whoami"}), child).said;
    assert!(said.contains("magi/main/root"), "{said}");
    assert!(said.contains("attention"), "and what to do with it: {said}");
}

#[test]
fn whoami_says_what_it_may_stop() {
    let mut standing = standing();
    standing.forked.push("magi/main/gamma".to_owned());
    let said = call(json!({"verb": "whoami"}), standing).said;
    assert!(said.contains("magi/main/gamma"), "{said}");
    assert!(said.contains("may stop"), "{said}");
}

#[test]
fn only_a_cry_for_help_interrupts() {
    // An inbox that interrupts for every note is an inbox nobody leaves switched on.
    for sort in [Sort::Attention, Sort::Trouble] {
        assert!(sort.interrupts(), "{sort:?} should reach a busy session");
    }
    for sort in [Sort::Note, Sort::Question, Sort::Answer, Sort::Claim] {
        assert!(!sort.interrupts(), "{sort:?} should wait");
    }
}

#[test]
fn the_inbox_marks_what_is_urgent_and_what_is_owed_an_answer() {
    let mut standing = standing();
    standing.inbox.push(Message::new("magi/main/beta", "fyi"));
    standing.inbox.push(Message::sent(
        "magi/main/gamma",
        "which parser?",
        Sort::Question,
        None,
    ));
    standing.inbox.push(Message::sent(
        "magi/main/delta",
        "I am stuck",
        Sort::Attention,
        None,
    ));
    let said = call(json!({"verb": "inbox"}), standing).said;
    assert!(
        said.contains("! `magi/main/delta`"),
        "urgent unmarked: {said}"
    );
    assert!(
        said.contains("`reply`"),
        "no way back to the question: {said}"
    );
    assert!(said.contains("[question]"), "sorts are not shown: {said}");
}

#[test]
fn a_message_can_be_answered_by_the_id_the_inbox_showed() {
    // The id has to survive the round trip, or `reply` quotes something nobody has.
    let message = Message::sent("magi/main/gamma", "which parser?", Sort::Question, None);
    let text = serde_json::to_string(&message).expect("encodes");
    let back: Message = serde_json::from_str(&text).expect("decodes");
    assert_eq!(back.id, message.id);
    assert_eq!(back.sort, Sort::Question);
}
