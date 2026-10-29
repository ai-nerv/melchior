//! The real `melchior` binary, driven the way a harness drives it: shared by the integration tests
//! that start sessions, talk to them and watch what crosses their pipes.
#![allow(dead_code)]

use melchior::scratch::Scratch;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};

/// How long a test waits for a session to say something before it fails rather than hangs.
const PATIENCE: std::time::Duration = std::time::Duration::from_secs(10);

/// A running `melchior serve`, and the runtime directory it is alone in.
pub struct Serving {
    pub child: Child,
    /// What it says on its stdout, a line at a time, read on a thread so a wait can end.
    lines: std::sync::mpsc::Receiver<String>,
    /// The directory it listens in, owned when this session is the one that made it.
    ///
    /// **`Some` for exactly one session per directory.** [`Serving::beside`] and
    /// [`Serving::under`] start a second one in somebody else's, and a guard each would mean the
    /// first to finish deleting the socket the other is still answering on.
    own: Option<Scratch>,
    runtime: std::path::PathBuf,
    /// Where it said it was listening, rather than where a test guessed it would be.
    at: std::path::PathBuf,
    /// What it said it is called, for addressing it and for checking who a message came from.
    named: String,
}

impl Serving {
    /// Start one alone in its own runtime directory, under `$TMPDIR` through a guard that removes
    /// it on the unwind. Short, because a unix socket path is capped at `SUN_LEN`.
    pub fn start(name: &str) -> Self {
        let own = Scratch::new("melchior-t", name);
        let mut serving = Self::beside(&own, "alpha-rho");
        serving.own = Some(own);
        serving
    }

    /// Start one alone, by way of a shell, so this test process is a stranger to it: neither the
    /// process that started it nor one that process started.
    pub fn stranger(name: &str) -> Self {
        let own = Scratch::new("melchior-s", name);
        let mut serving = Self::spawned(&own, &[("MAGI_MELCHIOR_ID", "alpha-rho")], true);
        serving.own = Some(own);
        serving
    }

    /// Start one as a child: a parent named in its environment, and the secret handed to it on
    /// its stdin the way a harness hands it after `melchior fork`.
    pub fn under(runtime: &std::path::Path, id: &str, parent: &str, token: &str) -> Self {
        let mut serving = Self::spawned(
            runtime,
            &[("MAGI_MELCHIOR_ID", id), ("MAGI_MELCHIOR_PARENT", parent)],
            false,
        );
        serving.told(&format!(r#"{{"event":"token","token":"{token}"}}"#));
        serving
    }

    /// Start one in a runtime directory somebody else may already be in, so two sessions can see
    /// each other: `start` gives each its own directory, and the project wall does its job.
    pub fn beside(runtime: &std::path::Path, id: &str) -> Self {
        Self::spawned(runtime, &[("MAGI_MELCHIOR_ID", id)], false)
    }

    /// `melchior serve` in project `demo` in `runtime`, with `envs`, started by this process or by
    /// a shell it starts.
    pub fn spawned(runtime: &std::path::Path, envs: &[(&str, &str)], shell: bool) -> Self {
        melchior::places::ensure(runtime).expect("a runtime directory");
        let binary = env!("CARGO_BIN_EXE_melchior");
        let mut command = if shell {
            let mut through = Command::new("sh");
            through.arg("-c").arg("\"$0\" serve; :").arg(binary);
            through
        } else {
            let mut direct = Command::new(binary);
            direct.arg("serve");
            direct
        };
        let child = command
            .env("MAGI_MELCHIOR_PROJECT", "demo")
            .envs(envs.iter().copied())
            .env("XDG_RUNTIME_DIR", runtime)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("melchior serve");
        Self::listened(child, runtime)
    }

    /// Hold a started `serve`, once it has said where it listens.
    fn listened(mut child: Child, runtime: &std::path::Path) -> Self {
        let out = BufReader::new(child.stdout.take().expect("stdout"));
        let (said_tx, lines) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in out.lines().map_while(Result::ok) {
                if said_tx.send(line).is_err() {
                    return;
                }
            }
        });
        let first = lines.recv_timeout(PATIENCE).unwrap_or_else(|why| {
            let status = child.try_wait();
            let _ = child.kill();
            let reaped = child.wait();
            panic!("serve did not announce readiness: {why}; status {status:?}; reaped {reaped:?}");
        });
        let said: serde_json::Value = serde_json::from_str(&first).expect("one JSON object");
        assert_eq!(said["event"], "listening", "{first}");
        let at = std::path::PathBuf::from(said["at"].as_str().expect("it says where"));
        let named = said["as"].as_str().expect("it says who").to_owned();
        Self {
            child,
            lines,
            own: None,
            runtime: runtime.to_path_buf(),
            at,
            named,
        }
    }

