//! What a session has said it is working on, so two of them do not do it twice.
//!
//! One file per claim, given its name by a single exclusive syscall, so two agents asking for the
//! same piece of work at once are settled by the kernel and the loser reads back who beat it. A
//! claim is refused only if the file is there and its holder still answers its socket, which is
//! the liveness test [`super::listening`] applies to a socket.

use super::{answers, home, safe, socket};
use std::path::{Path, PathBuf};

/// What the directory of claims is called. The leading dot is what keeps it out of the roster:
/// [`super::listening`] dials every dotless entry in a project directory as a socket.
pub const HELD: &str = ".claims";

/// How long the name of a piece of work may be, in characters. It becomes a filename.
pub const NAMED_AT_MOST: usize = 120;

/// One session's word that it is working on something.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    pub by: String,
    /// When it was taken, in milliseconds since the epoch.
    pub at: u64,
    /// What it is about, as whoever took it wrote it. Kept in the file as well as flattened into
    /// its name, because flattening is lossy: `src/a.rs` and `src-a.rs` name one file.
    pub about: String,
}

impl Claim {
    /// As it goes in the file.
    fn written(&self) -> String {
        format!("{}\n{}\n{}", self.by, self.at, self.about)
    }

    /// Read one back, or `None` for a file that is empty or half-written.
    fn read(said: &str) -> Option<Self> {
        let mut lines = said.splitn(3, '\n');
        let by = lines.next()?.trim().to_owned();
        if by.is_empty() {
            return None;
        }
        Some(Self {
            by,
            at: lines
                .next()
                .and_then(|said| said.trim().parse().ok())
                .unwrap_or_default(),
            about: lines.next().unwrap_or_default().trim().to_owned(),
        })
    }

    /// How long ago it was taken, in seconds, for saying so in a refusal.
    #[must_use]
    pub fn ago(&self) -> u64 {
        crate::wire::now_ms().saturating_sub(self.at) / 1_000
    }
}

/// Where a project's claims live.
#[must_use]
pub fn held_in(project: &str) -> PathBuf {
    home(project).join(HELD)
}

/// Where one claim lives.
fn at(project: &str, about: &str) -> PathBuf {
    held_in(project).join(safe(about))
}

/// What is wrong with a name for a piece of work, if anything.
pub fn named(about: &str) -> Result<(), String> {
    let said = about.trim();
    if said.is_empty() {
        return Err("a claim needs `about` — what piece of work is being taken".to_owned());
    }
    if said.chars().count() > NAMED_AT_MOST {
        return Err(format!(
            "that names {} characters of work and a claim may name {NAMED_AT_MOST}. It is a name \
             others match against, not a description of what is being done.",
            said.chars().count()
        ));
    }
    Ok(())
}

/// Take a piece of work in `by`'s name, or say who holds it.
pub fn take(project: &str, by: &str, about: &str) -> Result<Claim, String> {
    named(about)?;
    let mine = Claim {
        by: by.to_owned(),
        at: crate::wire::now_ms(),
        about: about.trim().to_owned(),
    };
    let path = at(project, about);
    if let Err(why) = std::fs::create_dir_all(held_in(project)) {
        return Err(format!("claims cannot be recorded here: {why}"));
    }
    // Two passes: the second is for the file that was there and whose holder had gone.
    for pass in 0..2 {
        match named_at(project, &mine, &path) {
            Ok(true) => return Ok(mine),
            Ok(false) => {}
            Err(why) => return Err(format!("`{about}` could not be claimed: {why}")),
        }
        let Some(held) = read_at(&path) else {
            // Half a file, or none by now. Neither is a claim, and leaving it refuses this piece
            // of work to everybody for as long as the project lives.
            let _ = std::fs::remove_file(&path);
            continue;
        };
        if held.by == by {
            return Ok(held);
        }
        if pass > 0 || answers(&socket(project, &held.by)) {
            return Err(format!(
                "`{}` claimed `{}` {}s ago and is still answering. Ask it, or take something \
                 else — `claims` says what everybody is on.",
                held.by,
                held.about,
                held.ago()
            ));
        }
        let _ = std::fs::remove_file(&path);
    }
    Err(format!(
        "`{about}` is held by somebody this session cannot outrun"
    ))
}

