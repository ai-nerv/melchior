//! Which run an agent belongs to, and who else is in it.
//!
//! Split from [`super`] under THE RULE, which caps a file at 800 lines.
//!
//! # Two notes, two questions
//!
//! `<id>.parent` says who may stop this agent. `<id>.session` says which run it belongs to.
//! They start out saying nearly the same thing and they are not the same fact, which is why
//! there are two files:
//!
//! - **The parent moves.** [`super::adopted`] rewrites it when somebody at a keyboard consents,
//!   and that is the whole point of the handshake.
//! - **The session does not.** It is written once, at announce time, and nothing rewrites it —
//!   not adoption, not a restart. An agent's memory and its place in a roster are filed under
//!   it, and a store cannot have its path change out from under it. Derived by walking `.parent`
//!   instead, the run an agent belonged to would move the moment it was handed to a new parent,
//!   and everything it had already written would be filed under a run it was no longer in.
//!
//! So an adopted agent keeps answering to its new parent and keeps belonging to the run it was
//! born in. That is the honest answer: adoption decides who may direct it, and it does not
//! decide where its work was done.
//!
//! # No `<session>/` in a path
//!
//! The note is a note. `sun_path` is about 104 bytes and the runtime tree stays two levels deep
//! — see [`super::inside`] — so a run cannot become a directory the sockets sit inside without
//! spending a whole path segment on it. Deep durable state, shallow rendezvous.

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

/// Which run a spawned process was started under, from its environment.
///
/// `None` for one nobody handed a run down to, which is what a root is: it is told its run at
/// announce time by writing its own id, not by inheriting one.
#[must_use]
pub fn inherited() -> Option<String> {
    said(SESSION)
}

/// Which run `me` belongs to, as everybody else can see it.
///
/// **The note first, the environment second**, for the same reason [`super::parent_of`] reads
/// its note first: the note is what every other agent reads, and an answer that disagreed with
/// it would put this one in a roster nobody else builds.
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

/// Leave the note saying which run this agent belongs to.
///
/// **A root mints the name**, where it writes no `.parent` at all. The asymmetry is the design:
/// having no parent is what makes a root, so absence says it; being its own run is something a
/// root *is*, and a missing note would leave every reader to guess it — see
/// [`Whom::session_root`](crate::policy::Whom::session_root) for what that guess costs.
pub fn began(me: &Identity) {
    let run = inherited().unwrap_or_else(|| minted(&me.id, since_epoch()));
    let path = session_at(me);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, run);
}

/// A run name that no later run can be handed.
///
/// **The id alone recycles.** [`free_of`](crate::identity::free_of) picks a name none of the
/// *currently listening* sessions holds, so `alpha-rho` comes round again as soon as the first
/// one ends — twenty-four Greek words pair into 552 names, which on a busy machine is weeks
/// rather than years. A run name is what balthasar files a session's history under, and a
/// returning name is not a collision it can see: it reopens the months-old record of the same
/// name and appends this run's transcript to it.
///
/// Stamped rather than made random so the name still reads as the run it is. The id is the part
/// a person and a roster both use; the seconds are only there to stop it coming round.
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

/// Everyone in `me`'s run, `me` included, as the directory has them.
///
/// Listening and nothing else: [`listening`] dials before it reports, so a crew member is one
/// that answers rather than one whose socket a crash left behind. Not filtered by what `me` may
/// reach — the roster is who is in the run, and who may be spoken to is a separate question with
/// its own answer in [`policy`](crate::policy).
#[must_use]
pub fn crew(me: &Whom) -> Vec<Whom> {
    let run = me.session_root().to_owned();
    listening(&me.project)
        .into_iter()
        .map(|id| whom(&me.project, &id))
        .filter(|them| them.session_root() == run)
        .collect()
}

/// The note is written once and read by everybody, and adoption does not move it.
#[cfg(test)]
mod tests {
    use super::*;

    /// A project of its own, so these do not read each other's directory.
    fn alone(name: &str) -> String {
        let project = format!("melchior-session-{}-{name}", std::process::id());
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
    fn a_root_writes_its_own_id_because_absence_would_be_a_guess() {
        let project = alone("root");
        let root = id(&project, "alpha-rho");
        // Through `announce`, because that is the wiring worth testing: the note is written
        // where the socket is announced, and a session that bound without writing it would be
        // on nobody's roster including its own.
        super::super::announce(&root, &crate::directory::roles::Role::default());
        let run = session_of(&root).expect("the note");
        assert!(run.starts_with("alpha-rho-"), "{run}");
        assert_eq!(session_in(&project, "alpha-rho"), Some(run));
        let _ = std::fs::remove_dir_all(home(&project));
    }

    #[test]
    fn a_run_name_does_not_come_round_again_when_its_id_does() {
        // `free_of` only avoids the names currently listening, so `alpha-rho` is handed out
        // again as soon as the first one ends. balthasar files a session's history under the
        // run, and it cannot see a returning name as a collision — it reopens the record of the
        // same name and appends a new run's transcript to a months-old one.
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
        // The invariant the whole two-note split exists for. `adopted` rewrites the parent
        // because somebody consented to it; the run an agent did its work in is not something
        // consent changes, and a store filed under it cannot have its path move.
        let project = alone("adopt");
        let child = id(&project, "psi-eta");
        std::fs::write(session_at(&child), "alpha-rho").expect("the note");

        super::super::adopted(&child, "beta-nu");

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
        let _ = std::fs::remove_dir_all(home(&project));
    }

    #[test]
    fn the_note_sits_beside_the_socket_and_is_not_mistaken_for_one() {
        // `listening` drops anything with a dot in it, and an id is two Greek words and a dash.
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
        // Two runs in one project, and real sockets: `listening` dials before it reports, so a
        // roster built from notes alone would be a roster of nobody. The crew of one run must
        // not contain the other's agents — requirement 2's easy half, restated as a list.
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
        let _ = std::fs::remove_dir_all(home(&project));
    }
}
