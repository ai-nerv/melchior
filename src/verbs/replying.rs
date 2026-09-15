//! `reply` finds who to answer from the message it quotes. Split from [`super`] under THE RULE,
//! which caps a file at 800 lines.

use super::*;
use crate::wire::{Message, Sort};

fn asked_by(from: &str) -> Standing {
    Standing {
        me: "magi/main/alpha-rho".to_owned(),
        parent: None,
        forked: Vec::new(),
        minted: std::collections::BTreeMap::new(),
        inbox: vec![Message::sent(from, "which parser?", Sort::Question, None)],
    }
}

#[test]
fn it_goes_to_whoever_asked() {
    let standing = asked_by("magi/main/beta-nu");
    let about = standing.inbox[0].id.clone();
    let who = answering(&json!({"verb": "reply", "about": about}), &standing);
    assert_eq!(who.as_deref().ok(), Some("magi/main/beta-nu"));
}

#[test]
fn an_id_that_names_nothing_is_refused_with_what_the_inbox_holds() {
    // "No such message" on its own leaves a model with nowhere to go.
    let standing = asked_by("magi/main/beta-nu");
    let Err(refused) = answering(&json!({"verb": "reply", "about": "made-up"}), &standing) else {
        panic!("an id that names nothing must not resolve to somebody");
    };
    assert!(refused.failed);
    assert!(refused.said.contains("beta-nu"), "{}", refused.said);
}

#[test]
fn an_empty_inbox_says_so_rather_than_listing_nothing() {
    let mut standing = asked_by("magi/main/beta-nu");
    standing.inbox.clear();
    let Err(refused) = answering(&json!({"verb": "reply", "about": "m1"}), &standing) else {
        panic!("refused");
    };
    assert!(refused.said.contains("send") || refused.said.contains("ask"));
}

#[test]
fn a_named_recipient_still_wins_when_the_id_is_not_ours() {
    // A session may be answering something it was told about out of band.
    let standing = asked_by("magi/main/beta-nu");
    let who = answering(
        &json!({"verb": "reply", "about": "elsewhere", "who": "gamma-xi"}),
        &standing,
    );
    assert_eq!(who.as_deref().ok(), Some("gamma-xi"));
}

#[test]
fn reply_without_an_about_is_still_refused_before_anything_is_looked_up() {
    let out = answer(
        &json!({"verb": "reply", "message": "yes"}),
        &asked_by("x/y/z"),
    );
    assert!(out.failed);
    assert!(out.said.contains("about"), "{}", out.said);
}
