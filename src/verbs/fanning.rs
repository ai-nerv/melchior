//! Saying one thing to the whole run. Split from [`super`] under THE RULE, which caps a file at
//! 800 lines.
//!
//! There is no route of its own: each recipient goes through [`super::doing::decide`], the same
//! function `send` goes through, so a fan-out is not a way round a refusal, and refusals are
//! collected and reported rather than swallowed. It is charged against
//! [`crate::directory::sending::IN_A_WINDOW`] once rather than per recipient, which charged per
//! recipient would trip its own cap on the first call.

use super::{Answer, Standing, doing};
use crate::directory::{sending, sessions};
use crate::policy;

/// Send the same thing to everyone in this session's run.
///
/// `verb` is `announce` or `trouble`, and the difference is entirely what the far end does with
/// it: [`crate::wire::Sort::Trouble`] interrupts a turn and a note waits.
pub fn fanned(verb: &str, arguments: &serde_json::Value, standing: &Standing) -> Answer {
    let said = arguments
        .get("message")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    // Once, and before the roster is read: a message too long to send to one peer is not made
    // acceptable by being sent to twelve.
    if let Err(why) = sending::sized(said) {
        return Answer::refused(why);
    }
    let me = standing.whom();
    let held: Vec<_> = sessions::crew(&me)
        .into_iter()
        .filter(|them| them.id != me.id)
        .collect();
    if held.is_empty() {
        return Answer::said(format!(
            "There is nobody else in run `{}` to tell. `list` says who else is in `{}`, and \
             `send` reaches one of them by name.",
            me.session_root(),
            me.project
        ));
    }
    if held.len() > sending::AT_ONCE {
        return Answer::refused(format!(
            "run `{}` has {} other agents in it and `{verb}` reaches {}. Twenty-four turns spent \
             reading one sentence is not a way to coordinate — `crew` lists them, and `send` \
             reaches the ones that need this.",
            me.session_root(),
            held.len(),
            sending::AT_ONCE
        ));
    }
    if let Err(why) = sending::allow(&standing.identity(), &format!("{verb}\u{0}\u{0}{said}")) {
        return Answer::refused(why);
    }

    let mut landed: Vec<String> = Vec::new();
    let mut refused: Vec<String> = Vec::new();
    for them in &held {
        match doing::decide(verb, &them.id, arguments, standing) {
            Err(why) => refused.push(format!("`{}` ({})", them.id, first_line(&why.said))),
            Ok(wanted) => {
                let out = doing::carried(&wanted, standing);
                if out.failed {
                    refused.push(format!("`{}` ({})", them.id, first_line(&out.said)));
                } else {
                    landed.push(format!("`{}`", them.id));
                }
            }
        }
    }
    told(verb, &me, &landed, &refused)
}

/// What to tell the model about a fan-out that has run.
fn told(verb: &str, me: &policy::Whom, landed: &[String], refused: &[String]) -> Answer {
    let urgent = if verb == "trouble" {
        ", marked so it can interrupt whatever they are doing"
    } else {
        ""
    };
    let mut said = if landed.is_empty() {
        format!("Nothing in run `{}` took it.", me.session_root())
    } else {
        format!(
            "In the inbox of {} of run `{}`{urgent}: {}.",
            landed.len(),
            me.session_root(),
            landed.join(", ")
        )
    };
    if !refused.is_empty() {
        // Named: a coordinator heard by three of eight will plan on the eight otherwise.
        said.push_str(&format!(
            " Not delivered to {}: {}.",
            refused.len(),
            refused.join(", ")
        ));
    }
    // A fan-out that reached nobody is a failure; a partial one is not.
    if landed.is_empty() {
        Answer::refused(said)
    } else {
        Answer::said(said)
    }
}

