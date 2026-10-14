//! A parent killed outright takes its `melchior serve` with it, and leaves nothing behind.

mod fixture;

use fixture::left_in;
use melchior::scratch::Scratch;
use std::process::{Child, Command, Stdio};

/// A parent that starts one `melchior serve` and then does nothing at all.
///
/// A shell rather than this process: the parent has to be something the test can kill outright,
/// and killing the test runner is not available. Its stdin is a pipe *this* process holds the
/// other end of, handed down explicitly — a background job in a non-interactive shell is given
/// `/dev/null` otherwise, and stdin staying open is the whole point of the exercise.
struct Killable {
    shell: Child,
    /// The write end of the pipe `serve` is reading, kept open on purpose. While this is held,
    /// end of file cannot be what stops it, so the kernel is the only explanation left.
    held: Option<std::process::ChildStdin>,
    served: u32,
    /// Kept for its `Drop`, which removes the directory however the test ends.
    _runtime: Scratch,
}

impl Killable {
    fn start(name: &str) -> Self {
        // Under `$TMPDIR`, for the same reason `Serving::start` is: this named
        // `/tmp/melchior-k-<pid>-<name>` outright, which escapes the isolated root
        // `gate-hermetic` runs the suite under and so leaked past it unremarked.
        let runtime = Scratch::new("melchior-k", name);
        let pids = runtime.join("pid");
        // `--ui`, so there is a fourth file to leave behind. Without it a session writes its
        // socket, its `.role` and its `.session`, and the case the field reported — four files
        // per session, the screen note among them — is one the fixture could not reproduce.
        let script = format!(
            "exec 3<&0; XDG_RUNTIME_DIR={runtime} {binary} serve --project killed \
             --ui {runtime}/screen.sock <&3 >/dev/null 2>&1 & echo $! > {pids}; wait",
            runtime = runtime.display(),
            binary = env!("CARGO_BIN_EXE_melchior"),
            pids = pids.display(),
        );
        let mut shell = Command::new("sh")
            .arg("-c")
            .arg(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start the caller");
        // Taken out before anything waits on the shell: `Child::wait` drops its own stdin, and
        // dropping this one would close the pipe and end `serve` for the ordinary reason.
        let held = shell.stdin.take();
        let served = read_pid(&pids).expect("the caller said which melchior it started");
        let it = Self {
            shell,
            held,
            served,
            _runtime: runtime,
        };
        let home = it.home();
        if !settled_in(&home, std::time::Duration::from_secs(10)) {
            let left = left_in(&home);
            it.cleared();
            panic!("the session never came up: {left:?}");
        }
        it
    }

    /// Where this session's socket and the notes beside it are.
    fn home(&self) -> std::path::PathBuf {
        self._runtime.join("melchior").join("killed")
    }

    /// End the caller the way a crash would: with nothing running inside it.
    fn killed(&mut self) {
        let _ = self.shell.kill();
        let _ = self.shell.wait();
    }

    /// Leave nothing running, whatever the assertions are about to do.
    ///
    /// The directory is not this function's job any more: it goes when `self` does.
    fn cleared(mut self) {
        let _ = self.shell.kill();
        let _ = self.shell.wait();
        drop(self.held.take());
        let _ = Command::new("kill")
            .arg("-9")
            .arg(self.served.to_string())
            // Already gone is the passing case, and its complaint reads like a failure.
            .stderr(Stdio::null())
            .status();
    }
}

/// Wait for the session the shell started to come up, and say whether it did.
///
/// The pid is written the moment the shell forks, and `serve` writes its notes and binds its
/// socket well after — so listing the directory on the next line found it empty on roughly one
/// loaded run in three, and a kill in that window landed before the child had asked the kernel
/// to end it with its parent. Polled: the `listening` line belongs to the shell.
fn settled_in(project: &std::path::Path, patience: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + patience;
    loop {
        // The notes go down before the socket binds, so the socket says it is all there.
        let names = left_in(project);
        if names.iter().any(|it| !it.contains('.')) && names.iter().any(|it| it.ends_with(".ui")) {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// The pid the shell wrote down, once it has written it.
fn read_pid(at: &std::path::Path) -> Option<u32> {
    for _ in 0..250 {
        if let Ok(text) = std::fs::read_to_string(at)
            && let Ok(pid) = text.trim().parse::<u32>()
        {
            return Some(pid);
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    None
}

/// Whether a process exists and is not merely a corpse waiting to be reaped.
///
/// The state field rather than the directory's existence: every process here is started by a
/// shell that is about to be killed, so a zombie is the expected shape of "gone".
fn alive(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .is_ok_and(|stat| stat.split_whitespace().nth(2) != Some("Z"))
}

/// Wait for `pid` to go away, and say whether it did.
fn gone_within(pid: u32, patience: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + patience;
    while std::time::Instant::now() < deadline {
        if !alive(pid) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    !alive(pid)
}

#[test]
fn nothing_outlives_a_parent_that_was_killed_outright() {
    // The case `nothing_outlives_the_parent` cannot reach. That one lets go of the pipe, which
    // is a parent with a way out; this one is the panic, the `kill -9` and the OOM, where
    // nothing in the parent runs at all and the pipe is never closed by anybody. The write end
    // is still held here while the assertion runs, so end of file is not available as an
    // explanation and `PR_SET_PDEATHSIG` is the only thing left that could have done it.
    let mut caller = Killable::start("killed");
    let served = caller.served;
    assert!(
        alive(served),
        "it is up while the parent that started it is"
    );

    caller.killed();
    let went = gone_within(served, std::time::Duration::from_secs(10));

    caller.cleared();
    assert!(
        went,
        "melchior serve must not outlive the process that started it"
    );
}

#[test]
fn a_killed_parent_leaves_none_of_its_sessions_notes_behind() {
    // The case the field reported, and the reason the exit path being correct was not enough:
    // a headless magi taken down with `kill -9`, its melchior ended by the kernel's `SIGTERM`,
    // and four files still in the directory afterwards — the socket, and the `.ui`, `.role` and
    // `.session` notes beside it. Every sibling that listed the project was then offered a name
    // that answers nothing until something else came along and swept it.
    let mut caller = Killable::start("swept");
    let home = caller.home();
    let bound = left_in(&home);
    assert!(
        bound.iter().any(|name| name.ends_with(".ui")),
        "the fixture is not the reported case: {bound:?}"
    );

    caller.killed();
    let went = gone_within(caller.served, std::time::Duration::from_secs(10));
    // Read after it is gone rather than on a timer: the unlinking is the last thing it does, so
    // the process being gone is what makes this listing an answer rather than a race.
    let left = left_in(&home);

    caller.cleared();
    assert!(went, "melchior serve outlived the parent that started it");
    assert!(
        left.is_empty(),
        "the kernel's signal ended it and these stayed: {left:?}"
    );
}
