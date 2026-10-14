//! The vocabulary, answered to the caller each verb is for and refused to the rest.

use super::*;

fn about() -> About {
    About {
        me: Identity {
            project: "magi".to_owned(),
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

fn whom(project: &str, id: &str, parent: Option<&str>) -> Whom {
    Whom {
        project: project.to_owned(),
        id: id.to_owned(),
        parent: parent.map(ToOwned::to_owned),
        session: None,
        root: None,
    }
}

fn call(verb: &str) -> Call {
    Call {
        call: verb.to_owned(),
        ..Call::default()
    }
}

#[test]
fn verbs_is_answered_before_anybody_has_said_who_they_are() {
    let (reply, then) = answer(&call("verbs"), &about(), None);
    assert!(reply.ok, "{reply:?}");
    assert_eq!(reply.n, reply.result.len());
    assert_eq!(then, Then::Nothing);
}

#[test]
fn everything_else_needs_a_caller() {
    for verb in ["identity", "status", "inbox", "tell", "stop"] {
        let (reply, then) = answer(&call(verb), &about(), None);
        assert!(!reply.ok, "{verb} answered a stranger");
        assert_eq!(then, Then::Nothing);
    }
}

#[test]
fn a_verb_nobody_answers_is_named_to_any_caller() {
    let them = whom("magi", "beta-nu", None);
    for caller in [None, Some(&them)] {
        let (reply, then) = answer(&call("no-such-verb"), &about(), caller);
        assert!(!reply.ok);
        assert_eq!(reply.fault, Some(crate::wire::Fault::Refused));
        let why = reply.error.unwrap_or_default();
        assert!(why.contains("no such call: no-such-verb"), "{why}");
        assert_eq!(then, Then::Nothing);
    }
}

#[test]
fn a_main_answers_another_main_in_the_same_project() {
    let them = whom("magi", "beta-nu", None);
    let (reply, _) = answer(&call("status"), &about(), Some(&them));
    assert!(reply.ok, "{reply:?}");
}

#[test]
fn nothing_from_another_project_is_answered() {
    let them = whom("other", "beta-nu", None);
    for verb in ["identity", "status", "tell", "stop"] {
        let (reply, _) = answer(&call(verb), &about(), Some(&them));
        assert!(!reply.ok, "{verb} crossed the wall");
        let why = reply.error.unwrap_or_default();
        assert!(why.contains("projects"), "{why}");
    }
    let (inbox, _) = answer(&call("inbox"), &about(), Some(&them));
    assert!(!inbox.ok, "an inbox crossed the wall");
}

#[test]
fn an_inbox_is_read_by_its_own_session_and_nobody_else() {
    let mut held = about();
    held.parent = Some("beta-nu".to_owned());
    for them in [
        whom("magi", "beta-nu", None),
        whom("magi", "zeta-pi", Some("beta-nu")),
        whom("magi", "iota-mu", Some("alpha-rho")),
        whom("magi", "gamma-xi", None),
    ] {
        let (reply, _) = answer(&call("inbox"), &held, Some(&them));
        assert!(!reply.ok, "{} read this session's inbox", them.id);
    }
    let (own, _) = answer(
        &call("inbox"),
        &held,
        Some(&whom("magi", "alpha-rho", None)),
    );
    assert!(own.ok, "{own:?}");
}

#[test]
fn a_cousin_is_refused_at_the_default_and_told_what_would_help() {
    let mut about = about();
    about.parent = Some("beta-nu".to_owned());
    let them = whom("magi", "tau-chi", Some("gamma-xi"));
    let (reply, _) = answer(&call("status"), &about, Some(&them));
    assert!(!reply.ok);
    let why = reply.error.unwrap_or_default();
    assert!(why.contains("agent_talk"), "{why}");
}

#[test]
fn a_message_is_stamped_with_who_actually_sent_it() {
    let them = whom("magi", "beta-nu", None);
    let call = Call {
        call: "tell".to_owned(),
        args: vec![serde_json::json!("the parser is done")],
        from: Some("magi/main/somebody-else".to_owned()),
        token: None,
    };
    let (reply, then) = answer(&call, &about(), Some(&them));
    assert!(reply.ok, "{reply:?}");
    let Then::Keep(message) = then else {
        panic!("it was not kept: {then:?}");
    };
    assert_eq!(message.from, "magi/main/beta-nu");
}

#[test]
fn a_message_carries_the_sort_it_was_sent_as() {
    let them = whom("magi", "beta-nu", None);
    let call = Call {
        call: "tell".to_owned(),
        args: vec![
            serde_json::json!("I am stuck"),
            serde_json::json!("trouble"),
        ],
        ..Call::default()
    };
    let (_, then) = answer(&call, &about(), Some(&them));
    let Then::Keep(message) = then else {
        panic!("it was not kept");
    };
    assert!(message.sort.interrupts(), "{:?}", message.sort);
}

#[test]
fn a_main_nobody_started_cannot_be_stopped_by_anything() {
    let them = whom("magi", "beta-nu", None);
    let (reply, then) = answer(&call("stop"), &about(), Some(&them));
    assert!(!reply.ok);
    assert_eq!(then, Then::Nothing);
}

#[test]
fn a_parent_that_knows_the_secret_may_stop_its_child() {
    let mut about = about();
    about.parent = Some("beta-nu".to_owned());
    about.token = Some("s3cret".to_owned());
    let parent = whom("magi", "beta-nu", None);
    let call = Call {
        call: "stop".to_owned(),
        token: Some("s3cret".to_owned()),
        ..Call::default()
    };
    let (reply, then) = answer(&call, &about, Some(&parent));
    assert!(reply.ok, "{reply:?}");
    assert_eq!(then, Then::Stop);
}

#[test]
fn the_name_alone_is_not_enough_to_stop_anything() {
    // Any process of this user can connect claiming to be the parent.
    let mut about = about();
    about.parent = Some("beta-nu".to_owned());
    about.token = Some("s3cret".to_owned());
    let pretending = whom("magi", "beta-nu", None);
    for token in [None, Some("guessed".to_owned())] {
        let call = Call {
            call: "stop".to_owned(),
            token,
            ..Call::default()
        };
        let (reply, then) = answer(&call, &about, Some(&pretending));
        assert!(!reply.ok, "a stop went through without the secret");
        assert_eq!(then, Then::Nothing);
    }
}

#[test]
fn a_sibling_holding_the_secret_still_may_not_stop_it() {
    // The secret is proof of identity, not a permission.
    let mut about = about();
    about.parent = Some("beta-nu".to_owned());
    about.token = Some("s3cret".to_owned());
    let sibling = whom("magi", "zeta-pi", Some("beta-nu"));
    let call = Call {
        call: "stop".to_owned(),
        token: Some("s3cret".to_owned()),
        ..Call::default()
    };
    let (reply, then) = answer(&call, &about, Some(&sibling));
    assert!(!reply.ok, "{reply:?}");
    assert_eq!(then, Then::Nothing);
}

#[test]
fn an_adopted_root_is_stopped_with_the_secret_its_new_parent_minted() {
    // A root has no token of its own; adoption hands it one, held in `adopted_token`, and the
    // parent that took it on may stop it by quoting that.
    let mut about = about();
    about.parent = Some("beta-nu".to_owned());
    about.adopted_token = Some("granted".to_owned());
    let parent = whom("magi", "beta-nu", None);

    let bare = Call {
        call: "stop".to_owned(),
        ..Call::default()
    };
    let (reply, then) = answer(&bare, &about, Some(&parent));
    assert!(!reply.ok, "stopped with no secret");
    assert_eq!(then, Then::Nothing);

    let with = Call {
        call: "stop".to_owned(),
        token: Some("granted".to_owned()),
        ..Call::default()
    };
    let (reply, then) = answer(&with, &about, Some(&parent));
    assert!(reply.ok, "{reply:?}");
    assert_eq!(then, Then::Stop);
}

#[test]
fn adoption_hands_the_new_parent_a_secret_to_stop_by() {
    // The `adopted` call carries the stop secret in its third argument and the nonce in its
    // fourth; the secret comes up as part of `Then::Adopted` for the loop to hold.
    let project = crate::scratch::Project::new("melchior-answer", "handed");
    let mut about = about();
    about.me.project = project.to_string();
    let nonce = crate::directory::pending::asked(&about.me, "beta-nu").expect("an asking");
    let call = |nonce: &str| Call {
        call: "adopted".to_owned(),
        args: vec![
            serde_json::json!(format!("{project}/main/beta-nu")),
            serde_json::Value::Null,
            serde_json::json!("the-secret"),
            serde_json::json!(nonce),
        ],
        from: Some(format!("{project}/main/beta-nu")),
        token: None,
    };
    let (guessed, then) = answer(&call("guessed"), &about, None);
    assert!(!guessed.ok, "a guessed nonce was taken: {guessed:?}");
    assert_eq!(then, Then::Nothing);

    let (reply, then) = answer(&call(&nonce), &about, None);
    assert!(reply.ok, "{reply:?}");
    let Then::Adopted { secret, .. } = then else {
        panic!("not an adoption: {then:?}");
    };
    assert_eq!(secret.as_deref(), Some("the-secret"));
}

#[test]
fn kin_says_how_the_caller_stands_and_whether_it_may_stop() {
    let mut about = about();
    about.parent = Some("beta-nu".to_owned());
    let parent = whom("magi", "beta-nu", None);
    let (reply, _) = answer(&call("kin"), &about, Some(&parent));
    assert!(reply.ok, "{reply:?}");
    assert_eq!(reply.result[0]["relation"], "parent");
    assert_eq!(reply.result[0]["may_stop"], true);
}

#[test]
fn a_verb_nothing_answers_says_so_rather_than_dropping_the_connection() {
    let them = whom("magi", "beta-nu", None);
    let (reply, _) = answer(&call("nope"), &about(), Some(&them));
    assert!(!reply.ok);
    assert!(
        reply.error.unwrap_or_default().contains("nope"),
        "it did not say which"
    );
}

/// The session a listed verb is put to, the caller entitled to put it, and what it says. Several
/// are answered only for a particular caller, so one parent-shaped caller would not do.
fn entitled(verb: &str, project: &str) -> (About, Whom, Vec<serde_json::Value>) {
    let mut about = about();
    about.me.project = project.to_owned();
    let said = vec![serde_json::json!("something")];
    match verb {
        // Asked by the session itself, over the socket it bound. `tool`'s one argument is the
        // coordination map; `help` is the verb that always answers.
        "tool" => (
            about,
            whom(project, "alpha-rho", None),
            vec![serde_json::json!({ "verb": "help" })],
        ),
        "mint" | "minted" | "inbox" => (about, whom(project, "alpha-rho", None), said),
        // A main asking a main, so this one is deliberately left without a parent.
        "adopt" => (
            about,
            whom(project, "beta-nu", None),
            vec![serde_json::json!("something"), serde_json::json!("nonce")],
        ),
        // The answer to an asking this session left, quoting its nonce.
        "adopted" => {
            let nonce = crate::directory::pending::asked(&about.me, "beta-nu").expect("an asking");
            let by = format!("{project}/main/beta-nu");
            let quoted = vec![
                serde_json::json!(by),
                serde_json::Value::Null,
                serde_json::Value::Null,
                serde_json::json!(nonce),
            ];
            (about, whom(project, "beta-nu", None), quoted)
        }
        _ => {
            about.parent = Some("beta-nu".to_owned());
            about.token = Some("s3cret".to_owned());
            (about, whom(project, "beta-nu", None), said)
        }
    }
}

#[test]
fn every_verb_the_family_is_told_about_is_one_that_answers() {
    for (name, _) in VERBS {
        let project = crate::scratch::Project::new("melchior-answer", name);
        let (about, caller, args) = entitled(name, &project);
        let call = Call {
            call: (*name).to_owned(),
            args,
            token: Some("s3cret".to_owned()),
            from: None,
        };
        let (reply, _) = answer(&call, &about, Some(&caller));
        assert!(reply.ok, "{name} is listed and refuses: {reply:?}");
    }
}

#[test]
fn a_verb_answered_only_for_one_caller_is_still_advertised() {
    // Refusing a caller is not the same as not having the verb, and only the list says which.
    let listed: Vec<&str> = VERBS.iter().map(|(name, _)| *name).collect();
    for verb in ["mint", "minted", "adopt", "adopted"] {
        assert!(listed.contains(&verb), "`{verb}` is answered and unlisted");
    }
}

#[test]
fn a_message_longer_than_a_sender_may_send_is_refused_where_it_lands() {
    let them = whom("magi", "beta-nu", None);
    let long = Call {
        call: "tell".to_owned(),
        args: vec![serde_json::json!(
            "z".repeat(crate::directory::sending::AT_MOST + 1)
        )],
        ..Call::default()
    };
    let (reply, then) = answer(&long, &about(), Some(&them));
    assert!(!reply.ok, "a 1 MiB tell landed");
    assert_eq!(then, Then::Nothing);
}
