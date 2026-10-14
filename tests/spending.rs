//! The spending cap, against the real binary and a provider on a local port that counts what it
//! is sent: a run at its cap is refused with nothing sent, what a child spends counts against the
//! run its root began, and an attempt that failed is paid for before the next is tried.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

use melchior::mind::spending::Ledger;
use melchior::scratch::Scratch;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const MELCHIOR: &str = env!("CARGO_BIN_EXE_melchior");

/// What the provider took for one answer: a token in and one out, which its rates make $2.
const USAGE: &str = r#"data: {"choices":[],"usage":{"prompt_tokens":1,"completion_tokens":1}}"#;

const ASK: &str = r#"{"model":"spendtest/dear","context":{"messages":[{"role":"user","content":[{"type":"text","text":"hi"}]}]}}"#;

/// How the provider ends an answer.
#[derive(Clone, Copy)]
enum Ending {
    Whole,
    /// The connection closed mid-body, after the usage: a failure melchior tries again.
    Dropped,
}

/// One world: a home, a config, a runtime directory and a provider, all gone with the guard.
struct World {
    dir: Scratch,
    asked: Arc<AtomicUsize>,
}

impl World {
    fn new(name: &str, ending: Ending) -> Self {
        let dir = Scratch::new("mel-sp", name);
        let asked = Arc::new(AtomicUsize::new(0));
        let port = provider(ending, Arc::clone(&asked));
        std::fs::create_dir_all(dir.join("cfg/plugin")).expect("config");
        std::fs::create_dir_all(dir.join("cache/melchior/models")).expect("cache");
        melchior::places::ensure(&dir.join("r")).expect("runtime");
        // Fresh and empty, so nothing is discovered over the network.
        for discovered in ["openrouter", "ollama"] {
            std::fs::write(
                dir.join(format!("cache/melchior/models/{discovered}.json")),
                "[]",
            )
            .expect("cache");
        }
        std::fs::write(
            dir.join("cfg/providers.lua"),
            format!(
                r#"melchior.provider("spendtest", {{
  name = "Spend test", api = "openai-completions",
  base_url = "http://127.0.0.1:{port}/v1", auth = {{ kind = "none" }},
  models = {{ {{ id = "dear", name = "Dear", context_window = 8192, max_tokens = 128,
                cost = {{ input = 1000000, output = 1000000 }} }} }},
}})
"#
            ),
        )
        .expect("provider");
        Self { dir, asked }
    }

    /// Set the cap the way a person does, in a file of melchior's own.
    fn capped(&self, lua: &str) {
        std::fs::write(self.dir.join("cfg/plugin/cap.lua"), lua).expect("cap");
    }

    fn asked(&self) -> usize {
        self.asked.load(Ordering::SeqCst)
    }

    fn ledger(&self, run: &str) -> Ledger {
        Ledger::at(&self.dir.join("r/melchior/.spent"), run)
    }

    /// `melchior` with this world's environment and nothing of the caller's.
    fn melchior(&self, envs: &[(&str, &str)]) -> Command {
        let mut command = Command::new(MELCHIOR);
        command
            .env_clear()
            .env("HOME", &*self.dir)
            .env("TMPDIR", std::env::temp_dir())
            .env("XDG_RUNTIME_DIR", self.dir.join("r"))
            .env("XDG_CACHE_HOME", self.dir.join("cache"))
            .env("XDG_DATA_HOME", self.dir.join("data"))
            .env("MELCHIOR_CONFIG", self.dir.join("cfg"))
            .envs(envs.iter().copied())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        command
    }

    /// One `melchior ask`, started.
    fn asking(&self, envs: &[(&str, &str)]) -> Child {
        let mut child = self
            .melchior(envs)
            .arg("ask")
            .spawn()
            .expect("melchior ask");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(ASK.as_bytes())
            .expect("the ask");
        child
    }

    /// One `melchior ask`, run to its end: what it said, a line each.
    fn ask(&self, envs: &[(&str, &str)]) -> Vec<serde_json::Value> {
        let mut said = String::new();
        let mut child = self.asking(envs);
        child
            .stdout
            .take()
            .expect("stdout")
            .read_to_string(&mut said)
            .expect("read");
        child.wait().expect("ended");
        said.lines()
            .map(|line| serde_json::from_str(line).expect("one JSON object"))
            .collect()
    }
}

/// How the stream an ask said ended.
fn last(said: &[serde_json::Value]) -> &serde_json::Value {
    said.last().expect("it said something")
}

fn text(path: &Path) -> &str {
    path.to_str().expect("a path in UTF-8")
}

/// A provider that answers every request with `ending`, counting the requests it reads.
fn provider(ending: Ending, asked: Arc<AtomicUsize>) -> u16 {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("a port");
    let port = listener.local_addr().expect("its address").port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let asked = Arc::clone(&asked);
            std::thread::spawn(move || answer(&mut stream, ending, &asked));
        }
    });
    port
}

fn answer(stream: &mut std::net::TcpStream, ending: Ending, asked: &AtomicUsize) {
    let mut reading = BufReader::new(stream.try_clone().expect("clone"));
    let mut length = 0;
    loop {
        let mut line = String::new();
        if reading.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        if let Some(said) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            length = said.trim().parse().unwrap_or(0);
        }
        if line == "\r\n" {
            break;
        }
    }
    let mut body = vec![0; length];
    if reading.read_exact(&mut body).is_err() {
        return;
    }
    asked.fetch_add(1, Ordering::SeqCst);
    let chunk = |text: &str| format!("{:x}\r\n{text}\r\n", text.len());
    let mut out = String::from(
        "HTTP/1.1 200 Fixture\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\
         Connection: close\r\n\r\n",
    );
    out.push_str(&chunk(&format!("{USAGE}\n\n")));
    if matches!(ending, Ending::Whole) {
        let done =
            r#"data: {"choices":[{"index":0,"delta":{"content":"hi"},"finish_reason":"stop"}]}"#;
        out.push_str(&chunk(&format!("{done}\n\ndata: [DONE]\n\n")));
        out.push_str("0\r\n\r\n");
    }
    let _ = stream.write_all(out.as_bytes());
    let _ = stream.flush();
}

