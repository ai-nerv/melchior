//! What a caller says it is for buys it nothing.
//!
//! [`super::placed`] reads a caller's whole name off the frame and throws the middle of it away.
//! The name is the caller's to write and every process of this user can open the socket, so a
//! role in a [`Whom`](crate::policy::Whom) would be a permission a caller granted itself.

use super::placed;
use crate::answering::About;
use crate::directory::kin_at;
use crate::identity::Identity;
use crate::policy::{self, Relation, Whom};
use crate::scratch::Project;

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
    }
}

fn mine(project: &str) -> Whom {
    Whom {
        project: project.to_owned(),
        id: "alpha-rho".to_owned(),
        parent: None,
        session: None,
    }
}

#[test]
fn the_role_a_caller_signs_with_is_dropped_before_anything_is_decided() {
    // Five names for one agent, differing only in the middle.
    let project = alone("dropped");
    let expected = Some("gamma-xi".to_owned());
    for signed in [
        format!("{project}/scratch/beta-nu"),
        format!("{project}/main/beta-nu"),
        format!("{project}/coordinator/beta-nu"),
        format!("{project}/parent/beta-nu"),
        format!("{project}/beta-nu"),
    ] {
        let placed = placed(Some(&signed), &about(&project)).expect("a caller");
        assert_eq!(
            placed.parent, expected,
            "`{signed}` was placed somewhere the directory does not put it"
        );
    }
}

#[test]
fn calling_yourself_main_does_not_make_you_one() {
    // `Root` is what a main gets at every setting and `Cousin` what another's subagent gets at
    // none, so this is the concrete thing a believed role would have bought.
    let project = alone("main");
    let claiming =
        placed(Some(&format!("{project}/main/beta-nu")), &about(&project)).expect("a caller");
    assert!(
        !claiming.is_main(),
        "signing as `main` made it one, and a main reaches every other main"
    );
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
    let across = placed(Some("other/main/beta-nu"), &about(&project)).expect("a caller");
    assert_eq!(
        policy::between(&mine(&project), &across),
        Relation::Elsewhere
    );
    assert_eq!(across.session, None, "another project's run is not read");
}
