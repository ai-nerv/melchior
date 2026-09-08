//! Being adopted is asked for, never taken.
//!
//! Split from [`super`] under THE RULE, which caps a file at 800 lines.
//!
//! The whole point of the handshake: one session cannot make itself another's master, and cannot
//! make itself another's child either. It can only put the question, and somebody at a keyboard
//! on the other side answers it.

use super::*;
use crate::wire::Call;

fn call_from(why: &str) -> Call {
    Call {
        call: "adopt".to_owned(),
        args: vec![serde_json::Value::String(why.to_owned())],
        from: Some("demo/main/beta-nu".to_owned()),
        token: None,
    }
}

fn a_main() -> About {
    About {
        me: Identity {
            project: "demo".to_owned(),
            role: "main".to_owned(),
            id: "alpha-rho".to_owned(),
        },
        parent: None,
        token: None,
        busy: false,
        working_for: 0,
        inbox: Vec::new(),
        minted: std::collections::BTreeMap::new(),
    }
}

fn caller(parent: Option<&str>) -> Whom {
    Whom {
        project: "demo".to_owned(),
        id: "beta-nu".to_owned(),
        parent: parent.map(ToOwned::to_owned),
    }
}

#[test]
fn asking_puts_the_question_and_settles_nothing() {
    // The reply must not read as a yes. A caller told "accepted" by the process it asked
    // would be reading its own request back, and would carry on as though it had a parent.
    let (reply, then) = answer(
        &call_from("I want your grants"),
        &a_main(),
        Some(&caller(None)),
    );
    assert!(reply.ok, "{reply:?}");
    let Then::Ask(request) = then else {
        panic!("a request must be held for a person, not acted on: {then:?}");
    };
    assert_eq!(request.from, "demo/main/beta-nu");
    assert_eq!(request.why, "I want your grants");
    assert!(
        !request.id.is_empty(),
        "an answer has to be able to name it"
    );
}

#[test]
fn a_session_that_already_answers_to_somebody_is_not_taken_on() {
    // Two lines of authority over one session makes "who may direct this" unanswerable.
    //
    // Refused twice over, and either is enough: the wall gets there first — a session with a
    // parent is not a main, and another instance's main may not reach it — and the rule in
    // the `adopt` arm catches the cases the wall lets through. What matters is that nothing
    // reaches a person: a prompt is the only thing that can turn into a yes.
    let mut held = a_main();
    held.parent = Some("demo/main/gamma-xi".to_owned());
    let (reply, then) = answer(&call_from("be mine"), &held, Some(&caller(None)));
    assert!(!reply.ok, "{reply:?}");
    assert_eq!(then, Then::Nothing, "it must not reach a person at all");
}

#[test]
fn a_session_with_a_parent_may_not_go_looking_for_another() {
    // Behind its parent's back, which is the objection: the parent lent it authority on the
    // understanding that it answers to them.
    let (reply, then) = answer(
        &call_from("adopt me too"),
        &a_main(),
        Some(&caller(Some("demo/main/gamma-xi"))),
    );
    assert!(!reply.ok);
    assert_eq!(then, Then::Nothing);
}

#[test]
fn a_stranger_that_says_nothing_about_itself_is_not_asked_about() {
    // `None` is a caller that did not name itself. Everything here is about who they are.
    let (reply, then) = answer(&call_from("hello"), &a_main(), None);
    assert!(!reply.ok);
    assert_eq!(then, Then::Nothing);
}