#[test]
fn a_run_at_its_cap_sends_nothing_until_the_cap_is_raised() {
    let world = World::new("cap", Ending::Whole);
    // Set the way a coordinator sets it: in the directory it names.
    let given = world.dir.join("given");
    std::fs::create_dir_all(&given).expect("given");
    std::fs::write(given.join("melchior.lua"), "melchior.spend_cap_usd = 1\n").expect("cap");
    let envs = [
        ("MAGI_MELCHIOR_SESSION", "alpha-rho-1789480440"),
        ("NERV_GIVEN", text(&given)),
    ];

    assert_eq!(last(&world.ask(&envs))["event"], "stop");
    assert_eq!(world.asked(), 1);
    assert_eq!(world.ledger("alpha-rho-1789480440").spent(), 2_000_000);

    let said = world.ask(&envs);
    assert_eq!(last(&said)["event"], "failed", "{said:?}");
    assert_eq!(
        last(&said)["why"],
        "invalid",
        "a refusal nobody should retry"
    );
    let message = last(&said)["message"]
        .as_str()
        .expect("a message")
        .to_owned();
    for part in ["$2.00", "$1.00", "set melchior.spend_cap_usd"] {
        assert!(message.contains(part), "{part} missing: {message}");
    }
    assert_eq!(world.asked(), 1, "the refused ask reached the provider");

    std::fs::write(given.join("melchior.lua"), "melchior.spend_cap_usd = 5\n").expect("cap");
    assert_eq!(
        last(&world.ask(&envs))["event"],
        "stop",
        "raised, it asks again"
    );
    assert_eq!(world.asked(), 2);
}

#[test]
fn what_a_child_spends_counts_against_the_run_its_root_began() {
    let world = World::new("tree", Ending::Whole);
    world.capped("melchior.spend_cap_usd = 1\n");
    let (mine, theirs) = (world.dir.join("given-root"), world.dir.join("given-child"));

    // The root's own session, which names the run and the directory its coordinator gave it.
    let mut root = world
        .melchior(&[("NERV_GIVEN", text(&mine))])
        .args(["serve", "--project", "spend"])
        .spawn()
        .expect("melchior serve");
    let mut first = String::new();
    let mut hearing = BufReader::new(root.stdout.take().expect("stdout"));
    hearing.read_line(&mut first).expect("listening");
    let listening: serde_json::Value = serde_json::from_str(&first).expect("one JSON object");
    let run = listening["run"].as_str().expect("its run").to_owned();

    let child = world.ask(&[
        ("MAGI_MELCHIOR_SESSION", &run),
        ("NERV_GIVEN", text(&theirs)),
    ]);
    assert_eq!(last(&child)["event"], "stop", "{child:?}");
    assert_eq!(world.ledger(&run).spent(), 2_000_000);

    let own = world.ask(&[("NERV_GIVEN", text(&mine))]);
    assert_eq!(last(&own)["event"], "failed", "{own:?}");
    assert_eq!(
        world.asked(),
        1,
        "the root's ask was sent past its child's spending"
    );

    drop(root.stdin.take());
    root.wait().expect("the root ended");
    drop(hearing);
    assert!(
        !world.dir.join("r/melchior/.spent").exists(),
        "the run's ledger outlived the root that began it"
    );
}

#[test]
fn an_attempt_that_failed_is_paid_for_before_the_next_is_tried() {
    let world = World::new("failed", Ending::Dropped);
    let mut asking = world.asking(&[("MAGI_MELCHIOR_SESSION", "alpha-rho-1789480441")]);
    let (said_tx, said) = std::sync::mpsc::channel();
    let out = BufReader::new(asking.stdout.take().expect("stdout"));
    std::thread::spawn(move || {
        for line in out.lines().map_while(Result::ok) {
            let _ = said_tx.send(line);
        }
    });
    // The next attempt waits a backoff, and the first has been paid for by the time it is said.
    loop {
        let line = said
            .recv_timeout(std::time::Duration::from_secs(20))
            .expect("it retries");
        if line.contains(r#""event":"retrying""#) {
            break;
        }
    }
    assert_eq!(world.ledger("alpha-rho-1789480441").spent(), 2_000_000);
    asking.kill().expect("ended");
    asking.wait().expect("reaped");
}

#[test]
fn asks_at_once_lose_nothing_and_false_turns_the_cap_off() {
    let world = World::new("many", Ending::Whole);
    world.capped("melchior.spend_cap_usd = false\n");
    let ledger = world.ledger("alpha-rho-1789480442");
    ledger.add(50_000_000).expect("well past the default");
    let asking: Vec<Child> = (0..4)
        .map(|_| world.asking(&[("MAGI_MELCHIOR_SESSION", "alpha-rho-1789480442")]))
        .collect();
    for mut child in asking {
        let mut said = String::new();
        child
            .stdout
            .take()
            .expect("stdout")
            .read_to_string(&mut said)
            .expect("read");
        child.wait().expect("ended");
        assert!(said.contains(r#""event":"stop""#), "{said}");
    }
    assert_eq!(world.asked(), 4);
    assert_eq!(ledger.spent(), 50_000_000 + 4 * 2_000_000);
}
