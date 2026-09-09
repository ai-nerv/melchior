//! Which run an agent belongs to, and who else is in it.
//!
//! `<id>.parent` says who may stop this agent, and [`super::adopted`] rewrites it. `<id>.session`
//! says which run it belongs to and is written once: an agent's memory and its place in a roster
//! are filed under the run, and a store cannot have its path change out from under it. The run
//! stays a note rather than a path segment, because `sun_path` is about 104 bytes and the runtime
//! tree stays two levels deep — see [`super::inside`].

use super::{home, listening, safe, whom};
use crate::identity::Identity;
use crate::inherited::{SESSION, said};
use crate::policy::Whom;
use std::path::PathBuf;

/// Where the note saying which run `me` belongs to is put.
#[must_use]
pub fn session_at(me: &Identity) -> PathBuf {
    home(&me.project).join(format!("{}.session", safe(&me.id)))
}

/// Which run a spawned process was started under, from its environment. `None` for a root, which
/// is told its run at announce time by writing its own id rather than by inheriting one.
#[must_use]
pub fn inherited() -> Option<String> {
    said(SESSION)
}

/// Which run `me` belongs to, as everybody else can see it. The note first, the environment
/// second, because the note is what every other agent reads.
#[must_use]
pub fn session_of(me: &Identity) -> Option<String> {
    read(&session_at(me)).or_else(inherited)
}

/// Which run the agent called `id` belongs to, read off the project directory.
#[must_use]
pub fn session_in(project: &str, id: &str) -> Option<String> {
    read(&home(project).join(format!("{}.session", safe(id))))
}

/// The contents of a note, or `None` for one that is missing or blank.
fn read(path: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|said| said.trim().to_owned())
        .filter(|said| !said.is_empty())
}

/// Leave the note saying which run this agent belongs to. A root mints the name, where it writes
/// no `.parent` at all: a missing run note would leave every reader to guess it.
pub fn began(me: &Identity) -> Result<(), String> {
    let run = inherited().unwrap_or_else(|| minted(&me.id, since_epoch()));
    super::wrote(&session_at(me), &run)
}

/// A run name that no later run can be handed. The id alone recycles, because
/// [`free_of`](crate::identity::free_of) only avoids the names currently listening, and balthasar
/// files a session's history under the run: a returning name would reopen a months-old record.
fn minted(id: &str, at: u64) -> String {
    format!("{id}-{at}")
}

/// Seconds since the epoch.
fn since_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// Take the note back down.
pub fn ended(me: &Identity) {
    let _ = std::fs::remove_file(session_at(me));
}

/// The same, for a corpse being swept by somebody else.
pub fn forget_in(project: &str, id: &str) {
    let _ = std::fs::remove_file(home(project).join(format!("{}.session", safe(id))));
}

/// Everyone in `me`'s run, `me` included, and not filtered by what `me` may reach.
#[must_use]
pub fn crew(me: &Whom) -> Vec<Whom> {
    let run = me.session_root().to_owned();
    listening(&me.project)
        .into_iter()
        .map(|id| whom(&me.project, &id))
        .filter(|them| them.session_root() == run)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Project;

    /// A project of its own, removed by a guard so a failing test does not leave it behind.
    fn alone(name: &str) -> Project {
        Project::new("melchior-session", name)
    }

    fn id(project: &str, id: &str) -> Identity {
        Identity {
            project: project.to_owned(),
            role: "main".to_owned(),
            id: id.to_owned(),
        }
    }

    #[test]
    fn a_root_writes_its_own_id_because_absence_would_be_a_guess() {
        let project = alone("root");
        let root = id(&project, "alpha-rho");
        // Through `announce`, because that is where the note is written.
        super::super::announce(&root, &crate::directory::roles::Role::default(), None)
            .expect("announced");
        let run = session_of(&root).expect("the note");
        assert!(run.starts_with("alpha-rho-"), "{run}");
        assert_eq!(session_in(&project, "alpha-rho"), Some(run));
    }

    #[test]
    fn a_run_name_does_not_come_round_again_when_its_id_does() {
        // `free_of` only avoids the names currently listening, so an id is handed out again.
        let earlier = minted("alpha-rho", 1_700_000_000);
        let later = minted("alpha-rho", 1_800_000_000);
        assert_ne!(earlier, later, "the same id twice named one run");
        assert!(
            earlier.starts_with("alpha-rho") && later.starts_with("alpha-rho"),
            "and it still reads as the run a roster names"
        );
    }

    #[test]
    fn adoption_does_not_move_which_run_an_agent_belongs_to() {
        // The invariant the two-note split exists for: a store filed under a run cannot move.
        let project = alone("adopt");
        let child = id(&project, "psi-eta");
        std::fs::write(session_at(&child), "alpha-rho").expect("the note");

        super::super::adopted(&child, "beta-nu").expect("the note");

        assert_eq!(
            whom(&project, "psi-eta").parent.as_deref(),
            Some("beta-nu"),
            "the parent did not move, and it is the half that should have"
        );
        assert_eq!(
            session_in(&project, "psi-eta").as_deref(),
            Some("alpha-rho"),
            "adoption rewrote the run, and everything filed under the old one is now orphaned"
        );
    }

    #[test]
    fn the_note_sits_beside_the_socket_and_is_not_mistaken_for_one() {
        let me = id("magi", "alpha-rho");
        let path = session_at(&me);
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
    fn the_roster_is_the_run_and_not_the_project() {
        // Two runs in one project, and real sockets: `listening` dials before it reports.
        let project = alone("crew");
        let _bound: Vec<_> = [
            ("alpha-rho", "alpha-rho"),
            ("iota-mu", "alpha-rho"),
            ("beta-nu", "beta-nu"),
        ]
        .into_iter()
        .map(|(agent, run)| {
            let me = id(&project, agent);
            std::fs::write(session_at(&me), run).expect("the note");
            std::os::unix::net::UnixListener::bind(super::super::listening_at(&me)).expect("bind")
        })
        .collect();

        let held: Vec<String> = crew(&whom(&project, "alpha-rho"))
            .into_iter()
            .map(|them| them.id)
            .collect();
        assert!(held.contains(&"alpha-rho".to_owned()), "{held:?}");
        assert!(held.contains(&"iota-mu".to_owned()), "{held:?}");
        assert!(
            !held.contains(&"beta-nu".to_owned()),
            "another run's agent is on the roster: {held:?}"
        );
    }
}
