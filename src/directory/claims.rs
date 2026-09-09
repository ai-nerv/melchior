//! What a session has said it is working on, so two of them do not do it twice.
//!
//! Split from [`super`] under THE RULE, which caps a file at 800 lines.
//!
//! # A file, and the kernel decides who won
//!
//! One file per claim, given its name by a single exclusive syscall. That is the whole mechanism:
//! two agents that ask for the same piece of work in the same microsecond both try, the kernel
//! lets exactly one of them name the file, and the other reads back who beat it. No lock manager,
//! no protocol, no round trip — and no window between deciding and recording.
//!
//! This is the one thing being on one machine *buys* rather than merely permits. A distributed
//! version of this needs consensus; here it needs [`std::fs::hard_link`].
//!
//! # A dead claimant holds nothing
//!
//! A claim is refused only if the file is there **and** its holder still answers its socket —
//! the same liveness test [`super::listening`] applies to a socket, for the same reason. A
//! process that died did not get to let go of its work, and a claim nobody can be asked about is
//! a piece of work nobody will ever do again.
//!
//! # The directory's name starts with a dot, and that is load-bearing
//!
//! [`super::listening`] lists a project's directory, keeps every entry whose name holds no dot,
//! and then *dials it as a socket*. A directory called `claims` would be listed as an agent,
//! offered to a model as one, dial-tested on every sweep, and — failing that dial — handed to the
//! sweep that deletes a corpse's socket and notes. The dot is what keeps this out of the roster,
//! and there is a test that says so.

use super::{answers, home, safe, socket};
use std::path::{Path, PathBuf};

/// What the directory of claims is called.
///
/// Read by the tests that check it never reaches a roster, so the dot is one character in one
/// place rather than a convention somebody has to keep.
pub const HELD: &str = ".claims";

/// How long the name of a piece of work may be, in characters.
///
/// It becomes a filename, so an uncapped one is `ENAMETOOLONG` reported to a model as "the claim
/// could not be written", which says nothing it can act on. A hundred and twenty is a path, a
/// ticket number or a sentence naming a task — more than that is a description of the work rather
/// than a name for it, and two agents will never write the same one.
pub const NAMED_AT_MOST: usize = 120;

/// One session's word that it is working on something.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    /// The id of the session holding it.
    pub by: String,
    /// When it was taken, in milliseconds since the epoch.
    pub at: u64,
    /// What it is about, as whoever took it wrote it.
    ///
    /// Kept in the file as well as flattened into its name, because flattening is lossy:
    /// `src/a.rs` and `src-a.rs` name one file. A reader that had only the filename would be
    /// shown a claim nobody made.
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
///
/// # Errors
/// When it says nothing, or says more than a name can.
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

/// Take a piece of work in `by`'s name.
///
/// # Errors
/// When somebody else holds it and still answers, or when the file cannot be written at all.
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
    // Two passes and no more. The second exists for the one case worth retrying — the file was
    // there, its holder was gone, and this call swept it — and a third would be a spin against
    // somebody who keeps winning, which is not a failure a loop fixes.
    for pass in 0..2 {
        match named_at(project, &mine, &path) {
            Ok(true) => return Ok(mine),
            Ok(false) => {}
            Err(why) => return Err(format!("`{about}` could not be claimed: {why}")),
        }
        let Some(held) = read_at(&path) else {
            // Half a file, or none by the time it was read. Neither is a claim, and leaving it
            // would refuse this piece of work to everybody for as long as the project lives.
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
        // Swept the way a stale socket is, at the point somebody needed the answer. The window
        // between reading a dead holder and unlinking its file is a few microseconds wide, and
        // what fits in it is another sweeper taking the same claim first — which the second pass
        // then reports honestly, as a live holder.
        let _ = std::fs::remove_file(&path);
    }
    Err(format!(
        "`{about}` is held by somebody this session cannot outrun"
    ))
}

