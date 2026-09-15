//! Ends a serving session on `SIGTERM` or `SIGINT`, waited for rather than handled: a handler may
//! neither allocate nor take a lock, so it cannot unlink this session's notes, and blocking the
//! signal to `sigwait` on it leaves a mask that everything melchior `exec`s inherits.

use tokio::signal::unix::{Signal, SignalKind};

pub struct Ending {
    term: Signal,
    interrupt: Signal,
}

impl Ending {
    pub fn watching() -> std::io::Result<Self> {
        Ok(Self {
            term: tokio::signal::unix::signal(SignalKind::terminate())?,
            interrupt: tokio::signal::unix::signal(SignalKind::interrupt())?,
        })
    }

    pub async fn came(&mut self) {
        tokio::select! {
            _ = self.term.recv() => {}
            _ = self.interrupt.recv() => {}
        }
    }
}
