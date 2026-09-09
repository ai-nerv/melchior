//! Where an agent's harness draws it, so a peer can find the screen and not just the socket.
//!
//! An agent has two doors and only one of them is in this directory. `<project>/<id>` is where
//! other agents reach it; the other is the harness's own socket, which melchior does not open,
//! does not name and cannot work out — magi names its host socket under its own runtime directory
//! by a pid and a clock. So the harness says it once, at `serve --ui <path>`, and this directory
//! then answers "where does `beta-nu` draw" off a file. The note holds that path rather than
//! becoming one, because the runtime tree stays two levels deep — see [`super::inside`].

use super::{home, safe};
use crate::identity::Identity;
use std::path::{Path, PathBuf};

/// What the note is called, after the id it belongs to. The dot is what keeps it out of the
/// roster: [`super::listening`] dials every dotless entry in a project directory as a socket.
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

/// Leave the note saying where this agent draws, or take down whatever an earlier run left. The
/// ids recycle and the notes must not: a stale note from an id's previous holder would point
/// every peer at a socket belonging to somebody who has gone.
pub fn began(me: &Identity, ui: Option<&Path>) -> Result<(), String> {
    let path = ui_at(me);
    let Some(said) = ui.and_then(written) else {
        return match std::fs::remove_file(&path) {
            Err(why) if why.kind() != std::io::ErrorKind::NotFound => {
                Err(format!("{}: {why}", path.display()))
            }
            _ => Ok(()),
        };
    };
    super::wrote(&path, &said)
}

/// The path as it goes in the note, made absolute here, or `None` for one that says nothing. The
/// reader is another process in another working directory.
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

/// Where the agent called `id` draws, read off the project directory. `None` for one that
/// published none, which is every `melchior serve` run by hand.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Project;

    /// A project of its own, removed by a guard so a failing test does not leave it behind.
    fn alone(name: &str) -> Project {
        Project::new("melchior-screen", name)
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
        // Asserted on the name: a note called `<id>ui` fails the dial anyway, so the end-to-end
        // assertions below stay green while it is dial-tested and swept on every pass.
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
        )
        .expect("announced");

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
    }

    #[test]
    fn the_note_sits_beside_the_socket_and_is_not_mistaken_for_one() {
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
        began(&me, Some(&ui)).expect("the note");

        assert_eq!(ui_in(&project, "zeta-pi").as_ref(), Some(&ui));
        ended(&me);
        assert_eq!(ui_in(&project, "zeta-pi"), None);
    }

    #[test]
    fn a_session_that_publishes_nothing_takes_the_last_one_down_with_it() {
        // The ids recycle, so a note left by the previous holder would be drawn by every peer.
        let project = alone("stale");
        let me = id(&project, "zeta-pi");
        began(&me, Some(Path::new("/run/user/1000/magi/magi/older.host"))).expect("the note");
        began(&me, None).expect("the note");
        assert_eq!(ui_in(&project, "zeta-pi"), None);
        began(&me, Some(Path::new("   "))).expect("the note");
        assert_eq!(ui_in(&project, "zeta-pi"), None, "blank is nothing said");
    }

    #[test]
    fn a_relative_path_is_settled_here_rather_than_by_whoever_reads_it() {
        // Left relative, one note names as many sockets as there are readers.
        let project = alone("relative");
        let me = id(&project, "zeta-pi");
        began(&me, Some(Path::new("magi.host"))).expect("the note");
        let said = ui_in(&project, "zeta-pi").expect("a note");
        assert!(said.is_absolute(), "{said:?}");
        assert!(said.ends_with("magi.host"), "{said:?}");
    }

    #[test]
    fn a_corpse_leaves_no_screen_behind() {
        let project = alone("sweep");
        began(
            &id(&project, "zeta-pi"),
            Some(Path::new("/run/user/1000/magi/magi/gone.host")),
        )
        .expect("the note");
        forget_in(&project, "zeta-pi");
        assert_eq!(ui_in(&project, "zeta-pi"), None);
    }
}
