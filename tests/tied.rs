//! Whether the kernel takes a `melchior ask` with the magi that started it.
//!
//! Against the real binary, because there is nothing to test below it: `PR_SET_PDEATHSIG` is a
//! property of a live process and its parent, and a unit test could only assert that a function
//! calls a function.
//!
//! The case that matters is the magi that is *killed*, not the one that exits. A magi with a way
//! out can end its children on the way; the ones that leave a melchior running are the panic, the
//! `kill -9` and the OOM, where nothing in the magi runs at all. And `ask` is at its most exposed
//! in the middle of a turn: the ask arrives on stdin and stdin is read to end of file before the
//! provider is called at all, so from then on there is no pipe left for anybody to close — and
//! the call that follows has no overall timeout over it, because a long turn is a long turn.
//!
//! The provider here is a socket that accepts and then says nothing, which is the shape of the
//! leak rather than an approximation of it: a real melchior in exactly this state is holding a
//! connection open to somebody's API and streaming nothing to nobody.

use melchior::scratch::Scratch;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// How long to wait for a process to notice its parent is gone.
///
/// The signal is immediate; this covers the scheduler getting round to the process and the test
/// getting round to looking. Generous, because a slow machine failing this would report the
/// guarantee as broken when it is only late.
const NOTICES_WITHIN: Duration = Duration::from_secs(10);

/// The binary under test, named once because the magi here is a shell script.
const MELCHIOR: &str = env!("CARGO_BIN_EXE_melchior");

/// A provider that accepts the connection and then never answers.
///
/// The accepted sockets are kept rather than dropped: a closed one is a transport error melchior
/// reports in a fraction of a second, and the whole point is the turn that never ends. Says on
/// the channel when a connection has arrived, so the test can kill the magi at the moment the
/// melchior is actually holding something.
fn hanging(told: std::sync::mpsc::Sender<()>) -> u16 {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("a port");
    let port = listener.local_addr().expect("its address").port();
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for stream in listener.incoming() {
            let Ok(stream) = stream else { return };
            held.push(stream);
            if told.send(()).is_err() {
                return;
            }
        }
    });
    port
}

/// A magi that starts one `melchior ask` against that provider and then does nothing at all.
///
/// A shell rather than this process: the parent has to be something the test can kill outright,
/// and killing the test runner is not available. `$!` is the pid of the melchior rather than of
/// the shell, which is what has to be watched — a shell that dies takes nothing with it by
/// default, and that is the whole exercise.
struct Magi {
    shell: Child,
    asked: u32,
    /// Kept for its `Drop`: the config, the ask and the pid file go with it, including when
    /// `starting` itself gives up half way through.
    _dir: Scratch,
}

impl Magi {
    fn starting(port: u16) -> Self {
        let dir = Scratch::new("melchior-tied", "ask");
        std::fs::create_dir_all(dir.join("cfg")).expect("mkdir");
        // Added to the shipped catalog rather than replacing it: the config directory layers.
        std::fs::write(
            dir.join("cfg/providers.lua"),
            format!(
                r#"melchior.provider("tiedtest", {{
  name = "Tied test",
  api = "openai-completions",
  base_url = "http://127.0.0.1:{port}/v1",
  auth = {{ kind = "none" }},
  models = {{ {{ id = "hangs", name = "Hangs", context_window = 8192, max_tokens = 128 }} }},
}})
"#
            ),
        )
        .expect("wrote the provider");
        let ask = dir.join("ask.json");
        std::fs::write(
            &ask,
            r#"{"model":"tiedtest/hangs","context":{"messages":[{"role":"user","content":[{"type":"text","text":"hi"}]}]}}"#,
        )
        .expect("wrote the ask");

        // A regular file for stdin, so it is at end of file before the provider is called.
        // Nothing is left on the pipe for a dying magi to close, which is the gap this covers.
        let pids = dir.join("pid");
        let script = format!(
            "MELCHIOR_CONFIG={cfg} {MELCHIOR} ask <{ask} >/dev/null 2>&1 & echo $! > {pids}; wait",
            cfg = dir.join("cfg").display(),
            ask = ask.display(),
            pids = pids.display(),
        );
        let shell = Command::new("sh")
            .arg("-c")
            .arg(script)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start the magi");
        let asked = read_pid(&pids).expect("the magi said which melchior it started");
        Self {
            shell,
            asked,
            _dir: dir,
        }
    }

    /// End the magi the way a crash would: with nothing running inside it.
    fn killed(&mut self) {
        let _ = self.shell.kill();
        let _ = self.shell.wait();
    }

    /// Leave nothing running, whatever the assertions are about to do.
    ///
    /// The directory is not this function's job any more: it goes when `self` does.
    fn cleared(mut self) {
        self.killed();
        let _ = Command::new("kill")
            .arg("-9")
            .arg(self.asked.to_string())
            // Already gone is the passing case, and its complaint reads like a failure.
            .stderr(Stdio::null())
            .status();
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
        std::thread::sleep(Duration::from_millis(20));
    }
    None
}

/// Whether a process exists and is not merely a corpse waiting to be reaped.
///
/// The state field rather than the directory's existence: the process here is started by a shell
/// that is about to be killed, so a zombie is the expected shape of "gone".
fn alive(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .is_ok_and(|stat| stat.split_whitespace().nth(2) != Some("Z"))
}

/// Wait for `pid` to go away, and say whether it did.
fn gone_within(pid: u32, patience: Duration) -> bool {
    let deadline = Instant::now() + patience;
    while Instant::now() < deadline {
        if !alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    !alive(pid)
}

#[test]
fn a_turn_does_not_outlive_the_magi_that_asked_for_it() {
    let (told, arrived) = std::sync::mpsc::channel();
    let port = hanging(told);
    let mut magi = Magi::starting(port);
    let asked = magi.asked;

    // Killed only once the turn is genuinely under way. Before the connection there is still an
    // exit this could be mistaken for — a refusal, a catalog that would not load — and the
    // guarantee is about the melchior that is holding something.
    let connected = arrived.recv_timeout(Duration::from_secs(30)).is_ok();

    magi.killed();
    let went = gone_within(asked, NOTICES_WITHIN);

    magi.cleared();
    assert!(connected, "the ask reached the provider and hung there");
    assert!(
        went,
        "a turn must not outlive the magi that asked for it: it is holding a connection open"
    );
}
