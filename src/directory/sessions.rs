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

/// The same roster as a tree: each agent with how many forebears stand between it and the top of
/// its branch, depth-first from the roots and children in id order, so the list reads as the
/// indentation it becomes. Parentage is already on disk, so this is rendering and not new state.
///
/// A `.parent` note naming somebody already above it is a ring, which [`super::adopted`] can
/// write. The ring is cut at the second visit and whatever it swallowed is listed at the root, so
/// no agent in the run can be made invisible by a note somebody else wrote.
#[must_use]
pub fn tiered(held: &[Whom]) -> Vec<(&Whom, usize)> {
    let known: std::collections::BTreeSet<&str> = held.iter().map(|it| it.id.as_str()).collect();
    let mut roots: Vec<&Whom> = held
        .iter()
        .filter(|it| it.parent.as_deref().is_none_or(|up| !known.contains(up)))
        .collect();
    roots.sort_by(|one, two| one.id.cmp(&two.id));
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for root in roots {
        descend(held, root, 0, &mut seen, &mut out);
    }
    for them in held {
        if seen.insert(them.id.clone()) {
            out.push((them, 0));
        }
    }
    out
}

fn descend<'a>(
    held: &'a [Whom],
    them: &'a Whom,
    deep: usize,
    seen: &mut std::collections::BTreeSet<String>,
    out: &mut Vec<(&'a Whom, usize)>,
) {
    if !seen.insert(them.id.clone()) {
        return;
    }
    out.push((them, deep));
    for child in born_to(held, &them.id) {
        descend(held, child, deep + 1, seen, out);
    }
}

/// Everything under `id`, nearest first and never `id` itself. What a branch is, for a caller
/// about to end one: read off the notes rather than asked of anybody, because a session that
/// declined to answer is still in the branch.
#[must_use]
pub fn under<'a>(held: &'a [Whom], id: &str) -> Vec<&'a Whom> {
    let mut seen: std::collections::BTreeSet<String> = [id.to_owned()].into_iter().collect();
    let mut edge = std::collections::VecDeque::from([id.to_owned()]);
    let mut out = Vec::new();
    while let Some(up) = edge.pop_front() {
        for them in born_to(held, &up) {
            if seen.insert(them.id.clone()) {
                out.push(them);
                edge.push_back(them.id.clone());
            }
        }
    }
    out
}

/// Whoever names `id` as the one that started them, in id order.
fn born_to<'a>(held: &'a [Whom], id: &str) -> Vec<&'a Whom> {
    let mut out: Vec<&Whom> = held
        .iter()
        .filter(|it| it.parent.as_deref() == Some(id))
        .collect();
    out.sort_by(|one, two| one.id.cmp(&two.id));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Project;

    fn kin(id: &str, parent: Option<&str>) -> Whom {
        Whom {
            project: "magi".to_owned(),
            id: id.to_owned(),
            parent: parent.map(ToOwned::to_owned),
            session: Some("alpha-rho-1".to_owned()),
        }
    }

    /// Three generations, given out of order so nothing is riding on the directory's own.
    fn three() -> Vec<Whom> {
        vec![
            kin("phi-beta", Some("theta-nu")),
            kin("alpha-rho", None),
            kin("theta-nu", Some("alpha-rho")),
        ]
    }

    #[test]
    fn a_roster_of_three_generations_is_two_levels_of_indent() {
        let held = three();
        let laid: Vec<(&str, usize)> = tiered(&held)
            .into_iter()
            .map(|(them, deep)| (them.id.as_str(), deep))
            .collect();
        assert_eq!(
            laid,
            vec![("alpha-rho", 0), ("theta-nu", 1), ("phi-beta", 2)]
        );
    }

    #[test]
    fn a_grandchild_and_a_sibling_s_child_no_longer_read_the_same() {
        // The flat roster's complaint: both are "kin", and only the tree says which is whose.
        let mut held = three();
        held.push(kin("zeta-pi", Some("alpha-rho")));
        held.push(kin("omega-xi", Some("zeta-pi")));
        let laid: Vec<(&str, usize)> = tiered(&held)
            .into_iter()
            .map(|(them, deep)| (them.id.as_str(), deep))
            .collect();
        assert_eq!(
            laid,
            vec![
                ("alpha-rho", 0),
                ("theta-nu", 1),
                ("phi-beta", 2),
                ("zeta-pi", 1),
                ("omega-xi", 2)
            ]
        );
    }

    #[test]
    fn everybody_is_listed_once_however_the_notes_read() {
        // A ring, which adoption can write, and a parent outside the run.
        let held = vec![
            kin("alpha-rho", Some("theta-nu")),
            kin("theta-nu", Some("alpha-rho")),
            kin("phi-beta", Some("nobody-nowhere")),
        ];
        let laid = tiered(&held);
        assert_eq!(laid.len(), held.len(), "somebody was swallowed by a ring");
        let mut named: Vec<&str> = laid.iter().map(|(them, _)| them.id.as_str()).collect();
        named.sort_unstable();
        assert_eq!(named, vec!["alpha-rho", "phi-beta", "theta-nu"]);
    }

    #[test]
    fn a_branch_is_everything_under_it_and_never_itself() {
        let mut held = three();
        held.push(kin("zeta-pi", Some("alpha-rho")));
        let branch: Vec<&str> = under(&held, "theta-nu")
            .into_iter()
            .map(|them| them.id.as_str())
            .collect();
        assert_eq!(branch, vec!["phi-beta"], "a sibling's branch came along");
        let whole: Vec<&str> = under(&held, "alpha-rho")
            .into_iter()
            .map(|them| them.id.as_str())
            .collect();
        assert_eq!(
            whole,
            vec!["theta-nu", "zeta-pi", "phi-beta"],
            "nearest first"
        );
        assert!(under(&held, "phi-beta").is_empty());
    }

    #[test]
    fn a_ring_under_a_branch_ends_and_never_holds_the_branch_itself() {
        // Adoption wrote the top of the branch back underneath its own grandchild.
        let held = vec![
            kin("theta-nu", Some("psi-eta")),
            kin("phi-beta", Some("theta-nu")),
            kin("psi-eta", Some("phi-beta")),
        ];
        let branch: Vec<&str> = under(&held, "theta-nu")
            .into_iter()
            .map(|them| them.id.as_str())
            .collect();
        assert_eq!(branch, vec!["phi-beta", "psi-eta"]);
        assert!(
            !branch.contains(&"theta-nu"),
            "a branch that holds its own root would be stopped twice: {branch:?}"
        );
    }

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
