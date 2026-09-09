//! Being told what you are for: by the session itself, or by the session that started it, and by
//! nobody else. A role grants nothing — [`crate::policy`] has no field for one — so no token is
//! asked for. Everything else is refused by *relation* rather than by a name in the frame.

use super::{About, Then};
use crate::directory::roles::{AT_MOST, Role};
use crate::policy::Relation;
use crate::wire::{Call, Reply};

/// Whether this caller may say what the answering session is for, and why not.
///
/// `theirs` is how *this* session stands to the caller, so `Child` means the caller started us;
/// asking the other way round would let anything somebody's parent set that somebody's role.
#[must_use]
pub(super) fn refused(relation: Relation, theirs: Relation) -> Option<Reply> {
    if relation == Relation::Myself || theirs == Relation::Child {
        return None;
    }
    Some(Reply::refused(format!(
        "what a session is for is its own to say, or the session that started it: {} is \
         neither. An agent sets its own with `role`.",
        relation.named()
    )))
}

/// Take a role off a call, or say what is wrong with it. An over-long description is refused
/// rather than cut, where the parsing paths cut — see [`Role::new`].
pub(super) fn taken(call: &Call) -> Result<Role, Reply> {
    let Some(name) = super::text_at(call, 0).filter(|name| !name.trim().is_empty()) else {
        return Err(Reply::refused(
            "role takes what to call it, and may take a sentence saying what it does",
        ));
    };
    let description = super::text_at(call, 1).filter(|said| !said.trim().is_empty());
    if let Some(said) = &description
        && said.chars().count() > AT_MOST
    {
        return Err(Reply::refused(format!(
            "that description is {} characters and a role may say {AT_MOST}. It is what a \
             coordinator reads to pick somebody, not where the work is described.",
            said.chars().count()
        )));
    }
    Ok(Role::new(&name, description.as_deref()))
}

/// The same, where naming one at all is optional.
pub(super) fn asked(call: &Call) -> Result<Role, Reply> {
    if super::text_at(call, 0).is_none_or(|name| name.trim().is_empty()) {
        return Ok(Role::default());
    }
    taken(call)
}

/// Answer a `role`, having decided the caller may make it.
pub(super) fn set(call: &Call, about: &About) -> (Reply, Then) {
    match taken(call) {
        Err(refusal) => (refusal, Then::Nothing),
        // Carried up rather than written here: the loop that owns the session writes the note, so
        // a connection handler must not, or one file has two writers.
        Ok(role) => (
            Reply::of(serde_json::json!({
                "role": role.name,
                "description": role.description,
                "full": format!("{}/{}/{}", about.me.project, role.name, about.me.id),
            })),
            Then::Named(role),
        ),
    }
}

/// A role is said by the agent or by its parent, bounded, and never worth anything.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::answering::answer;
    use crate::identity::Identity;
    use crate::policy::Whom;

    fn about(parent: Option<&str>) -> About {
        About {
            me: Identity {
                project: "magi".to_owned(),
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

    fn whom(id: &str, parent: Option<&str>) -> Whom {
        Whom {
            project: "magi".to_owned(),
            id: id.to_owned(),
            parent: parent.map(ToOwned::to_owned),
            session: None,
        }
    }

    fn naming(name: &str, description: Option<&str>) -> Call {
        Call {
            call: "role".to_owned(),
            args: vec![
                serde_json::json!(name),
                description.map_or(serde_json::Value::Null, |said| serde_json::json!(said)),
            ],
            from: None,
            token: None,
        }
    }

    #[test]
    fn a_session_may_say_what_it_is_for() {
        // Requirement 5, and it is safe for exactly one reason: nothing consults the answer.
        let (reply, then) = answer(
            &naming("reviewer", Some("reads diffs")),
            &about(None),
            Some(&whom("alpha-rho", None)),
        );
        assert!(reply.ok, "{reply:?}");
        let Then::Named(role) = then else {
            panic!("nothing was recorded: {then:?}");
        };
        assert_eq!(role.name, "reviewer");
        assert_eq!(role.description.as_deref(), Some("reads diffs"));
    }

    #[test]
    fn the_session_that_started_it_may_say_so_without_the_secret() {
        // The same relation `stop` needs and none of the proof. A secret makes ending a life
        // refusable; a role is a word, and guarding it with the token would say otherwise.
        let call = Call {
            from: Some("magi/main/beta-nu".to_owned()),
            ..naming("reviewer", Some("reads diffs"))
        };
        let (reply, then) = answer(&call, &about(Some("beta-nu")), Some(&whom("beta-nu", None)));
        assert!(reply.ok, "a parent could not assign: {reply:?}");
        assert!(matches!(then, Then::Named(_)), "{then:?}");
    }

    #[test]
    fn nobody_else_may_and_the_refusal_says_who_may() {
        // A sibling, and another instance's main. Neither started this session, and a caller
        // that could name it could name every agent in the project.
        for caller in [whom("zeta-pi", Some("beta-nu")), whom("gamma-xi", None)] {
            let (reply, then) = answer(
                &naming("main", Some("do everything")),
                &about(Some("beta-nu")),
                Some(&caller),
            );
            assert!(!reply.ok, "{} named somebody else: {reply:?}", caller.id);
            assert_eq!(then, Then::Nothing);
            let why = reply.error.unwrap_or_default();
            assert!(why.contains("`role`"), "and did not say what may: {why}");
        }
    }

    #[test]
    fn a_description_over_the_cap_is_refused_and_says_the_number() {
        // Refused, not cut. There is somebody to tell, and router copy that stops half way
        // through a sentence reads as though the agent meant it that way.
        let long = "z".repeat(AT_MOST + 1);
        let (reply, then) = answer(
            &naming("reviewer", Some(&long)),
            &about(None),
            Some(&whom("alpha-rho", None)),
        );
        assert!(!reply.ok, "an uncapped description went through");
        assert_eq!(then, Then::Nothing);
        let why = reply.error.unwrap_or_default();
        assert!(why.contains(&AT_MOST.to_string()), "{why}");
    }

    #[test]
    fn a_role_that_names_nothing_is_refused() {
        let (reply, _) = answer(
            &Call {
                call: "role".to_owned(),
                ..Call::default()
            },
            &about(None),
            Some(&whom("alpha-rho", None)),
        );
        assert!(!reply.ok, "a nameless role went through");
    }
}
