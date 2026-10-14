//! What a caller says about itself buys it nothing.
//!
//! [`super::callers`] names a caller by the session its environment names and places it off the
//! project directory. The `from` a call carries, role and all, is never read: every process of
//! this user can open the socket and write any name it likes there.

use super::callers::{Pinned, caller_of};
use crate::answering::About;
use crate::directory::kin_at;
use crate::identity::Identity;
use crate::kin::Kinship;
use crate::policy::{self, Relation, Whom};
use crate::scratch::Project;
use crate::wire::Call;

/// A project of its own, holding one agent that is somebody's child. `beta-nu` is `gamma-xi`'s
/// subagent, and the directory is the only place that says so.
fn alone(name: &str) -> Project {
    let project = Project::new("melchior-placed", name);
    std::fs::write(
        kin_at(&Identity {
            project: project.to_string(),
            role: "main".to_owned(),
            id: "beta-nu".to_owned(),
        }),
        "gamma-xi",
    )
    .expect("the note");
    project
}

fn about(project: &str) -> About {
    About {
        me: Identity {
            project: project.to_owned(),
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

fn mine(project: &str) -> Whom {
    Whom {
        project: project.to_owned(),
        id: "alpha-rho".to_owned(),
        parent: None,
        session: None,
        root: None,
    }
}

/// A connection whose environment named `id` in `project`, and that the kernel said nothing for.
fn named(project: &str, id: &str) -> Pinned {
    Pinned {
        kinship: Some(Kinship::Stranger),
        claimed: Some((project.to_owned(), id.to_owned())),
    }
}

fn signed(name: &str) -> Call {
    Call {
        call: "status".to_owned(),
        from: Some(name.to_owned()),
        ..Call::default()
    }
}

#[test]
fn the_name_a_caller_signs_with_is_never_read() {
    let project = alone("dropped");
    for signed_as in [
        format!("{project}/scratch/delta-pi"),
        format!("{project}/main/alpha-rho"),
        format!("{project}/coordinator/gamma-xi"),
        format!("{project}/beta-nu"),
    ] {
        let placed = caller_of(
            &signed(&signed_as),
            &named(&project, "beta-nu"),
            &about(&project),
        )
        .expect("a caller");
        assert_eq!(placed.id, "beta-nu", "`{signed_as}` was believed");
        assert_eq!(
            placed.parent.as_deref(),
            Some("gamma-xi"),
            "placed somewhere the directory does not put it"
        );
    }
}

#[test]
fn calling_yourself_main_does_not_make_you_one() {
    // `Root` is what a main gets at every setting and `Cousin` what another's subagent gets at
    // none, so this is the concrete thing a believed role would have bought.
    let project = alone("main");
    let claiming = caller_of(
        &signed(&format!("{project}/main/beta-nu")),
        &named(&project, "beta-nu"),
        &about(&project),
    )
    .expect("a caller");
    assert!(!claiming.is_main(), "it read as a main");
    assert_eq!(
        policy::between(&mine(&project), &claiming),
        Relation::Cousin,
        "it got a rung it did not have"
    );
    assert!(
        !policy::may_at(
            &mine(&project),
            policy::between(&mine(&project), &claiming),
            policy::Reach::Ask,
            policy::Talk::Mains,
        ),
        "and the reach that goes with it"
    );
}

#[test]
fn a_name_from_another_project_is_a_stranger_whatever_it_calls_itself() {
    let project = alone("across");
    let across = caller_of(
        &signed(&format!("{project}/main/beta-nu")),
        &named("other", "beta-nu"),
        &about(&project),
    )
    .expect("a caller");
    assert_eq!(
        policy::between(&mine(&project), &across),
        Relation::Elsewhere
    );
    assert_eq!(across.session, None, "another project's run is not read");
}