/// Put the whole record in place under one name, or say somebody else got there first.
///
/// **Written first, named second.** `O_EXCL` on the final name is exclusive too, and it is what
/// this did at first — but it makes the file *exist* before it has anything in it. The loser of
/// the race then reads an empty file, decides it is what a crash left behind, unlinks the winner's
/// claim and takes the work: two claimants, two holders, and only sometimes. `link` is the same
/// one-syscall exclusion with the content already there, so the name appears complete or not at
/// all. Found by running two claimants in two threads, which is the only way this shows.
///
/// The scratch file carries a leading dot for the same reason the directory does — [`all`] reads
/// this directory, and a half-made claim is not one. A process that died between writing it and
/// linking it leaves one behind holding the corpse's name, and the sweep that clears a corpse's
/// claims clears that with them; one that died a syscall earlier leaves an empty file that names
/// nobody, and [`abandoned`] is what clears that.
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

/// What a half-made claim is called, before the name that counts.
///
/// The pid is in it, and that is what makes [`abandoned`] safe.
const MAKING_AT: &str = ".making-";

/// Let a piece of work go, in `by`'s name.
///
/// # Errors
/// When nothing holds it, or somebody else does. A session releasing another's claim would be a
/// session deciding somebody else had stopped working, which is not a fact it has.
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
///
/// Swept where the list is read, for the same reason [`super::listening`] sweeps there: the
/// claims needing it are exactly the ones whose holder never got to run its own exit path.
#[must_use]
pub fn all(project: &str) -> Vec<Claim> {
    let Ok(entries) = std::fs::read_dir(held_in(project)) else {
        return Vec::new();
    };
    let mut out: Vec<Claim> = entries
        .flatten()
        // A leading dot is a claim halfway through being made — see [`named_at`]. `safe` turns
        // every dot into a dash, so no claim of anybody's ever starts with one.
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
///
/// Called beside the socket and the notes, because a claim is one more thing a session leaves
/// behind and the sweep is one place rather than four.
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

/// Whether `path` is a half-made claim whose process is gone.
///
/// [`named_at`] writes the record and then names it, and a process killed between creating that
/// scratch file and writing into it leaves an empty one. Nothing removed it: it parses as no
/// claim, so it is held by no id and the sweep above passed over it — and `remove_dir` then
/// refuses the claims directory for good, which keeps the project's own directory standing after
/// every session in it has gone. One process killed in a one-syscall window, and that project
/// never folds away again.
///
/// The pid in the name is what makes this safe to unlink. A claimant that is still between the
/// two calls is a process that is still running, so its scratch is never taken out from under it;
/// a pid that has come round again only means the file stays, which is where it was.
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
    // `ESRCH` and nothing else. A process that exists and may not be signalled answers `EPERM`,
    // and reading every error as death would unlink the scratch of somebody still using it.
    rustix::process::test_kill_process(pid) == Err(rustix::io::Errno::SRCH)
}

/// Take the directory down if nothing is left in it.
///
/// `remove_dir` refuses a directory that still holds something, which is the whole test —
/// no listing, and no race against somebody taking a claim as this one leaves. Without it the
/// last session out cannot fold the project directory away, because this one is inside it.
pub fn leave(project: &str) {
    let _ = std::fs::remove_dir(held_in(project));
}

/// One claim off disk, or `None` for a file that is not one.
fn read_at(path: &Path) -> Option<Claim> {
    Claim::read(&std::fs::read_to_string(path).ok()?)
}

