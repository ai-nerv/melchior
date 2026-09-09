//! A note written by a consenting parent is one every reader agrees with.
//!
//! Split from [`super`] under THE RULE, which caps a file at 800 lines.

use super::*;
use crate::scratch::Project;

/// A project of its own, so these do not read each other's directory.
///
/// A guard rather than a name: these used to remove the directory on their last line, which a
/// failing assertion unwinds straight past — see [`crate::scratch`].
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
    // The bug this is here for: the note was written as a full name and every reader
    // compares it against a bare id, so the child read as a *cousin* — and the session that
    // had just accepted it was refused for reaching another instance's subagent.
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
    // The other reader of the same note, and it compares the same way.
    let project = alone("listed");
    let parent = id(&project, "beta-omicron");
    adopted(&id(&project, "psi-eta"), &parent.id).expect("the note");
    // `children` only counts sessions that are listening, so this asserts the note is read
    // rather than that the pair is live.
    assert_eq!(
        whom(&project, "psi-eta").parent.as_deref(),
        Some("beta-omicron")
    );
}

#[test]
fn a_session_reads_its_own_parent_off_the_note_rather_than_its_environment() {
    // Being adopted happens from outside: no variable can be set on a running process. Read
    // from the environment alone, an adopted session went on calling itself a main while
    // everybody else saw a child — and the rule against a second parent tests exactly that.
    let project = alone("mine");
    let child = id(&project, "psi-eta");
    assert_eq!(parent_of(&child), None, "it starts with nobody");
    adopted(&child, "beta-omicron").expect("the note");
    assert_eq!(parent_of(&child).as_deref(), Some("beta-omicron"));
}
#[test]
fn dialling_a_name_nobody_is_listening_under_is_an_error() {
    // The half that used to live on `Held::to`: a name resolves to a path, and a path with
    // nothing behind it is an error rather than a wait. A socket file outlives the process
    // that made it, so this is the ordinary answer for a session that ended.
    let missing = Identity {
        project: "no-such-project-here".to_owned(),
        role: "main".to_owned(),
        id: "nobody-nowhere".to_owned(),
    };
    assert!(dial(&missing, &missing).is_err());
}
