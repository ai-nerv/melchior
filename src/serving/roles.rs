//! What a caller says it is for buys it nothing.
//!
//! Split from [`super`] under THE RULE, which caps a file at 800 lines.
//!
//! [`super::placed`] reads a caller's whole name off the frame and throws the middle of it away.
//! That is the one place a role could have got into [`crate::policy`] over a socket: the name is
//! the caller's to write, every process of this user can open the socket, and a role in a
//! [`Whom`](crate::policy::Whom) would be a permission a caller granted itself.
//!
//! **These need a directory with a real `.parent` note in it.** Written against the ambient one
//! first, every caller came back with no parent whatever it signed as — so the assertions held
//! against a `placed` that believed the frame outright. A test that passes against the bug is
//! worse than no test.

use super::placed;
use crate::answering::About;
use crate::directory::{home, kin_at};
use crate::identity::Identity;
use crate::policy::{self, Relation, Whom};

/// A project of its own, holding one agent that is somebody's child.
///
/// `beta-nu` is `gamma-xi`'s subagent, and the directory is the only place that says so — which
/// is the whole point: the caller signs its own name, and the note is what answers back.
fn alone(name: &str) -> String {
    let project = format!("melchior-placed-{}-{name}", std::process::id());
    let _ = std::fs::remove_dir_all(home(&project));
    std::fs::create_dir_all(home(&project)).expect("mkdir");
    std::fs::write(
        kin_at(&Identity {
            project: project.clone(),
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
    // Five names for one agent, differing only in the middle. The directory says `beta-nu` is
    // `gamma-xi`'s subagent; if any signature placed it anywhere else, a caller would be
    // choosing where it stands by choosing what to call itself.
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
    let _ = std::fs::remove_dir_all(home(&project));
}

#[test]
fn calling_yourself_main_does_not_make_you_one() {
    // The sentence the invariant is written in: a session that could pick its own role could
    // pick `main` and claim a main's reach. `Root` is what a main gets at every setting and
    // `Cousin` is what another instance's subagent gets at none, so this is the concrete thing
    // a believed role would have bought.
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
    let _ = std::fs::remove_dir_all(home(&project));
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
    let _ = std::fs::remove_dir_all(home(&project));
}
