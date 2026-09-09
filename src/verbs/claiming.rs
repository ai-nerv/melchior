//! Saying what this session is working on, so two agents do not do it twice.
//!
//! Split from [`super`] under THE RULE, which caps a file at 800 lines.
//!
//! # Advisory, and said so out loud
//!
//! A claim stops nothing. There is no lock on the work itself — melchior does not know what the
//! work *is* — so what a claim buys is that the next agent to consider the same piece of it can
//! find out, before starting, that somebody is already on it. That is worth having and it is not
//! mutual exclusion, and every answer here says which of the two it is: a model told "claimed"
//! that read it as "reserved" would carry on regardless the first time it disagreed.
//!
//! # No message goes anywhere
//!
//! `claim` used to need somebody to tell. It does not: the file *is* how the crew finds out, and
//! `claims` is how they ask. Sending a note to every agent in the run every time somebody picked
//! up a task would spend a turn of everybody's attention on a fact none of them needed yet.

use super::{Answer, Standing};
use crate::directory::claims;

/// Take a piece of work in this session's name.
pub fn take(arguments: &serde_json::Value, standing: &Standing) -> Answer {
    let me = standing.identity();
    let about = about(arguments);
    match claims::take(&me.project, &me.id, about) {
        Err(why) => Answer::refused(why),
        Ok(held) => Answer::said(format!(
            "`{}` is recorded as this session's. Nothing enforces it — the record is what the \
             rest of the crew reads before starting something, and `claims` is how they read it. \
             `release` with the same `about` when this session is done or has changed its mind.",
            held.about
        )),
    }
}

/// Let a piece of work go.
pub fn let_go(arguments: &serde_json::Value, standing: &Standing) -> Answer {
    let me = standing.identity();
    match claims::let_go(&me.project, &me.id, about(arguments)) {
        Err(why) => Answer::refused(why),
        Ok(held) => Answer::said(format!(
            "`{}` is nobody's again. Anybody in the project may take it now.",
            held.about
        )),
    }
}

/// What everybody has said they are on.
///
/// The whole project rather than the run, and the difference matters: two runs in one checkout
/// edit the same files, so a roster-shaped answer would show a coordinator a piece of work as
/// free while another run's agent had it open. [`crate::directory::sessions::crew`] answers who
/// is on this job; this answers what is being touched.
pub fn held(standing: &Standing) -> Answer {
    let me = standing.identity();
    let held = claims::all(&me.project);
    if held.is_empty() {
        return Answer::said(format!(
            "Nothing in `{}` is claimed. `claim` records a piece of work as this session's \
             before starting it, so the rest of the crew can see it is taken.",
            me.project
        ));
    }
    let rows: Vec<String> = held
        .iter()
        .map(|claim| {
            let whose = if claim.by == me.id {
                " — this session's"
            } else {
                ""
            };
            format!(
                "- `{}` — `{}`, {}s ago{whose}",
                claim.about,
                claim.by,
                claim.ago()
            )
        })
        .collect();
    Answer::said(format!(
        "Claimed in `{}`:\n\n{}\n\nThese are claims, not locks: nothing stops an agent working \
         on something somebody else has taken, and a claim whose holder has stopped answering is \
         swept the moment anybody reads this list.",
        me.project,
        rows.join("\n")
    ))
}

/// What piece of work a call names.
fn about(arguments: &serde_json::Value) -> &str {
    arguments
        .get("about")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
}

/// A claim is recorded, read and let go, and none of it is a lock.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::directory::{home, listening_at, socket};
    use crate::identity::Identity;

    /// A project of its own, with a socket, so the holder is one that answers.
    fn alone(name: &str) -> (String, std::os::unix::net::UnixListener) {
        let project = format!("melchior-claiming-{}-{name}", std::process::id());
        let _ = std::fs::remove_dir_all(home(&project));
        std::fs::create_dir_all(home(&project)).expect("mkdir");
        let me = Identity {
            project: project.clone(),
            role: "main".to_owned(),
            id: "alpha-rho".to_owned(),
        };
        let bound = std::os::unix::net::UnixListener::bind(listening_at(&me)).expect("bind");
        (project, bound)
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
    fn a_claim_is_recorded_listed_and_let_go() {
        let (project, _bound) = alone("round");
        let standing = standing(&project);
        let took = take(&serde_json::json!({"about": "src/parser.rs"}), &standing);
        assert!(!took.failed, "{}", took.said);

        let listed = held(&standing);
        assert!(listed.said.contains("src/parser.rs"), "{}", listed.said);
        assert!(listed.said.contains("alpha-rho"), "{}", listed.said);
        // Said out loud, because a model that read "claimed" as "reserved" would carry on the
        // first time it disagreed with the record.
        assert!(listed.said.contains("not locks"), "{}", listed.said);

        let gone = let_go(&serde_json::json!({"about": "src/parser.rs"}), &standing);
        assert!(!gone.failed, "{}", gone.said);
        assert!(held(&standing).said.contains("Nothing in"), "still held");
        let _ = std::fs::remove_dir_all(home(&project));
    }

    #[test]
    fn a_claim_somebody_else_holds_is_refused_by_name() {
        // The whole point of the record: the answer says who to ask, because "no" on its own
        // sends a model back to try the same thing with a different word.
        let (project, _bound) = alone("taken");
        let other = socket(&project, "beta-nu");
        let _theirs = std::os::unix::net::UnixListener::bind(&other).expect("bind");
        claims::take(&project, "beta-nu", "src/parser.rs").expect("theirs");

        let took = take(
            &serde_json::json!({"about": "src/parser.rs"}),
            &standing(&project),
        );
        assert!(took.failed);
        assert!(took.said.contains("beta-nu"), "{}", took.said);
        assert!(
            took.said.contains("claims"),
            "and where to look: {}",
            took.said
        );
        let _ = std::fs::remove_dir_all(home(&project));
    }

    #[test]
    fn letting_go_of_what_this_session_never_took_is_refused() {
        let (project, _bound) = alone("stranger");
        let other = socket(&project, "beta-nu");
        let _theirs = std::os::unix::net::UnixListener::bind(&other).expect("bind");
        claims::take(&project, "beta-nu", "src/parser.rs").expect("theirs");

        let gone = let_go(
            &serde_json::json!({"about": "src/parser.rs"}),
            &standing(&project),
        );
        assert!(
            gone.failed,
            "one session decided another had stopped working"
        );
        assert!(gone.said.contains("beta-nu"), "{}", gone.said);
        let _ = std::fs::remove_dir_all(home(&project));
    }
}