/// The first sentence of a refusal, for putting several of them on one line.
fn first_line(said: &str) -> &str {
    said.lines().next().unwrap_or(said)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::directory::{listening_at, roles, sessions};
    use crate::identity::Identity;
    use crate::scratch::Project;

    /// A project of its own, removed on drop — see [`crate::scratch`].
    fn alone(name: &str) -> Project {
        Project::new("melchior-fan", name)
    }

    /// Bind a socket, leave the notes beside it, and answer every call with a bare success. It has
    /// to answer, not merely exist: a bound socket nobody serves costs the caller its patience.
    fn present(
        project: &str,
        id: &str,
        run: &str,
        parent: Option<&str>,
    ) -> std::os::unix::net::UnixListener {
        let me = Identity {
            project: project.to_owned(),
            role: "main".to_owned(),
            id: id.to_owned(),
        };
        std::fs::write(sessions::session_at(&me), run).expect("the run");
        if let Some(parent) = parent {
            crate::directory::adopted(&me, parent).expect("the note");
        }
        roles::given(project, id, &roles::Role::default()).expect("the note");
        let bound = std::os::unix::net::UnixListener::bind(listening_at(&me)).expect("bind");
        let heard = bound.try_clone().expect("a second handle");
        std::thread::spawn(move || {
            for stream in heard.incoming().flatten() {
                std::thread::spawn(move || answering(stream));
            }
        });
        bound
    }

    /// Read frames and answer each one, in the family's shape and by hand.
    fn answering(mut stream: std::os::unix::net::UnixStream) {
        use std::io::{Read, Write};
        let said = br#"{"ok":true,"family":1,"n":1,"result":[{"id":"one"}]}"#;
        loop {
            let mut header = [0_u8; 4];
            if stream.read_exact(&mut header).is_err() {
                return;
            }
            let mut body = vec![0_u8; u32::from_be_bytes(header) as usize];
            if stream.read_exact(&mut body).is_err() {
                return;
            }
            let mut frame = u32::try_from(said.len())
                .unwrap_or_default()
                .to_be_bytes()
                .to_vec();
            frame.extend_from_slice(said);
            if stream.write_all(&frame).is_err() {
                return;
            }
        }
    }

    fn standing(project: &str) -> Standing {
        Standing {
            me: format!("{project}/main/alpha-rho"),
            parent: None,
            forked: Vec::new(),
            minted: std::collections::BTreeMap::new(),
            inbox: Vec::new(),
        }
    }

    #[test]
    fn an_announce_reaches_this_run_and_stops_at_the_edge_of_it() {
        // The roster is the run, so a second run's agent in the same project is not on it.
        let project = alone("run");
        let _bound = (
            present(&project, "alpha-rho", "alpha-rho", None),
            present(&project, "iota-mu", "alpha-rho", Some("alpha-rho")),
            present(&project, "beta-nu", "beta-nu", None),
        );
        let said = fanned(
            "announce",
            &serde_json::json!({"message": "the parser is done"}),
            &standing(&project),
        );
        assert!(!said.failed, "{}", said.said);
        assert!(said.said.contains("iota-mu"), "{}", said.said);
        assert!(
            !said.said.contains("beta-nu"),
            "another run was told: {}",
            said.said
        );
        assert!(
            said.said.starts_with("In the inbox of 1 "),
            "it announced to itself: {}",
            said.said
        );
    }

    #[test]
    fn a_peer_the_setting_puts_out_of_reach_is_named_rather_than_quietly_skipped() {
        // Fanning out is not a way round a refusal: `magi.agent_talk` decides each recipient.
        let project = alone("wall");
        let _bound = (
            present(&project, "alpha-rho", "alpha-rho", None),
            present(&project, "iota-mu", "alpha-rho", Some("alpha-rho")),
            // A grandchild: kin rather than a child, and kin waits for `"instance"`.
            present(&project, "zeta-pi", "alpha-rho", Some("iota-mu")),
        );
        let said = fanned(
            "announce",
            &serde_json::json!({"message": "the parser is done"}),
            &standing(&project),
        );
        assert!(!said.failed, "{}", said.said);
        assert!(said.said.contains("Not delivered to 1"), "{}", said.said);
        assert!(said.said.contains("zeta-pi"), "{}", said.said);
        assert!(
            said.said.contains("agent_talk"),
            "and not which wall: {}",
            said.said
        );
    }

    #[test]
    fn a_run_of_one_is_told_where_to_look_rather_than_that_it_worked() {
        let project = alone("only");
        let _bound = present(&project, "alpha-rho", "alpha-rho", None);
        let said = fanned(
            "announce",
            &serde_json::json!({"message": "hello"}),
            &standing(&project),
        );
        assert!(!said.failed, "{}", said.said);
        assert!(said.said.contains("nobody else"), "{}", said.said);
        assert!(said.said.contains("`send`"), "{}", said.said);
    }

    #[test]
    fn a_message_too_long_for_one_peer_is_not_made_acceptable_by_being_sent_to_twelve() {
        let project = alone("long");
        let _bound = (
            present(&project, "alpha-rho", "alpha-rho", None),
            present(&project, "iota-mu", "alpha-rho", Some("alpha-rho")),
        );
        let said = fanned(
            "announce",
            &serde_json::json!({"message": "z".repeat(sending::AT_MOST + 1)}),
            &standing(&project),
        );
        assert!(said.failed);
        assert!(
            said.said.contains(&sending::AT_MOST.to_string()),
            "{}",
            said.said
        );
    }
}
