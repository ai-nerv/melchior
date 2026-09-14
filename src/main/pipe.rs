//! The pipe is a wire between two repositories, so its spellings are pinned here.
//!
//! Split from [`super`] under THE RULE, which caps a file at 800 lines.

use super::{Heard, Peer};

#[test]
fn a_peer_goes_up_the_pipe_as_an_id_a_role_and_a_screen() {
    // magi is a separate program in a separate repository and parses this by field name.
    let line = serde_json::to_string(&Heard::Around {
        agents: vec![Peer {
            id: "beta-nu".to_owned(),
            role: "reviewer".to_owned(),
            ui: Some("/run/user/1000/magi/magi/1f4a.host".to_owned()),
            parent: Some("alpha-rho".to_owned()),
            session: Some("alpha-rho".to_owned()),
            phase: None,
            cause: None,
            busy: true,
            working_for: 12,
            waiting: 0,
            claim: Some("the-parser".to_owned()),
            spent: Vec::new(),
        }],
    })
    .expect("a line");
    assert_eq!(
        line,
        r#"{"event":"around","agents":[{"id":"beta-nu","role":"reviewer","ui":"/run/user/1000/magi/magi/1f4a.host","parent":"alpha-rho","session":"alpha-rho","busy":true,"working_for":12,"waiting":0,"claim":"the-parser"}]}"#
    );
}

#[test]
fn an_agent_with_no_screen_says_so_rather_than_leaving_the_field_out() {
    // A missing field and a null both parse on the far side; only null says the harness was asked.
    let line = serde_json::to_string(&Heard::Around {
        agents: vec![Peer {
            id: "beta-nu".to_owned(),
            role: "main".to_owned(),
            ui: None,
            parent: None,
            session: None,
            phase: None,
            cause: None,
            busy: false,
            working_for: 0,
            waiting: 0,
            claim: None,
            spent: Vec::new(),
        }],
    })
    .expect("a line");
    assert!(line.contains(r#""ui":null"#), "{line}");
}

use super::signal_changes;

/// A peer with just the fields the signalling looks at.
fn peer(id: &str, parent: Option<&str>, phase: Option<&str>) -> Peer {
    Peer {
        id: id.to_owned(),
        role: "worker".to_owned(),
        ui: None,
        parent: parent.map(ToOwned::to_owned),
        session: None,
        phase: phase.map(ToOwned::to_owned),
        cause: None,
        busy: false,
        working_for: 0,
        waiting: 0,
        claim: None,
        spent: Vec::new(),
    }
}

#[test]
fn what_a_peer_spent_goes_up_the_pipe_and_nothing_when_it_spent_nothing() {
    let row = serde_json::json!({"model": "a/b", "input": 12, "cost_micros": 150});
    let line = serde_json::to_string(&Heard::Around {
        agents: vec![Peer {
            spent: vec![row],
            ..peer("kid", None, None)
        }],
    })
    .expect("a line");
    let sent: serde_json::Value = serde_json::from_str(&line).expect("json");
    assert_eq!(sent["agents"][0]["spent"][0]["model"], "a/b", "{line}");
    assert_eq!(sent["agents"][0]["spent"][0]["cost_micros"], 150, "{line}");
    let bare = serde_json::to_string(&Heard::Around {
        agents: vec![peer("kid", None, None)],
    })
    .expect("a line");
    assert!(!bare.contains("spent"), "{bare}");
}

fn kinds(signals: &[Heard]) -> Vec<(String, String)> {
    signals
        .iter()
        .filter_map(|h| match h {
            Heard::Signal { from, kind, .. } => Some((from.clone(), kind.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn a_child_changing_phase_signals_its_parent() {
    let was = vec![
        peer("lead", None, Some("working")),
        peer("kid", Some("lead"), Some("working")),
    ];
    let now = vec![
        peer("lead", None, Some("working")),
        peer("kid", Some("lead"), Some("finished")),
    ];
    // As the lead: the child finishing is a signal; the lead's own unchanged phase is not.
    assert_eq!(
        kinds(&signal_changes(
            "lead",
            None,
            &Default::default(),
            &was,
            &now
        )),
        vec![("kid".to_owned(), "finished".to_owned())]
    );
}

#[test]
fn a_stranger_and_an_unchanged_phase_signal_nothing() {
    let was = vec![
        peer("kid", Some("lead"), Some("working")),
        peer("other", Some("someone"), Some("working")),
    ];
    let now = vec![
        peer("kid", Some("lead"), Some("working")),
        peer("other", Some("someone"), Some("finished")),
    ];
    // As the lead: `other` is not ours, and `kid` did not change — neither signals.
    assert!(
        kinds(&signal_changes(
            "lead",
            None,
            &Default::default(),
            &was,
            &now
        ))
        .is_empty()
    );
}

#[test]
fn a_parent_finishing_signals_its_child() {
    let was = vec![peer("lead", None, Some("working"))];
    let now = vec![peer("lead", None, Some("finished"))];
    // As the kid, whose parent is the lead: the parent's change reaches it.
    assert_eq!(
        kinds(&signal_changes(
            "kid",
            Some("lead"),
            &Default::default(),
            &was,
            &now
        )),
        vec![("lead".to_owned(), "finished".to_owned())]
    );
}

#[test]
fn an_explicitly_watched_stranger_signals() {
    let was = vec![peer("far", Some("someone-else"), Some("working"))];
    let now = vec![peer("far", Some("someone-else"), Some("finished"))];
    let mut watched = std::collections::BTreeSet::new();
    watched.insert("far".to_owned());
    // `far` is neither child nor parent, but this session asked to watch it.
    assert_eq!(
        kinds(&signal_changes("lead", None, &watched, &was, &now)),
        vec![("far".to_owned(), "finished".to_owned())]
    );
}
