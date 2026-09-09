//! Where an agent's harness draws it, so a peer can find the screen and not just the socket.
//!
//! Split from [`super`] under THE RULE, which caps a file at 800 lines.
//!
//! # The missing link
//!
//! An agent has two doors and only one of them is in this directory. `<project>/<id>` is where
//! *other agents* reach it — it answers verbs, and melchior binds it. The other door is the
//! harness's own: the socket a UI draws a transcript over, which melchior does not open, does
//! not name and cannot work out.
//!
//! It cannot work it out because the harness deliberately made it unguessable. magi names its
//! host socket `<project>/<key>.host` under its *own* runtime directory, where `key` is a pid
//! and a clock — unique among the sessions that could collide, and never shown to anybody. That
//! directory is enumerable and every session in it can be dial-tested, so a sibling could always
//! find *a* live magi there; what it could never learn is which agent that magi was. A roster of
//! names on one side and a heap of opaque keys on the other, with nothing joining them.
//!
//! So the harness says, once, at the moment it starts the layer: `serve --ui <path>`. One flag,
//! one note. After that the directory answers "where does `beta-nu` draw" the same way it
//! already answers "whose subagent is `beta-nu`" — read off a file, without asking the session
//! anything and without trusting what it would have said.
//!
//! # The note holds a path; it does not become one
//!
//! `sun_path` is about 104 bytes, and melchior's runtime tree stays two levels deep — see
//! [`super::inside`], which enforces it. A screen reached by putting the harness's socket
//! *inside* this directory would spend a path segment on it and buy nothing: the socket already
//! exists, somewhere else, under a name its owner chose. What was missing was never a place to
//! put one, only the sentence saying where it is.
//!
//! # The name has a dot in it, and that is load-bearing
//!
//! [`super::listening`] lists a project's directory, keeps every entry whose name holds no dot,
//! and then *dials it as a socket*. A note called `<id>.ui` is skipped by that filter. One
//! called `<id>ui` would not be: it would be listed as an agent, offered to a model as one,
//! dial-tested on every sweep and — failing that dial — handed to the sweep that deletes a
//! corpse's socket and notes. The dot is the whole of what prevents that, so it is [`NOTE`],
//! named once, with a test that asserts on the name rather than on the roster.

use super::{home, safe};
use crate::identity::Identity;
use std::path::{Path, PathBuf};

/// What the note is called, after the id it belongs to.
///
/// A constant because a test reads it. The dot is not decoration — see the module note — and a
/// suffix spelled out at each of the five places that touch one is a suffix that loses its dot
/// at exactly one of them.
pub const NOTE: &str = ".ui";

/// Where the note saying where `me` draws is put.
#[must_use]
pub fn ui_at(me: &Identity) -> PathBuf {
    at(&me.project, &me.id)
}

/// The same, by the two parts of a name that place it.
fn at(project: &str, id: &str) -> PathBuf {
    home(project).join(format!("{}{NOTE}", safe(id)))
}

/// Leave the note saying where this agent draws, or take down whatever an earlier run left.
///
/// **Removed when there is nothing to say**, rather than left alone. A melchior started without
/// `--ui` is a session with no screen to offer — `serve` run by hand, or a harness that does not
/// have one — and a stale note from the id's previous holder would point every peer at a socket
/// belonging to somebody who has gone. The ids recycle; the notes must not.
pub fn began(me: &Identity, ui: Option<&Path>) {
    let path = ui_at(me);
    let Some(said) = ui.and_then(written) else {
        let _ = std::fs::remove_file(path);
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, said);
}

/// The path as it goes in the note, or `None` for one that says nothing.
///
/// **Made absolute here.** The reader is another process, in another working directory, and a
/// relative path means a different socket to each of them — which fails as "nothing is listening
/// there" rather than as anything a reader could act on. A harness that passes an absolute path,
/// which is the ordinary case, gets it back unchanged.
fn written(ui: &Path) -> Option<String> {
    let said = ui.to_string_lossy();
    let said = said.trim();
    if said.is_empty() {
        return None;
    }
    let path = Path::new(said);
    if path.is_absolute() {
        return Some(said.to_owned());
    }
    let here = std::env::current_dir().ok()?;
    Some(here.join(path).to_string_lossy().into_owned())
}

/// Where the agent called `id` draws, read off the project directory.
///
/// `None` for an agent that published none, which is every session started before this existed
/// and every `melchior serve` run by hand. A peer that finds nothing here has found out that
/// there is no screen to attach to, which is a fact rather than a failure.
#[must_use]
pub fn ui_in(project: &str, id: &str) -> Option<PathBuf> {
    let said = std::fs::read_to_string(at(project, id)).ok()?;
    let said = said.trim();
    if said.is_empty() {
        return None;
    }
    Some(PathBuf::from(said))
}

/// Take the note back down.
pub fn ended(me: &Identity) {
    let _ = std::fs::remove_file(ui_at(me));
}

/// The same, for a corpse being swept by somebody else.
pub fn forget_in(project: &str, id: &str) {
    let _ = std::fs::remove_file(at(project, id));
}