/// Put the whole record in place under one name, or say somebody else got there first.
///
/// Written first, named second: `O_EXCL` on the final name makes the file exist before it has
/// anything in it, and the loser of the race then reads an empty file, takes it for what a crash
/// left behind, and unlinks the winner's claim. The scratch file carries a leading dot so [`all`]
/// does not read a half-made claim as a claim.
fn named_at(project: &str, mine: &Claim, path: &Path) -> std::io::Result<bool> {
    let scratch = held_in(project).join(format!(
        "{MAKING_AT}{}-{}",
        std::process::id(),
        MAKING.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::write(&scratch, mine.written())?;
    let won = match std::fs::hard_link(&scratch, path) {
        Ok(()) => Ok(true),
        Err(why) if why.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(why) => Err(why),
    };
    let _ = std::fs::remove_file(&scratch);
    won
}

/// One counter per process, so two threads claiming at once do not share a scratch file.
static MAKING: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// What a half-made claim is called, before the name that counts. The pid in it is what makes
/// [`abandoned`] safe.
const MAKING_AT: &str = ".making-";

/// Let a piece of work go, in `by`'s name. Refused unless `by` is the session holding it.
pub fn let_go(project: &str, by: &str, about: &str) -> Result<Claim, String> {
    named(about)?;
    let path = at(project, about);
    let Some(held) = read_at(&path) else {
        return Err(format!(
            "nothing holds `{about}`, so there is nothing to let go. `claims` says what is held."
        ));
    };
    if held.by != by {
        return Err(format!(
            "`{}` holds `{}`, not this session. A claim is let go by whoever took it.",
            held.by, held.about
        ));
    }
    let _ = std::fs::remove_file(&path);
    Ok(held)
}

/// Everything held in `project`, with what a dead holder left swept on the way past.
#[must_use]
pub fn all(project: &str) -> Vec<Claim> {
    let Ok(entries) = std::fs::read_dir(held_in(project)) else {
        return Vec::new();
    };
    let mut out: Vec<Claim> = entries
        .flatten()
        // A leading dot is a claim halfway through being made — see `named_at`.
        .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
        .filter_map(|entry| {
            let path = entry.path();
            let held = read_at(&path)?;
            if answers(&socket(project, &held.by)) {
                return Some(held);
            }
            let _ = std::fs::remove_file(&path);
            None
        })
        .collect();
    out.sort_by(|one, other| one.at.cmp(&other.at).then(one.about.cmp(&other.about)));
    out
}

/// Drop everything `id` was holding, for a corpse being swept by somebody else.
pub fn forget_in(project: &str, id: &str) {
    let Ok(entries) = std::fs::read_dir(held_in(project)) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if read_at(&path).is_some_and(|held| held.by == id) || abandoned(&path) {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Whether `path` is a half-made claim whose process is gone. An empty scratch file parses as no
/// claim, so it is held by no id, the sweep above passes over it, and `remove_dir` then refuses
/// the claims directory for good. The pid in the name is what makes this safe to unlink.
fn abandoned(path: &Path) -> bool {
    let Some(pid) = path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix(MAKING_AT))
        .and_then(|rest| rest.split('-').next())
        .and_then(|pid| pid.parse::<i32>().ok())
        .and_then(rustix::process::Pid::from_raw)
    else {
        return false;
    };
    // `ESRCH` and nothing else: a process that exists but may not be signalled answers `EPERM`.
    rustix::process::test_kill_process(pid) == Err(rustix::io::Errno::SRCH)
}

/// Take the directory down if nothing is left in it, so the last session out can fold the
/// project's own directory away.
pub fn leave(project: &str) {
    let _ = std::fs::remove_dir(held_in(project));
}

/// One claim off disk, or `None` for a file that is not one.
fn read_at(path: &Path) -> Option<Claim> {
    Claim::read(&std::fs::read_to_string(path).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Identity;
    use crate::scratch::Project;

    /// A project of its own, removed by a guard so a failing test does not leave it behind.
    fn alone(name: &str) -> Project {
        Project::new("melchior-claim", name)
    }

    fn id(project: &str, id: &str) -> Identity {
        Identity {
            project: project.to_owned(),
            role: "main".to_owned(),
            id: id.to_owned(),
        }
    }

    /// Bind a socket, so the holder is one that answers.
    fn present(project: &str, id: &str) -> std::os::unix::net::UnixListener {
        std::os::unix::net::UnixListener::bind(super::super::listening_at(&self::id(project, id)))
            .expect("bind")
    }

    #[test]
    fn two_sessions_asking_at_once_produce_one_holder() {
        // Both threads ask for the same work; the kernel picks.
        let project = alone("race");
        let _bound = (present(&project, "alpha-rho"), present(&project, "beta-nu"));

        // Many rounds off one barrier, because the interleaving that matters is a handful of
        // instructions wide.
        for round in 0..256 {
            let work = format!("the parser {round}");
            let together = std::sync::Barrier::new(2);
            let held: Vec<Result<Claim, String>> = std::thread::scope(|scope| {
                let one = scope.spawn(|| {
                    together.wait();
                    take(&project, "alpha-rho", &work)
                });
                let other = scope.spawn(|| {
                    together.wait();
                    take(&project, "beta-nu", &work)
                });
                vec![one.join().expect("one"), other.join().expect("other")]
            });

            let won: Vec<&Claim> = held.iter().filter_map(|out| out.as_ref().ok()).collect();
            assert_eq!(
                won.len(),
                1,
                "round {round}: both took it, or neither did: {held:?}"
            );
            let lost = held
                .iter()
                .find_map(|out| out.as_ref().err())
                .expect("somebody lost");
            assert!(lost.contains(&won[0].by), "{lost}");
        }
        assert_eq!(all(&project).len(), 256, "one round left two files behind");
    }

    #[test]
    fn a_live_holder_keeps_its_claim_and_a_dead_one_does_not() {
        let project = alone("dead");
        let live = present(&project, "alpha-rho");
        let _heir = present(&project, "beta-nu");
        take(&project, "alpha-rho", "the parser").expect("taken");
        assert!(
            take(&project, "beta-nu", "the parser").is_err(),
            "a live holder's claim was taken from under it"
        );

        drop(live);
        let _ = std::fs::remove_file(super::super::listening_at(&id(&project, "alpha-rho")));
        let taken = take(&project, "beta-nu", "the parser").expect("a dead holder holds nothing");
        assert_eq!(taken.by, "beta-nu");
        assert_eq!(
            all(&project).len(),
            1,
            "the new holder's claim was swept too"
        );
    }

    #[test]
    fn the_claims_directory_is_never_offered_as_an_agent() {
        // `listening` keeps every entry with no dot in its name and then dials it, so a directory
        // called `claims` would be listed as an agent and handed to `forget_id` on the failed dial.
        let project = alone("hidden");
        let _bound = present(&project, "alpha-rho");
        std::fs::write(
            super::super::sessions::session_at(&id(&project, "alpha-rho")),
            "alpha-rho",
        )
        .expect("the run");
        take(&project, "alpha-rho", "the parser").expect("taken");

        // Asserted on the name: a directory fails the dial anyway, so the end-to-end assertions
        // below stay green while the trap is open.
        assert!(
            HELD.contains('.'),
            "`{HELD}` has no dot in it, so `listening` would keep it and hand it to the sweep"
        );
        let listed = super::super::listening(&project);
        assert!(
            !listed.iter().any(|name| name.contains("claim")),
            "the claims directory is in the roster: {listed:?}"
        );
        let crew: Vec<String> =
            super::super::sessions::crew(&super::super::whom(&project, "alpha-rho"))
                .into_iter()
                .map(|them| them.id)
                .collect();
        assert_eq!(crew, vec!["alpha-rho".to_owned()], "{crew:?}");
        assert!(
            held_in(&project).is_dir(),
            "and the sweep deleted it on the way past"
        );
    }

    #[test]
    fn a_claim_is_let_go_by_whoever_took_it() {
        let project = alone("release");
        let _bound = present(&project, "alpha-rho");
        take(&project, "alpha-rho", "the parser").expect("taken");
        assert!(
            let_go(&project, "beta-nu", "the parser").is_err(),
            "somebody else decided this session had stopped working"
        );
        assert!(let_go(&project, "alpha-rho", "the parser").is_ok());
        assert!(
            let_go(&project, "alpha-rho", "the parser").is_err(),
            "letting go of nothing said it worked"
        );
    }

    #[test]
    fn taking_the_same_work_twice_is_not_a_refusal() {
        let project = alone("again");
        let _bound = present(&project, "alpha-rho");
        let first = take(&project, "alpha-rho", "the parser").expect("taken");
        let again = take(&project, "alpha-rho", "the parser").expect("still ours");
        assert_eq!(first.about, again.about);
        assert_eq!(all(&project).len(), 1);
    }

    #[test]
    fn the_last_session_out_can_still_fold_the_project_directory_away() {
        // `remove_dir` refuses a directory that still holds something, and an empty claims
        // directory left inside is something.
        let project = alone("empty");
        let live = present(&project, "alpha-rho");
        take(&project, "alpha-rho", "the parser").expect("taken");
        let_go(&project, "alpha-rho", "the parser").expect("let go");
        drop(live);
        let _ = std::fs::remove_file(super::super::listening_at(&id(&project, "alpha-rho")));

        super::super::leave(&project);
        assert!(
            !home(&project).exists(),
            "the project directory outlived every session in it"
        );
    }

    #[test]
    fn a_half_made_claim_from_a_dead_process_does_not_keep_the_project_standing() {
        // The one-syscall window in `named_at`: the scratch file exists and nothing is in it yet.
        let project = alone("halfmade");
        std::fs::create_dir_all(held_in(&project)).expect("the claims directory");
        // Above `pid_max`, which is at most 2^22, so this names no process while the test runs.
        let half = held_in(&project).join(format!("{MAKING_AT}{}-0", i32::MAX));
        std::fs::write(&half, "").expect("the scratch file");

        super::super::forget(&id(&project, "alpha-rho"));
        super::super::leave(&project);
        assert!(
            !home(&project).exists(),
            "an empty scratch file outlived the process that made it and kept the project"
        );
    }

    #[test]
    fn a_half_made_claim_from_a_live_process_is_left_where_it_is() {
        // A claimant between the write and the link is still running, and unlinking its scratch
        // turns a race it would have won into `ENOENT`.
        let project = alone("halfmaking");
        std::fs::create_dir_all(held_in(&project)).expect("the claims directory");
        let live = held_in(&project).join(format!("{MAKING_AT}{}-0", std::process::id()));
        std::fs::write(&live, "").expect("the scratch file");

        super::super::forget(&id(&project, "alpha-rho"));
        assert!(live.exists(), "a live claimant's scratch file was swept");
    }

    #[test]
    fn a_claim_somebody_still_holds_keeps_the_directory_standing() {
        // A run somebody is still working in must not be swept by whoever leaves first.
        let project = alone("busy");
        let _live = present(&project, "alpha-rho");
        take(&project, "alpha-rho", "the parser").expect("taken");
        super::super::leave(&project);
        assert!(held_in(&project).is_dir(), "a live claim was deleted");
    }

    #[test]
    fn a_name_that_would_not_fit_in_a_filename_is_refused_by_name() {
        assert!(named("").is_err());
        let long = "z".repeat(NAMED_AT_MOST + 1);
        let why = named(&long).expect_err("an uncapped name went through");
        assert!(why.contains(&NAMED_AT_MOST.to_string()), "{why}");
    }

    #[test]
    fn a_name_cannot_climb_out_of_the_claims_directory() {
        // `about` is caller-supplied text and it becomes a path segment.
        let path = at("magi", "../../etc/passwd");
        assert!(!path.to_string_lossy().contains(".."), "{path:?}");
        assert_eq!(path.parent(), Some(held_in("magi").as_path()));
    }

    #[test]
    fn a_claim_reads_back_as_what_was_written() {
        // The name in the file, not the one in the path: `safe` is lossy.
        let claim = Claim {
            by: "alpha-rho".to_owned(),
            at: 1_700_000_000_000,
            about: "src/parser.rs".to_owned(),
        };
        assert_eq!(Claim::read(&claim.written()).as_ref(), Some(&claim));
        assert_eq!(Claim::read(""), None);
    }
}
