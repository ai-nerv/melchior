//! Nothing melchior runs is a service. Every command a magi starts belongs to that magi.
//!
//! Two of them can be running when the magi is killed, and they are the two that hold something:
//! `serve` holds a socket and a name in the directory, `ask` holds a connection to a provider
//! mid-turn. The rest — `tool`, `brief`, `fork`, `models`, `verbs`, `acknowledge` — read
//! something, answer, and are gone; the three that reach a sibling do it through
//! [`asking`](melchior::asking), whose ten-second read timeout is already a bound on how long
//! one of them can be left behind. An orphan that ends itself in ten seconds is not the thing
//! this is for.

/// Ask the kernel to end this process when the magi that started it ends.
///
/// The pipe is the ordinary way out and this is the floor under it. A magi that goes drops its
/// end of stdin, the reader sees the close, and the command stops — but that holds only while
/// stdin is a live pipe held by that magi and by nobody else. `PR_SET_PDEATHSIG` needs neither
/// condition: the kernel sends the signal, so a `kill -9`, an OOM, or a panic that runs no
/// destructor is covered exactly as well as a clean exit is.
///
/// **`ask` has neither condition for most of its life.** It reads stdin to end of file before it
/// says a word to a provider, so from the moment the turn actually starts there is no pipe left
/// for anybody to close, and the streaming call that follows has no overall timeout on purpose —
/// a long turn is a long turn. A magi killed mid-turn left a melchior holding an open connection
/// and answering to nothing.
///
/// `SIGTERM` rather than `SIGKILL`, so a `serve` can still unlink its socket and the note beside
/// it. A session left in the directory answers nothing and is found by a sibling that has
/// already sent to it, which is worse than a name that is simply gone.
///
/// The signal watches only from the moment it is set, so a magi that died a moment before is a
/// death nothing was ever sent for. Reading who the parent is on either side of the call closes
/// what can be closed from in here: a different answer means the reparenting has already
/// happened. What it does not close is the window before the first of those reads, and that
/// would take magi naming its own pid on the command line the way balthasar's `--tied` does. It
/// does not have to: in exactly that case stdin is already at end of file, and both callers stop
/// on their first read of it — `serve` because its loop is over, `ask` because an empty body is
/// not an ask. Comparing against pid 1 is the tempting version and is wrong — a magi that is
/// itself pid 1 in a container would spawn a melchior that exits before it does anything.
///
/// **The "parent" the kernel watches is a thread, not a process.** `PR_SET_PDEATHSIG` fires when
/// the thread that created this process exits, which is the standing footgun: a program that
/// spawns from a short-lived worker gets the signal while it is still alive. It is not one here.
/// Both callers reach this from `fn main`, before any runtime exists, and neither runtime melchior
/// builds is multi-threaded — `rt-multi-thread` is not in the feature graph at all — so the thread
/// this is set from is the one that lives as long as the process. What melchior cannot rule out is
/// the other end: a magi that spawned it from a thread of its own would send this signal by that
/// thread exiting, and there is nothing on this side that could tell that apart from a person
/// typing `kill`.
///
/// **A person at a terminal gets the same thing and should.** Run by hand the parent is the
/// shell that is already waiting for it, so this changes nothing about a foreground job; what it
/// rules out is an `ask` deliberately detached to outlive the shell that started it, which is the
/// thing the family's rule says may not exist.
pub fn to_magi() -> std::io::Result<()> {
    let magi = rustix::process::getppid();
    rustix::process::set_parent_process_death_signal(Some(rustix::process::Signal::Term))?;
    if rustix::process::getppid() != magi {
        std::process::exit(0);
    }
    Ok(())
}
