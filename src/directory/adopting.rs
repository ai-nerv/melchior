//! A note written by a consenting parent is one every reader agrees with.

use super::*;
use crate::scratch::Project;

/// A project of its own, removed by a guard so a failing assertion does not leave it behind.
fn alone(name: &str) -> Project {
    Project::new("melchior-adopt", name)
}

fn id(project: &str, id: &str) -> Identity {
    Identity {
        project: project.to_owned(),
        role: "main".to_owned(),
        id: id.to_owned(),
    }
}

#[test]
fn the_adopted_session_reads_as_the_adopters_child() {
    // Every reader compares the note against a bare id, so a full name in it reads as a cousin.
    let project = alone("child");
    let parent = id(&project, "beta-omicron");
    let child = id(&project, "psi-eta");

    adopted(&child, &parent.id).expect("the note");

    let theirs = whom(&project, &child.id);
    let mine = whom(&project, &parent.id);
    assert_eq!(
        policy::between(&mine, &theirs),
        policy::Relation::Child,
        "the adopter does not see a child"
    );
    assert_eq!(
        policy::between(&theirs, &mine),
        policy::Relation::Parent,
        "the adopted does not see a parent"
    );
}

#[test]
fn and_shows_up_as_one_of_the_adopters_children() {
    let project = alone("listed");
    let parent = id(&project, "beta-omicron");
    adopted(&id(&project, "psi-eta"), &parent.id).expect("the note");
    // `children` only counts sessions that are listening, so this reads the note instead.
    assert_eq!(
        whom(&project, "psi-eta").parent.as_deref(),
        Some("beta-omicron")
    );
}

#[test]
fn a_session_reads_its_own_parent_off_the_note_rather_than_its_environment() {
    // Being adopted happens from outside: no variable can be set on a running process.
    let project = alone("mine");
    let child = id(&project, "psi-eta");
    assert_eq!(parent_of(&child), None, "it starts with nobody");
    adopted(&child, "beta-omicron").expect("the note");
    assert_eq!(parent_of(&child).as_deref(), Some("beta-omicron"));
}
#[test]
fn an_adopted_subtree_walks_up_to_the_new_root_and_reads_as_its_kin() {
    // B adopts root A, which already had a child C. Only A's `.parent` moves, but membership is
    // walked off the notes, so C's branch is B's run now: C reads as kin to B, not a cousin.
    let project = alone("graft");
    let b = id(&project, "beta-omicron"); // the adopter, a root
    let a = id(&project, "alpha-rho"); // the adopted root
    let c = id(&project, "psi-eta"); // A's own child

    adopted(&c, &a.id).expect("C under A");
    adopted(&a, &b.id).expect("A under B, the graft");

    assert_eq!(
        root_of(&project, &c.id).as_deref(),
        Some("beta-omicron"),
        "C did not walk up past its adopted root to the new one"
    );
    let them = whom(&project, &c.id);
    assert_eq!(them.tree_root(), "beta-omicron");
    assert_eq!(
        policy::between(&whom(&project, &b.id), &them),
        policy::Relation::Kin,
        "the grafted grandchild is not kin to the new root"
    );
}

#[test]
fn dialling_a_name_nobody_is_listening_under_is_an_error() {
    // A socket file outlives the process that made it, so this is the answer for one that ended.
    let missing = Identity {
        project: "no-such-project-here".to_owned(),
        role: "main".to_owned(),
        id: "nobody-nowhere".to_owned(),
    };
    assert!(dial(&missing, &missing).is_err());
}
