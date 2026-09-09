//! What a session killed outright leaves behind, and whether the sweep takes all of it.
//!
//! `SIGKILL` is the death nothing can be waited for and no destructor sees: melchior stops where
//! it stands and everything it wrote is still on disk. `listening` is the whole answer to that —
//! it dials every name it lists and takes down the ones that do not answer — and the question
//! here is whether it takes down *all* of what a session leaves. A sweep that unlinks the socket
//! and leaves the notes beside it reads as working from every angle anybody looks at it: the
//! roster is right, nobody is offered a dead name, and the directory fills up anyway, one `.role`
//! and one `.session` at a time.
//!
//! Against the library rather than the binary, because the six files are the point and naming
//! them is how this stays a test of coverage rather than of one session's tidiness.

use melchior::directory::{self, claims, roles, screens, sending, sessions};
use melchior::identity::Identity;
use melchior::scratch::Project;

/// A session that wrote every one of the six, the way a session writes them.
///
/// Bound first, and by a real listener: two of the six exist only for a session that answers —
/// [`sending::allow`] refuses to record a send for one that never bound, and the sweep only ever
/// looks at names a socket is sitting under.
fn a_session_that_left_everything(
    project: &str,
) -> (
    Identity,
    std::os::unix::net::UnixListener,
    [std::path::PathBuf; 6],
) {
    let me = Identity {
        project: project.to_owned(),
        role: "main".to_owned(),
        id: "chi-beta".to_owned(),
    };
    let bound = std::os::unix::net::UnixListener::bind(directory::listening_at(&me)).expect("bind");
    let role = roles::Role::new("main", Some("does the work"));
    let ui = std::path::Path::new("/run/nothing/ui.sock");
    directory::announce(&me, &role, Some(ui)).expect("the notes");
    directory::adopted(&me, "alpha-rho").expect("the parent note");
    sending::allow(&me, "tell:alpha-rho:hello").expect("the sent note");
    claims::take(project, &me.id, "the-work").expect("a claim");
    let files = [
        directory::listening_at(&me),
        directory::kin_at(&me),
        sessions::session_at(&me),
        roles::role_at(&me),
        screens::ui_at(&me),
        sending::sent_at(project, &me.id),
    ];
    (me, bound, files)
}

#[test]
fn a_corpse_takes_all_six_of_its_files_with_it() {
    // A project of its own, and a guard rather than a name: a failing assertion unwinds straight
    // past a trailing `remove_dir_all` — see `melchior::scratch`.
    let project = Project::new("melchior-sweep", "all-six");
    let (_me, bound, files) = a_session_that_left_everything(&project);
    for path in &files {
        assert!(path.exists(), "{} was never written", path.display());
    }

    // Killed the way `kill -9` kills: the socket file stays exactly where it is and there is
    // nothing behind it any more. Nothing in the process ran, so this is all the sweep has.
    drop(bound);

    assert!(
        directory::listening(&project).is_empty(),
        "a name nothing answers is still being offered"
    );
    let left: Vec<String> = files
        .iter()
        .filter(|path| path.exists())
        .map(|path| path.display().to_string())
        .collect();
    assert!(
        left.is_empty(),
        "the sweep took the socket and left these: {left:?}"
    );
    assert!(
        claims::all(&project).is_empty(),
        "a claim nobody can be asked about is work that will never be done again"
    );
}
