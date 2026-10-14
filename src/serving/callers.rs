//! Who a connection is, settled once when it is accepted and held for every call it makes.
//!
//! Two facts, and they are worth different things. The kernel says how the connecting process
//! stands to the session that started this `serve` — [`crate::kin`] — and that is authority: the
//! session's own process and what it started speak *as* this session. The process's environment
//! names a session too, and that is attribution only: it says who a message is from, and never
//! makes anybody this session. A name the call itself carries is read for neither.

use crate::answering::About;
use crate::directory::whom;
use crate::kin::Kinship;
use crate::policy::Whom;
use crate::wire::Call;

/// What was learned about a connection when it was accepted.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Pinned {
    /// How the kernel says the far end stands to this session.
    pub kinship: Option<Kinship>,
    /// The session its environment names, as `(project, id)`.
    pub claimed: Option<(String, String)>,
}

/// Read the far end, process `pid`, once: its kinship from `/proc/<pid>/stat`, its name from
/// `/proc/<pid>/environ`, and the start time checked again after, so a pid reused in between
/// pins nothing.
#[must_use]
pub fn pinned(pid: Option<i32>, origin: Option<&crate::kin::Origin>) -> Pinned {
    let Some(pid) = pid else {
        return Pinned::default();
    };
    let Some(before) = crate::kin::stat(pid) else {
        return Pinned::default();
    };
    let environ = crate::kin::environ(pid).unwrap_or_default();
    if crate::kin::stat(pid).is_none_or(|after| after.start != before.start) {
        return Pinned::default();
    }
    Pinned {
        kinship: origin.map(|origin| origin.kinship(&before)),
        claimed: var_in(&environ, crate::inherited::PROJECT)
            .zip(var_in(&environ, crate::inherited::ID)),
    }
}

/// Who to treat as the caller of `call`. This session, only when the kernel vouches for the far
/// end and its environment names no other; `mint` alone is also taken from what the session's own
/// program started. Otherwise whoever the environment names, or nobody — never `call.from`.
#[must_use]
pub fn caller_of(call: &Call, pinned: &Pinned, about: &About) -> Option<Whom> {
    let me = (about.me.project.clone(), about.me.id.clone());
    let names_me = pinned.claimed.as_ref().is_none_or(|named| *named == me);
    let vouched = match pinned.kinship {
        Some(Kinship::Session) => true,
        Some(Kinship::Spawner) => call.call == "mint",
        _ => false,
    };
    if vouched && names_me {
        return Some(place(&me.0, &me.1, about));
    }
    match &pinned.claimed {
        Some(named) if *named != me => Some(place(&named.0, &named.1, about)),
        _ => None,
    }
}

/// Where a session by this project and id sits in the tree. Another project's is placed with no
/// parent and no run: its directory is not one this session lists.
fn place(project: &str, id: &str, about: &About) -> Whom {
    if project != about.me.project {
        return Whom {
            project: project.to_owned(),
            id: id.to_owned(),
            parent: None,
            session: None,
            root: None,
        };
    }
    whom(project, id)
}

/// One variable out of a NUL-separated `/proc/<pid>/environ` block, the older `MAGI_*` spelling too.
fn var_in(environ: &[u8], name: &str) -> Option<String> {
    let find = |key: &str| {
        let prefix = format!("{key}=");
        environ
            .split(|byte| *byte == 0)
            .filter_map(|entry| std::str::from_utf8(entry).ok())
            .find_map(|entry| entry.strip_prefix(&prefix))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    find(name).or_else(|| {
        find(&format!(
            "MAGI_{}",
            name.trim_start_matches("MAGI_MELCHIOR_")
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Identity;

    fn about() -> About {
        About {
            me: Identity {
                project: "no-such-project-callers".to_owned(),
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

    fn call(verb: &str, from: Option<&str>) -> Call {
        Call {
            call: verb.to_owned(),
            from: from.map(ToOwned::to_owned),
            ..Call::default()
        }
    }

    fn pinned(kinship: Option<Kinship>, claimed: Option<&str>) -> Pinned {
        Pinned {
            kinship,
            claimed: claimed.map(|id| ("no-such-project-callers".to_owned(), id.to_owned())),
        }
    }

    #[test]
    fn a_peer_with_no_name_and_no_kin_is_nobody_whatever_its_call_says() {
        let anonymous = pinned(Some(Kinship::Stranger), None);
        for verb in ["tell", "stop", "status", "inbox", "mint"] {
            let named = call(verb, Some("no-such-project-callers/main/beta-nu"));
            assert_eq!(caller_of(&named, &anonymous, &about()), None, "{verb}");
        }
    }

    #[test]
    fn the_session_s_own_processes_are_this_session() {
        let own = pinned(Some(Kinship::Session), None);
        let caller = caller_of(&call("minted", None), &own, &about()).expect("a caller");
        assert_eq!(caller.id, "alpha-rho");
        let named_so = pinned(Some(Kinship::Session), Some("alpha-rho"));
        assert_eq!(
            caller_of(&call("tool", None), &named_so, &about()).map(|it| it.id),
            Some("alpha-rho".to_owned())
        );
    }

    #[test]
    fn a_forged_environment_names_a_session_and_is_never_this_one() {
        let forged = pinned(Some(Kinship::Stranger), Some("alpha-rho"));
        assert_eq!(caller_of(&call("minted", None), &forged, &about()), None);
        let other = pinned(Some(Kinship::Stranger), Some("beta-nu"));
        let caller = caller_of(&call("minted", None), &other, &about()).expect("attributed");
        assert_eq!(
            caller.id, "beta-nu",
            "attribution, which no authority verb accepts"
        );
    }

    #[test]
    fn what_the_session_s_own_program_started_may_name_a_child_and_nothing_else() {
        let spawner = pinned(Some(Kinship::Spawner), Some("alpha-rho"));
        assert_eq!(
            caller_of(&call("mint", None), &spawner, &about()).map(|it| it.id),
            Some("alpha-rho".to_owned())
        );
        for verb in ["minted", "tool", "inbox", "role"] {
            assert_eq!(
                caller_of(&call(verb, None), &spawner, &about()),
                None,
                "{verb}"
            );
        }
    }

    #[test]
    fn a_name_is_read_out_of_an_environment_block() {
        let block = b"HOME=/h\0MAGI_MELCHIOR_PROJECT=demo\0MAGI_ID=beta-nu\0EMPTY=\0";
        assert_eq!(
            var_in(block, crate::inherited::PROJECT).as_deref(),
            Some("demo")
        );
        assert_eq!(
            var_in(block, crate::inherited::ID).as_deref(),
            Some("beta-nu")
        );
        assert_eq!(var_in(block, "EMPTY"), None);
    }
}
