//! Being adopted is asked for, never taken: a session can only put the question, and somebody at
//! a keyboard on the other side answers it.

use super::*;
use crate::scratch::Project;
use crate::wire::Call;

fn call_from(project: &Project, why: &str) -> Call {
    Call {
        call: "adopt".to_owned(),
        args: vec![
            serde_json::Value::String(why.to_owned()),
            serde_json::Value::String("the-nonce".to_owned()),
        ],
        from: Some(format!("{project}/main/beta-nu")),
        token: None,
    }
}

fn a_main(project: &Project) -> About {
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

fn caller(project: &Project, parent: Option<&str>) -> Whom {
    if let Some(parent) = parent {
        let them = Identity {
            project: project.to_string(),
            role: "main".to_owned(),
            id: "beta-nu".to_owned(),
        };
        crate::directory::wrote(&crate::directory::kin_at(&them), parent).expect("the note");
    }
    crate::directory::whom(project, "beta-nu")
}

#[test]
fn asking_puts_the_question_and_settles_nothing() {
    // The reply says the question was put, not that it was answered.
    let project = Project::new("melchior-adopting", "asks");
    let (reply, then) = answer(
        &call_from(&project, "I want your grants"),
        &a_main(&project),
        Some(&caller(&project, None)),
    );
    assert!(reply.ok, "{reply:?}");
    let Then::Ask(request) = then else {
        panic!("a request must be held for a person, not acted on: {then:?}");
    };
    assert_eq!(request.from, format!("{project}/main/beta-nu"));
    assert_eq!(request.why, "I want your grants");
    assert_eq!(
        request.nonce, "the-nonce",
        "the answer has to quote it back"
    );
    assert!(
        !request.id.is_empty(),
        "an answer has to be able to name it"
    );
}

#[test]
fn an_asking_with_no_nonce_is_not_put_to_anybody() {
    let project = Project::new("melchior-adopting", "bare");
    let mut bare = call_from(&project, "take me");
    bare.args.truncate(1);
    let (reply, then) = answer(&bare, &a_main(&project), Some(&caller(&project, None)));
    assert!(!reply.ok, "{reply:?}");
    assert_eq!(then, Then::Nothing);
}

#[test]
fn a_session_that_already_answers_to_somebody_is_not_taken_on() {
    let project = Project::new("melchior-adopting", "held");
    let held = a_main(&project);
    crate::directory::wrote(&crate::directory::kin_at(&held.me), "gamma-xi").expect("the note");
    let (reply, then) = answer(
        &call_from(&project, "be mine"),
        &held,
        Some(&caller(&project, None)),
    );
    assert!(!reply.ok, "{reply:?}");
    assert_eq!(then, Then::Nothing, "it must not reach a person at all");
}

#[test]
fn a_session_with_a_parent_may_not_go_looking_for_another() {
    let project = Project::new("melchior-adopting", "looking");
    let (reply, then) = answer(
        &call_from(&project, "adopt me too"),
        &a_main(&project),
        Some(&caller(&project, Some("gamma-xi"))),
    );
    assert!(!reply.ok);
    assert_eq!(then, Then::Nothing);
}

#[test]
fn a_stranger_that_says_nothing_about_itself_is_not_asked_about() {
    // `None` is a caller nobody could name.
    let project = Project::new("melchior-adopting", "stranger");
    let (reply, then) = answer(&call_from(&project, "hello"), &a_main(&project), None);
    assert!(!reply.ok);
    assert_eq!(then, Then::Nothing);
}
