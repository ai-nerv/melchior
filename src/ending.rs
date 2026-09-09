//! What ends a serving session when nothing inside it decided to.
//!
//! `serve` takes its own notes down at the bottom of its loop — the socket, and the `.parent`,
//! `.session`, `.role`, `.ui` and `.sent` beside it — and until now that code was correct and
//! unreachable. The way a melchior actually dies is not the pipe closing. It is `SIGTERM`, sent
//! by the kernel because the magi that started it was killed, and the default disposition for a
//! signal ends the process where it stands: no unwinding, no destructors, nothing below the
//! loop. A headless magi taken down with `kill -9` left four files per session behind it, and
//! every sibling that listed the project afterwards was offered a name that answers nothing.
//!
//! # Waited for, not handled
//!
//! What a handler may do is close to nothing — its body must be async-signal-safe, so it may not
//! allocate and may not take a lock, which rules out unlinking anything from inside it and
//! leaves a flag for the loop to poll. Waiting for the signal instead puts the cleanup back in
//! ordinary safe Rust on the path that already knows what this session wrote, and there is
//! exactly one of those, so a session never half-tidies itself from two threads at once.
//!
//! `sigwait` on a thread of its own is the other way to wait, and it is not available here.
//! `rustix`'s safe surface has no `sigprocmask` — both it and `sigwait` live in its `runtime`
//! module, are `unsafe fn`, and this crate denies `unsafe_code`. A blocked signal is also
//! inherited across `exec` where a handler's disposition is reset, so a mask would follow
//! anything melchior ever starts and leave it deaf to `SIGTERM` in turn. tokio's signal driver
//! is the same wait with the handler written once, upstream and audited: it writes a byte to a
//! pipe, and this is the read of the other end. It costs a feature on a dependency this program
//! already runs its whole loop on, and no new edge in the graph.
//!
//! # A second signal during the cleanup
//!
//! Installing a handler means the default disposition is gone for the life of the process, so a
//! second `SIGTERM` while the notes are being unlinked no longer ends it where it stands. Only
//! `SIGKILL` does. Measured, on the release binary, from `kill -TERM` to exit: 3.0ms with nothing
//! in the project, 6.3ms with five hundred claim files to scan, 13.1ms with five thousand. The
//! path is `read_dir` and `unlink` under `$XDG_RUNTIME_DIR`, which is tmpfs — no peer is dialled,
//! no lock is taken and nothing waits.
//!
//! It can block in one state: `$XDG_RUNTIME_DIR` unset, so the fallback is `$TMPDIR`, and that
//! being a hung network or FUSE mount. A process blocked there is in uninterruptible sleep and is
//! delivered no signal at all, so restoring the second `SIGTERM` would not shorten it either. The
//! escalation that works is the one that already works.

use tokio::signal::unix::{Signal, SignalKind};

/// The signals that mean this session is over.
pub struct Ending {
    /// What the kernel sends when the magi goes, and what anybody shutting the machine down
    /// sends.
    term: Signal,
    /// What ctrl-C sends. A `serve` started by hand ends that way, and a person's own leftovers
    /// are the ones they meet again tomorrow.
    interrupt: Signal,
}

impl Ending {
    /// Start listening for them.
    ///
    /// Called before the socket is bound and the notes are written, so the only death this
    /// cannot tidy up after is one that arrives while there is still nothing to tidy.
    ///
    /// # Errors
    /// When the handler cannot be installed, which is a runtime with no I/O driver under it
    /// rather than anything about this session. Refused rather than served: a `serve` that
    /// could not arrange to clean up after itself is the leak this module exists to close, and
    /// it would come up looking identical to one that can.
    pub fn watching() -> std::io::Result<Self> {
        Ok(Self {
            term: tokio::signal::unix::signal(SignalKind::terminate())?,
            interrupt: tokio::signal::unix::signal(SignalKind::interrupt())?,
        })
    }

    /// Wait until one of them arrives.
    ///
    /// Which one is not reported, because nothing downstream would do anything different with
    /// the answer: both mean this session is over, and both leave the same six files behind if
    /// the loop does not get to the lines under it.
    pub async fn came(&mut self) {
        tokio::select! {
            _ = self.term.recv() => {}
            _ = self.interrupt.recv() => {}
        }
    }
}
