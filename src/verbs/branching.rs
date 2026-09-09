//! Ending a branch: an instance this session started, and everything under it.
//!
//! Nothing here stops a grandchild directly, and nothing here could: the secret that makes a
//! `stop` refusable is held only by the session that minted it, so a grandchild's belongs to its
//! parent and never to this one. What ends a branch is the same `stop` the tool already has — a
//! forked magi watches the pid of the session that forked it and exits when that pid goes, so a
//! stop runs down the branch a generation at a time. This verb names the branch, waits for that
//! cascade, and reports which of it went.
//!
//! The branch is whoever is *listening* under it, re-read on every look. A session that was still
//! coming up when the stop went out is in nobody's directory yet, and one read taken before the
//! stop would have reported it as never having existed.

use super::{Answer, Standing, doing};
use crate::directory::sessions;
use crate::policy::Whom;
use serde_json::Value;

/// How long to wait for a branch to come apart. The cascade is one corpse-check per generation at
/// the far end, so a deep branch takes longer than a shallow one; this bounds the wait rather than
/// predicting it, and the answer says what was still there when it ran out.
const WAIT: std::time::Duration = std::time::Duration::from_secs(8);

/// How long to keep looking at a branch that has shown nothing at all. One corpse-check period at
/// the far end, so a child part-way through announcing itself still lands, without every disband
/// of a leaf paying [`WAIT`] to be told what it already knew.
const GRACE: std::time::Duration = std::time::Duration::from_secs(1);

/// How often to look.
const LOOK: std::time::Duration = std::time::Duration::from_millis(200);

/// End `who` and everything under it. Refused wherever `stop` is refused and in the same words:
/// the branch is read only after this session has proved it may.
pub fn disband(arguments: &Value, standing: &Standing) -> Answer {
    let Some(who) = arguments
        .get("who")
        .and_then(Value::as_str)
        .filter(|who| !who.is_empty())
    else {
        return Answer::refused(
            "`disband` needs `who` — which instance to end, along with everything under it.",
        );
    };
    // The gate first, so a name this session may not stop learns nothing about who is under it.
    let wanted = match doing::decide("stop", who, arguments, standing) {
        Ok(wanted) => wanted,
        Err(refused) => return refused,
    };
    let out = doing::perform(&wanted, standing);
    if out.failed {
        return out;
    }
    let (branch, left) = watched(&standing.whom(), &wanted.who.id, WAIT, GRACE);
    told(&wanted.who.full(), &branch, &left, WAIT)
}

/// Everything seen under `id` while watching, and what of it was still listening when the watch
/// ended. Ends early when a branch that was seen has gone, or when one that was never seen has had
/// `grace` to appear.
fn watched(
    me: &Whom,
    id: &str,
    wait: std::time::Duration,
    grace: std::time::Duration,
) -> (Vec<String>, Vec<String>) {
    let began = std::time::Instant::now();
    let mut ever: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    loop {
        // `crew` dial-tests as it reads, so what comes back is what is listening now.
        let held = sessions::crew(me);
        let left: Vec<String> = sessions::under(&held, id)
            .into_iter()
            .map(|them| them.id.clone())
            .collect();
        ever.extend(left.iter().cloned());
        let over = began.elapsed() >= wait
            || (ever.is_empty() && began.elapsed() >= grace)
            || (!ever.is_empty() && left.is_empty());
        if over {
            return (ever.into_iter().collect(), left);
        }
        std::thread::sleep(LOOK);
    }
}

/// What to tell the model. A half-stopped branch is the answer this verb exists to give: naming
/// what is still there is what a coordinator can act on.
fn told(named: &str, branch: &[String], left: &[String], wait: std::time::Duration) -> Answer {
    if branch.is_empty() {
        return Answer::said(format!(
            "`{named}` was told to stop. Nothing listening in this run names it as the one that \
             started it, so there was no branch to follow."
        ));
    }
    let went: Vec<&String> = branch.iter().filter(|id| !left.contains(*id)).collect();
    let mut said = format!("`{named}` was told to stop.");
    if went.is_empty() {
        said.push_str(" Nothing under it went with it.");
    } else {
        said.push_str(&format!(
            " {} of the {} under it went with it: {}.",
            went.len(),
            branch.len(),
            listed(went.into_iter())
        ));
    }
    if !left.is_empty() {
        said.push_str(&format!(
            " Still answering {wait:?} later: {}. This session did not start those, so it holds \
             nothing that could stop them — `crew` says who did.",
            listed(left.iter())
        ));
    }
    Answer::said(said)
}