/// The kernel decides who won, a dead holder holds nothing, and none of this is a session.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Identity;
    use crate::scratch::Project;

    /// A project of its own, so these do not read each other's directory.
    ///
    /// A guard rather than a name: the line that removed it came after the assertions, so a
    /// failing test left it behind for good — see [`crate::scratch`].
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
        // The whole mechanism, and the only part of it that is not this file's code: `O_EXCL` is
        // one syscall, so there is no window between deciding and recording for a second caller
        // to slip through. Both threads ask for the same work; the kernel picks.
        let project = alone("race");
        let _bound = (present(&project, "alpha-rho"), present(&project, "beta-nu"));

        // Many rounds, and both threads let go of a barrier together, because the interleaving
        // that matters is a handful of instructions wide. One round is a coin toss: the version
        // of this that gave the file its name before it wrote the record into it passed its first
        // run and failed its third, and what failed was the loser reading an empty file, calling
        // it a corpse, and unlinking the winner's claim on its way in.
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
            // And the loser was told who beat it, which is the only thing it can act on.
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
        // The liveness test, which is the same one a socket gets. A process that died did not get
        // to let go of its work, and a claim nobody can be asked about is work nobody does again.
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
        // The trap. `listening` keeps every entry with no dot in its name and then dials it: a
        // directory called `claims` would be listed as an agent, shown to a model as one, and
        // handed to `forget_id` when the dial failed. The dot is what keeps it out.
        let project = alone("hidden");
        let _bound = present(&project, "alpha-rho");
        std::fs::write(
            super::super::sessions::session_at(&id(&project, "alpha-rho")),
            "alpha-rho",
        )
        .expect("the run");
        take(&project, "alpha-rho", "the parser").expect("taken");

        // **Asserted on the name, because that is where breaking it shows.** `listening` keeps
        // every entry with no dot in it and only *then* dials — and a directory fails that dial,
        // so a claims directory called `claims` drops out of the roster anyway and the end-to-end
        // assertions below stay green while the trap is wide open. What actually goes wrong is
        // everything between: it is dial-tested on every sweep, and it is handed to the sweep
        // that deletes an agent's socket and notes.
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
        // A model that has forgotten it already asked must not be told a stranger holds its own
        // work: the answer it can act on is "you have it", and the answer it cannot is "`you`
        // claimed this".
        let project = alone("again");
        let _bound = present(&project, "alpha-rho");
        let first = take(&project, "alpha-rho", "the parser").expect("taken");
        let again = take(&project, "alpha-rho", "the parser").expect("still ours");
        assert_eq!(first.about, again.about);
        assert_eq!(all(&project).len(), 1);
    }

    #[test]
    fn the_last_session_out_can_still_fold_the_project_directory_away() {
        // `leave` is `remove_dir`, which refuses a directory that still holds something — and an
        // empty claims directory left inside is something. Without folding this one away first,
        // a machine collects a project directory per run for good, which is how the runtime
        // directory filled up the first time.
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
        // The one-syscall window in `named_at`: the scratch file exists and nothing has been
        // written into it yet. It parses as no claim, so it is held by no id, so the sweep
        // passed over it -- and `remove_dir` then refused this directory for good.
        let project = alone("halfmade");
        std::fs::create_dir_all(held_in(&project)).expect("the claims directory");
        // Above `pid_max`, which is at most 2^22, so this names no process now and cannot come to
        // name one while the test runs -- which a real corpse's pid could.
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
        // The other half. A claimant between the write and the link is a process that is still
        // running, and unlinking its scratch turns a race it would have won into `ENOENT`.
        let project = alone("halfmaking");
        std::fs::create_dir_all(held_in(&project)).expect("the claims directory");
        let live = held_in(&project).join(format!("{MAKING_AT}{}-0", std::process::id()));
        std::fs::write(&live, "").expect("the scratch file");

        super::super::forget(&id(&project, "alpha-rho"));
        assert!(live.exists(), "a live claimant's scratch file was swept");
    }

    #[test]
    fn a_claim_somebody_still_holds_keeps_the_directory_standing() {
        // The other half, and the reason this is `remove_dir` rather than a listing: a run
        // somebody is still working in must not have its record swept by whoever leaves first.
        let project = alone("busy");
        let _live = present(&project, "alpha-rho");
        take(&project, "alpha-rho", "the parser").expect("taken");
        super::super::leave(&project);
        assert!(held_in(&project).is_dir(), "a live claim was deleted");
    }

    #[test]
    fn a_name_that_would_not_fit_in_a_filename_is_refused_by_name() {
        // Uncapped this is `ENAMETOOLONG` reaching a model as "could not be written", which says
        // nothing it can act on.
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
        // The name in the file, not the one in the path: `safe` is lossy, and a reader with only
        // the filename would show a claim nobody made.
        let claim = Claim {
            by: "alpha-rho".to_owned(),
            at: 1_700_000_000_000,
            about: "src/parser.rs".to_owned(),
        };
        assert_eq!(Claim::read(&claim.written()).as_ref(), Some(&claim));
        assert_eq!(Claim::read(""), None);
    }
}
