//! The pipe is a wire between two repositories, so its spellings are pinned here.
//!
//! Split from [`super`] under THE RULE, which caps a file at 800 lines.

use super::{Heard, Peer};

#[test]
fn a_peer_goes_up_the_pipe_as_an_id_a_role_and_a_screen() {
    // magi is a separate program in a separate repository and parses this by field name.
    // Nothing fails when a name here drifts: the harness goes on reading the line, finds no
    // agents in it, and quietly has no peers — which reads as "nobody else is running".
    let line = serde_json::to_string(&Heard::Around {
        agents: vec![Peer {
            id: "beta-nu".to_owned(),
            role: "reviewer".to_owned(),
            ui: Some("/run/user/1000/magi/magi/1f4a.host".to_owned()),
        }],
    })
    .expect("a line");
    assert_eq!(
        line,
        r#"{"event":"around","agents":[{"id":"beta-nu","role":"reviewer","ui":"/run/user/1000/magi/magi/1f4a.host"}]}"#
    );
}

#[test]
fn an_agent_with_no_screen_says_so_rather_than_leaving_the_field_out() {
    // A missing field and a null both parse on the far side, and only one of them says the
    // harness was asked and had nothing to give.
    let line = serde_json::to_string(&Heard::Around {
        agents: vec![Peer {
            id: "beta-nu".to_owned(),
            role: "main".to_owned(),
            ui: None,
        }],
    })
    .expect("a line");
    assert!(line.contains(r#""ui":null"#), "{line}");
}