    /// What it calls itself, as `project/role/id`.
    pub fn named(&self) -> String {
        self.named.clone()
    }

    /// The runtime directory it listens in, for putting a second session beside it.
    pub fn runtime(&self) -> std::path::PathBuf {
        self.runtime.clone()
    }

    /// Where it said it is listening.
    pub fn at(&self) -> std::path::PathBuf {
        self.at.clone()
    }

    /// Tell it something on its pipe.
    pub fn told(&mut self, line: &str) {
        let stdin = self.child.stdin.as_mut().expect("stdin");
        writeln!(stdin, "{line}").expect("wrote");
        stdin.flush().expect("flushed");
    }

    /// Read until it says something of `kind`, skipping the rest: `around` arrives on its own
    /// schedule and would otherwise be read as the answer to whatever was asked.
    pub fn heard(&mut self, kind: &str) -> serde_json::Value {
        for _ in 0..256 {
            let Ok(line) = self.lines.recv_timeout(PATIENCE) else {
                break;
            };
            let said: serde_json::Value = serde_json::from_str(&line).expect("one JSON object");
            if said["event"] == kind {
                return said;
            }
        }
        panic!("nothing of kind {kind} came back");
    }

    /// One call over the socket, framed by hand, the way a sibling would.
    pub fn asked(&self, body: &str) -> serde_json::Value {
        let mut sock = std::os::unix::net::UnixStream::connect(self.at()).expect("connected");
        sock.set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .expect("a timeout");
        let mut frame = u32::try_from(body.len())
            .expect("fits")
            .to_be_bytes()
            .to_vec();
        frame.extend_from_slice(body.as_bytes());
        sock.write_all(&frame).expect("wrote");
        sock.shutdown(std::net::Shutdown::Write).expect("hung up");
        let mut back = Vec::new();
        sock.read_to_end(&mut back).expect("read");
        serde_json::from_slice(&back[4..]).expect("it is JSON")
    }

    /// End it the way the kernel does, `SIGTERM` rather than `SIGKILL`, and wait for it to go.
    pub fn signalled(&mut self) -> bool {
        let _ = Command::new("kill")
            .arg("-TERM")
            .arg(self.child.id().to_string())
            .status();
        for _ in 0..200 {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        false
    }

    /// Let go of the pipe, and wait. The directory goes with `self`, on the return and on the
    /// panic alike.
    pub fn let_go(mut self) -> bool {
        drop(self.child.stdin.take());
        for _ in 0..100 {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return !self.at().exists();
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let _ = self.child.kill();
        panic!("melchior serve outlived the parent that started it");
    }
}

/// What is in a project's directory, by name, whether or not the directory is still there.
pub fn left_in(project: &std::path::Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(project) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// One `melchior tool` call, as the session `named`, in the directory it is listening in.
pub fn tool(runtime: &std::path::Path, named: &str, args: &[&str]) -> std::process::Output {
    let mut parts = named.split('/');
    Command::new(env!("CARGO_BIN_EXE_melchior"))
        .arg("tool")
        .args(args)
        .env("XDG_RUNTIME_DIR", runtime)
        .env("MAGI_MELCHIOR_PROJECT", parts.next().unwrap_or_default())
        .env("MAGI_MELCHIOR_ROLE", parts.next().unwrap_or_default())
        .env("MAGI_MELCHIOR_ID", parts.next().unwrap_or_default())
        .output()
        .expect("melchior tool runs")
}

pub fn stdout(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

pub fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// The last segment, which is what a sibling is addressed by inside one project.
pub fn id_of(named: &str) -> &str {
    named.rsplit('/').next().unwrap_or_default()
}

/// One hand-written call, framed the way the family frames everything.
pub fn ask(at: &std::path::Path, body: &str) -> serde_json::Value {
    let mut sock = std::os::unix::net::UnixStream::connect(at).expect("connected");
    sock.set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .expect("a timeout");
    let mut frame = u32::try_from(body.len())
        .expect("fits")
        .to_be_bytes()
        .to_vec();
    frame.extend_from_slice(body.as_bytes());
    sock.write_all(&frame).expect("wrote");
    let mut header = [0_u8; 4];
    sock.read_exact(&mut header).expect("read a header");
    let mut answer = vec![0_u8; u32::from_be_bytes(header) as usize];
    sock.read_exact(&mut answer).expect("read a body");
    serde_json::from_slice(&answer).expect("it is JSON")
}
