//! What the kernel says about a process, and how a connecting one stands to the session that
//! started this `serve`. A pid alone can be reused, so every process is named by its pid and the
//! start time `/proc/<pid>/stat` gives it, read when the connection is accepted.

use std::os::unix::fs::MetadataExt;

/// One process, as `/proc/<pid>/stat` describes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Proc {
    pub pid: i32,
    pub ppid: i32,
    /// Clock ticks after boot; with the pid, what makes this process this one.
    pub start: u64,
}

/// A process's pid, parent and start time, or `None` when it is gone.
#[must_use]
pub fn stat(pid: i32) -> Option<Proc> {
    let line = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    read_stat(pid, &line)
}

/// The same, from the text of a `stat` file. The command name may hold spaces and parentheses, so
/// the fields are counted from the last `)`.
#[must_use]
pub fn read_stat(pid: i32, line: &str) -> Option<Proc> {
    let rest = &line[line.rfind(')')? + 1..];
    let fields: Vec<&str> = rest.split_whitespace().collect();
    Some(Proc {
        pid,
        ppid: fields.get(1)?.parse().ok()?,
        start: fields.get(19)?.parse().ok()?,
    })
}

/// Which file a process is running, by device and inode, so two names for one binary agree.
#[must_use]
pub fn exe(pid: i32) -> Option<(u64, u64)> {
    let meta = std::fs::metadata(format!("/proc/{pid}/exe")).ok()?;
    Some((meta.dev(), meta.ino()))
}

/// One variable out of a process's environment as it was at `exec`.
#[must_use]
pub fn environ(pid: i32) -> Option<Vec<u8>> {
    std::fs::read(format!("/proc/{pid}/environ")).ok()
}

/// How a connecting process stands to the session that started this one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kinship {
    /// The session's own process, or one it started directly: what speaks for it.
    Session,
    /// Started by a copy of the session's own program that the session started: `magi fork`
    /// running `melchior fork`, which may name a child and nothing more.
    Spawner,
    /// Anything else, which the kernel says nothing for.
    Stranger,
}

/// The process that started this one, read once at startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Origin {
    pub parent: Proc,
}

impl Origin {
    /// Who started this process, now.
    #[must_use]
    pub fn here() -> Option<Self> {
        let parent = rustix::process::getppid()?.as_raw_nonzero().get();
        Some(Self {
            parent: stat(parent)?,
        })
    }

    /// How `peer` stands to the session that started this one, from what the kernel says now.
    #[must_use]
    pub fn kinship(&self, peer: &Proc) -> Kinship {
        self.kinship_with(peer, stat, exe)
    }

    /// The same, with the kernel handed in.
    pub fn kinship_with(
        &self,
        peer: &Proc,
        stat: impl Fn(i32) -> Option<Proc>,
        exe: impl Fn(i32) -> Option<(u64, u64)>,
    ) -> Kinship {
        let parent = self.parent;
        if stat(parent.pid).is_none_or(|now| now.start != parent.start) {
            return Kinship::Stranger;
        }
        if (peer.pid == parent.pid && peer.start == parent.start) || peer.ppid == parent.pid {
            return Kinship::Session;
        }
        let Some(between) = stat(peer.ppid) else {
            return Kinship::Stranger;
        };
        let same = exe(between.pid).is_some_and(|it| exe(parent.pid) == Some(it));
        if between.ppid == parent.pid && same {
            return Kinship::Spawner;
        }
        Kinship::Stranger
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc(pid: i32, ppid: i32, start: u64) -> Proc {
        Proc { pid, ppid, start }
    }

    /// A kernel of three processes: the session (10), what it started (20), and a stranger (30).
    fn kernel(pid: i32) -> Option<Proc> {
        match pid {
            10 => Some(proc(10, 1, 500)),
            20 => Some(proc(20, 10, 600)),
            21 => Some(proc(21, 10, 610)),
            30 => Some(proc(30, 1, 700)),
            _ => None,
        }
    }

    fn binary(pid: i32) -> Option<(u64, u64)> {
        match pid {
            10 | 20 => Some((1, 42)),
            21 => Some((1, 7)),
            _ => Some((1, 99)),
        }
    }

    fn origin() -> Origin {
        Origin {
            parent: proc(10, 1, 500),
        }
    }

    #[test]
    fn a_stat_line_is_read_from_the_last_parenthesis() {
        let line = "4242 (a (weird) name) S 17 4242 4242 0 -1 4194560 1 0 0 0 0 0 0 0 20 0 1 0 \
                    98765 1000 10";
        assert_eq!(read_stat(4242, line), Some(proc(4242, 17, 98765)));
        assert_eq!(read_stat(1, "garbage"), None);
    }

    #[test]
    fn this_process_reads_back_as_itself() {
        let me = i32::try_from(std::process::id()).expect("a pid");
        let read = stat(me).expect("this process");
        assert_eq!(read.pid, me);
        assert!(read.start > 0);
    }

    #[test]
    fn the_session_and_what_it_started_speak_for_it() {
        let origin = origin();
        let k = |peer: Proc| origin.kinship_with(&peer, kernel, binary);
        assert_eq!(k(proc(10, 1, 500)), Kinship::Session, "the session itself");
        assert_eq!(k(proc(20, 10, 600)), Kinship::Session, "its direct child");
        assert_eq!(k(proc(30, 1, 700)), Kinship::Stranger, "somebody else");
        assert_eq!(k(proc(10, 1, 501)), Kinship::Stranger, "a reused pid");
    }

    #[test]
    fn only_the_session_s_own_program_may_stand_between() {
        let origin = origin();
        let k = |peer: Proc| origin.kinship_with(&peer, kernel, binary);
        assert_eq!(k(proc(40, 20, 800)), Kinship::Spawner, "magi fork's child");
        assert_eq!(k(proc(41, 21, 810)), Kinship::Stranger, "a shell's child");
        assert_eq!(
            k(proc(42, 30, 820)),
            Kinship::Stranger,
            "a stranger's child"
        );
    }

    #[test]
    fn a_session_that_is_gone_or_was_replaced_vouches_for_nobody() {
        let gone = Origin {
            parent: proc(11, 1, 500),
        };
        assert_eq!(
            gone.kinship_with(&proc(20, 11, 600), kernel, binary),
            Kinship::Stranger
        );
        let replaced = Origin {
            parent: proc(10, 1, 499),
        };
        assert_eq!(
            replaced.kinship_with(&proc(20, 10, 600), kernel, binary),
            Kinship::Stranger
        );
    }
}