/// A screen is published by its own harness, read by everybody, and is not an agent.
#[cfg(test)]
mod tests {
    use super::*;

    /// A project of its own, so these do not read each other's directory.
    fn alone(name: &str) -> String {
        let project = format!("melchior-screen-{}-{name}", std::process::id());
        let _ = std::fs::remove_dir_all(home(&project));
        std::fs::create_dir_all(home(&project)).expect("mkdir");
        project
    }

    fn id(project: &str, id: &str) -> Identity {
        Identity {
            project: project.to_owned(),
            role: "main".to_owned(),
            id: id.to_owned(),
        }
    }

    #[test]
    fn the_screen_note_is_never_offered_as_an_agent() {
        // The trap. `listening` keeps every entry with no dot in its name and only *then* dials
        // it — and a note fails that dial, so a screen note called `<id>ui` drops out of the
        // roster anyway and every end-to-end assertion below stays green while the trap is open.
        // What goes wrong is everything in between: it is dial-tested on every sweep, offered to
        // a model as a session, and handed to the sweep that deletes an agent's socket and notes.
        // So this asserts on the *name*, which is where breaking it shows.
        assert!(
            NOTE.contains('.'),
            "`{NOTE}` has no dot in it, so `listening` would keep it and hand it to the sweep"
        );

        let project = alone("hidden");
        let me = id(&project, "alpha-rho");
        let _bound =
            std::os::unix::net::UnixListener::bind(super::super::listening_at(&me)).expect("bind");
        super::super::announce(
            &me,
            &super::super::roles::Role::default(),
            Some(Path::new("/run/user/1000/magi/magi/deadbeef.host")),
        );

        let listed = super::super::listening(&project);
        assert_eq!(listed, vec!["alpha-rho".to_owned()], "{listed:?}");
        let crew: Vec<String> =
            super::super::sessions::crew(&super::super::whom(&project, "alpha-rho"))
                .into_iter()
                .map(|them| them.id)
                .collect();
        assert_eq!(crew, vec!["alpha-rho".to_owned()], "{crew:?}");
        assert!(
            ui_in(&project, "alpha-rho").is_some(),
            "and the sweep took the note away on its way past"
        );
        let _ = std::fs::remove_dir_all(home(&project));
    }

    #[test]
    fn the_note_sits_beside_the_socket_and_is_not_mistaken_for_one() {
        // `listening` drops anything with a dot in it, and an id is two Greek words and a dash.
        let me = id("magi", "alpha-rho");
        let path = ui_at(&me);
        assert_eq!(path.parent(), super::super::listening_at(&me).parent());
        assert!(
            path.file_name()
                .expect("a name")
                .to_string_lossy()
                .contains('.'),
            "{path:?} would be listed as an agent"
        );
    }

    #[test]
    fn what_the_harness_published_is_what_every_other_agent_reads() {
        let project = alone("note");
        let me = id(&project, "zeta-pi");
        let ui = PathBuf::from("/run/user/1000/magi/magi/1f4a0b3c.host");
        began(&me, Some(&ui));

        assert_eq!(ui_in(&project, "zeta-pi").as_ref(), Some(&ui));
        ended(&me);
        assert_eq!(ui_in(&project, "zeta-pi"), None);
        let _ = std::fs::remove_dir_all(home(&project));
    }

    #[test]
    fn a_session_that_publishes_nothing_takes_the_last_one_down_with_it() {
        // The ids recycle — `free_of` only avoids the names currently listening — so a note left
        // by the previous holder of `zeta-pi` would send every peer to a socket that belongs to
        // somebody who has gone, and that peer would draw it.
        let project = alone("stale");
        let me = id(&project, "zeta-pi");
        began(&me, Some(Path::new("/run/user/1000/magi/magi/older.host")));
        began(&me, None);
        assert_eq!(ui_in(&project, "zeta-pi"), None);
        began(&me, Some(Path::new("   ")));
        assert_eq!(ui_in(&project, "zeta-pi"), None, "blank is nothing said");
        let _ = std::fs::remove_dir_all(home(&project));
    }

    #[test]
    fn a_relative_path_is_settled_here_rather_than_by_whoever_reads_it() {
        // The reader is another process in another working directory. Left relative, one note
        // names as many sockets as there are readers, and every one of them is missing.
        let project = alone("relative");
        let me = id(&project, "zeta-pi");
        began(&me, Some(Path::new("magi.host")));
        let said = ui_in(&project, "zeta-pi").expect("a note");
        assert!(said.is_absolute(), "{said:?}");
        assert!(said.ends_with("magi.host"), "{said:?}");
        let _ = std::fs::remove_dir_all(home(&project));
    }

    #[test]
    fn a_corpse_leaves_no_screen_behind() {
        let project = alone("sweep");
        began(
            &id(&project, "zeta-pi"),
            Some(Path::new("/run/user/1000/magi/magi/gone.host")),
        );
        forget_in(&project, "zeta-pi");
        assert_eq!(ui_in(&project, "zeta-pi"), None);
        let _ = std::fs::remove_dir_all(home(&project));
    }
}
