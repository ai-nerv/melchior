//! What a parent lends is taken only from a parent.
//!
//! Split from [`super`] under THE RULE, which caps a file at 800 lines.

use super::*;
use crate::wire::Call;

fn call_from(who: &str) -> Call {
    Call {
        call: "adopted".to_owned(),
        args: vec![
            serde_json::Value::String(format!("demo/main/{who}")),
            serde_json::Value::String("[{\"verb\":\"run\"}]".to_owned()),
        ],
        from: Some(format!("demo/main/{who}")),
        token: None,
    }
}

fn me(parent: Option<&str>) -> About {
    About {
        me: Identity {
            project: "demo".to_owned(),
            role: "main".to_owned(),
            id: "alpha-rho".to_owned(),
        },
        parent: parent.map(ToOwned::to_owned),
        token: None,
        busy: false,
        working_for: 0,
        inbox: Vec::new(),
        minted: std::collections::BTreeMap::new(),
    }
}

fn caller(id: &str) -> Whom {
    Whom {
        project: "demo".to_owned(),
        id: id.to_owned(),
        parent: None,
        session: None,
    }
}

#[test]
fn a_parent_may_hand_over() {
    let (reply, then) = answer(
        &call_from("beta-nu"),
        &me(Some("beta-nu")),
        Some(&caller("beta-nu")),
    );
    assert!(reply.ok, "{reply:?}");
    let Then::Adopted { handover, .. } = then else {
        panic!("it must reach the harness: {then:?}");
    };
    assert_eq!(handover.as_deref(), Some("[{\"verb\":\"run\"}]"));
}

#[test]
fn a_session_that_is_not_the_parent_may_not() {
    // The one that matters. Anybody who could send this could lend a session permissions
    // nobody consented to — so it is believed only from the session the *directory* says
    // took this one on, and that note was written by whoever accepted.
    let (reply, then) = answer(
        &call_from("gamma-xi"),
        &me(Some("beta-nu")),
        Some(&caller("gamma-xi")),
    );
    assert!(!reply.ok, "a stranger handed over permissions");
    assert_eq!(then, Then::Nothing);
}

#[test]
fn a_session_with_no_parent_takes_nothing_from_anybody() {
    // Nothing accepted it, so there is nobody whose authority this could be.
    let (reply, then) = answer(&call_from("beta-nu"), &me(None), Some(&caller("beta-nu")));
    assert!(!reply.ok);
    assert_eq!(then, Then::Nothing);
}