fn listed<'a>(ids: impl Iterator<Item = &'a String>) -> String {
    ids.map(|id| format!("`{id}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Project;

    fn standing() -> Standing {
        Standing {
            me: "magi/main/alpha-rho".to_owned(),
            parent: None,
            forked: Vec::new(),
            minted: std::collections::BTreeMap::new(),
            inbox: Vec::new(),
        }
    }

    fn said(branch: &[&str], left: &[&str]) -> String {
        let branch: Vec<String> = branch.iter().map(|id| (*id).to_owned()).collect();
        let left: Vec<String> = left.iter().map(|id| (*id).to_owned()).collect();
        told(
            "magi/main/theta-nu",
            &branch,
            &left,
            std::time::Duration::from_secs(8),
        )
        .said
    }

    #[test]
    fn disbanding_something_this_session_did_not_start_is_refused_word_for_word_as_a_stop_is() {
        let out = disband(&serde_json::json!({"who": "beta-nu"}), &standing());
        assert!(out.failed);
        let stopping = super::super::answer(
            &serde_json::json!({"verb": "stop", "who": "beta-nu"}),
            &standing(),
        );
        assert_eq!(
            out.said, stopping.said,
            "ending a branch is refused on different grounds from ending one instance"
        );
    }

    #[test]
    fn a_branch_this_session_may_not_end_is_never_even_read() {
        // The refusal names the instance and nothing under it: a name this session may not stop
        // must not become a way to ask who is behind it.
        let out = disband(&serde_json::json!({"who": "beta-nu"}), &standing());
        assert!(!out.said.contains("under it"), "{}", out.said);
    }

    #[test]
    fn a_branch_with_no_name_to_end_says_so() {
        let out = disband(&serde_json::json!({}), &standing());
        assert!(out.failed);
        assert!(out.said.contains("who"), "{}", out.said);
    }

    #[test]
    fn holding_the_secret_gets_past_the_gate_and_no_further() {
        // Nothing listens in a test, so reaching the socket is the evidence the gate passed.
        let mut standing = standing();
        standing.forked.push("iota-mu".to_owned());
        standing
            .minted
            .insert("iota-mu".to_owned(), "s3cret".to_owned());
        let out = disband(&serde_json::json!({"who": "iota-mu"}), &standing);
        assert!(out.failed);
        assert!(out.said.contains("nothing is listening"), "{}", out.said);
    }

    #[test]
    fn a_branch_that_half_went_names_both_halves() {
        let out = said(&["psi-eta", "phi-beta"], &["phi-beta"]);
        assert!(out.contains("1 of the 2"), "{out}");
        assert!(out.contains("`psi-eta`"), "it went unnamed: {out}");
        assert!(
            out.contains("Still answering 8s later: `phi-beta`"),
            "the half that stayed went unreported: {out}"
        );
    }

    #[test]
    fn a_branch_that_all_went_names_nobody_left() {
        let out = said(&["psi-eta", "phi-beta"], &[]);
        assert!(out.contains("2 of the 2"), "{out}");
        assert!(!out.contains("Still answering"), "{out}");
    }

    #[test]
    fn a_branch_that_none_of_went_says_that_rather_than_counting_to_zero() {
        let out = said(&["psi-eta"], &["psi-eta"]);
        assert!(out.contains("Nothing under it went with it"), "{out}");
        assert!(out.contains("Still answering"), "{out}");
    }

    #[test]
    fn nothing_under_it_is_reported_as_what_was_seen_and_not_as_what_exists() {
        // It said "nothing was started under it" of a grandchild that had not finished coming up.
        let out = said(&[], &[]);
        assert!(out.contains("Nothing listening"), "{out}");
        assert!(
            !out.contains("was started under it"),
            "it stated as fact what it only read off a directory: {out}"
        );
    }

    #[test]
    fn a_branch_nothing_is_under_gives_up_after_the_grace_rather_than_the_whole_wait() {
        let project = Project::new("melchior-branch", "grace");
        let me = Whom {
            project: project.to_string(),
            id: "alpha-rho".to_owned(),
            parent: None,
            session: Some("alpha-rho-1".to_owned()),
        };
        let began = std::time::Instant::now();
        let (branch, left) = watched(
            &me,
            "theta-nu",
            std::time::Duration::from_secs(30),
            std::time::Duration::from_millis(300),
        );
        assert!(branch.is_empty() && left.is_empty());
        assert!(
            began.elapsed() < std::time::Duration::from_secs(5),
            "every disband of a leaf would pay the whole wait: {:?}",
            began.elapsed()
        );
    }
}
