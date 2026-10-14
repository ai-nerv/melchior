//! What a parent lends is taken only from the session this one asked, quoting the nonce it left.
//!
//! Split from [`super`] under THE RULE, which caps a file at 800 lines.

use super::*;
use crate::scratch::Project;
use crate::wire::Call;

fn call_from(project: &Project, who: &str, nonce: &str) -> Call {
    Call {
        call: "adopted".to_owned(),
        args: vec![
            serde_json::Value::String(format!("{project}/main/{who}")),
            serde_json::Value::String("[{\"verb\":\"run\"}]".to_owned()),
            serde_json::Value::Null,
            serde_json::Value::String(nonce.to_owned()),
        ],
        from: Some(format!("{project}/main/{who}")),
        token: None,
    }
}

fn me(project: &Project) -> About {
    About {
        me: Identity {
            project: project.to_string(),
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
    }
}

#[test]
fn the_session_asked_may_hand_over_once() {
    let project = Project::new("melchior-handover", "asked");
    let about = me(&project);
    let nonce = crate::directory::pending::asked(&about.me, "beta-nu").expect("an asking");
    let (reply, then) = answer(&call_from(&project, "beta-nu", &nonce), &about, None);
    assert!(reply.ok, "{reply:?}");
    let Then::Adopted { handover, .. } = then else {
        panic!("it must reach the harness: {then:?}");
    };
    assert_eq!(handover.as_deref(), Some("[{\"verb\":\"run\"}]"));

    let (again, then) = answer(&call_from(&project, "beta-nu", &nonce), &about, None);
    assert!(!again.ok, "an answer was taken twice: {again:?}");
    assert_eq!(then, Then::Nothing);
}

#[test]
fn a_session_that_was_not_asked_may_not_even_with_the_nonce() {
    // The one that matters. Anybody who could send this could lend a session permissions nobody
    // consented to — so it is believed only quoting what this session's own asking left.
    let project = Project::new("melchior-handover", "other");
    let about = me(&project);
    let nonce = crate::directory::pending::asked(&about.me, "beta-nu").expect("an asking");
    let (reply, then) = answer(&call_from(&project, "gamma-xi", &nonce), &about, None);
    assert!(!reply.ok, "a stranger handed over permissions");
    assert_eq!(then, Then::Nothing);
}

#[test]
fn a_session_that_asked_nobody_takes_nothing_from_anybody() {
    let project = Project::new("melchior-handover", "nobody");
    let (reply, then) = answer(
        &call_from(&project, "beta-nu", "made-up"),
        &me(&project),
        None,
    );
    assert!(!reply.ok);
    assert_eq!(then, Then::Nothing);
}

#[test]
fn two_sessions_cannot_take_each_other_on() {
    // `alpha-rho` asked `beta-nu`, and `beta-nu` already answers to `alpha-rho`: a ring.
    let project = Project::new("melchior-handover", "ring");
    let about = me(&project);
    let nonce = crate::directory::pending::asked(&about.me, "beta-nu").expect("an asking");
    let them = Identity {
        project: project.to_string(),
        role: "main".to_owned(),
        id: "beta-nu".to_owned(),
    };
    crate::directory::adopted(&them, "alpha-rho").expect("the note");
    let (reply, then) = answer(&call_from(&project, "beta-nu", &nonce), &about, None);
    assert!(!reply.ok, "a ring was closed: {reply:?}");
    assert_eq!(then, Then::Nothing);
}
