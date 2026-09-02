//! `atom` — the agent layer, as a program.
//!
//! Three jobs, and they are three because each has a different lifetime and a different way of
//! being talked to:
//!
//! | | |
//! |---|---|
//! | `atom serve` | binds this session's socket and answers for it, for as long as its parent lives |
//! | `atom tool` | the tool a model calls, spoken over the harness's own pipe protocol |
//! | `atom lua-api` | prints the client library, for redirecting into a config directory |
//!
//! # Nothing here is a daemon
//!
//! `serve` is a *child*. It reads its parent's pipe and exits when that closes, so a session's
//! socket lives exactly as long as the session does. A layer that outlived its harness would
//! leave a name in the directory that answers and cannot act, which is worse than a name that
//! is simply gone: a sibling would send to it and be told the message landed.
//!
//! # Why a pipe and not a library call
//!
//! The harness could link this crate — it did once. It is a separate program now so that a
//! harness in another language can use it, and so that the two can be released apart. What
//! crosses the boundary is what a harness genuinely cannot do for itself, and no more:
//!
//! - **up**, on stdin: what this session is doing, so `status` can answer truthfully
//! - **down**, on stdout: a message arrived, and here it is
//!
//! One JSON object per line, because both ends are line-oriented already and a length prefix
//! buys nothing on a pipe that nothing else shares.

use std::io::{BufRead, Write};

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("serve") => serve(args.collect()),
        Some("tool") => tool::run(),
        Some("lua-api") => {
            print!("{}", atom::CLIENT);
            Ok(())
        }
        Some("verbs") => {
            for (verb, does) in atom::wire::VERBS {
                println!("{verb:<10} {does}");
            }
            Ok(())
        }
        Some(other) => {
            eprintln!("atom: no such command: {other}");
            eprintln!("usage: atom serve | tool | lua-api | verbs");
            std::process::exit(2);
        }
        None => {
            eprintln!("usage: atom serve | tool | lua-api | verbs");
            eprintln!();
            eprintln!("  serve     bind this session's socket and answer for it");
            eprintln!("  tool      the tool a model calls, over the harness's pipe protocol");
            eprintln!("  lua-api   print the Lua client library");
            eprintln!("  verbs     what a session answers");
            std::process::exit(2);
        }
    }
}

mod tool;

/// What the parent tells us, one JSON object per line on stdin.
#[derive(serde::Deserialize)]
#[serde(tag = "say", rename_all = "lowercase")]
enum Told {
    /// What this session is doing now, so `status` answers truthfully rather than plausibly.
    Doing {
        /// Whether a turn is running.
        busy: bool,
        /// For how long, in seconds.
        #[serde(default)]
        working_for: u64,
        /// How much is waiting to be read.
        #[serde(default)]
        waiting: usize,
    },
}

/// What we tell the parent, one JSON object per line on stdout.
#[derive(serde::Serialize)]
#[serde(tag = "heard", rename_all = "lowercase")]
enum Heard {
    /// Ready: the socket is bound and this session can be reached.
    Listening {
        /// Where, so a parent can say so and a test can find it.
        at: String,
    },
    /// A message arrived for this session.
    Message {
        /// Who sent it, as `project/role/id`.
        who: String,
        /// What sort it is.
        sort: String,
        /// What they said.
        text: String,
        /// The message it answers, when it answers one.
        about: Option<String>,
    },
    /// Somebody with the right to stop this session did.
    Stopped,
}

/// Bind this session's socket and answer for it until the parent goes away.
///
/// Everything it needs to *be* somebody comes from the environment, the same way it did when
/// this ran inside a harness: `ATOM_PROJECT`, `ATOM_ROLE`, `ATOM_ID`, and the `AXON_*` names
/// they replaced. That is the one thing a separate process cannot work out for itself.
fn serve(_args: Vec<String>) -> std::io::Result<()> {
    let Some(me) = atom::directory::mine() else {
        eprintln!(
            "atom serve: no session to be. Set {} and {} — they say which session this is \
             answering for, and nothing on disk can be asked instead.",
            atom::directory::PROJECT,
            atom::directory::ID
        );
        std::process::exit(2);
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let (about_tx, about_rx) = tokio::sync::watch::channel(atom::answering::About {
            me: me.clone(),
            parent: atom::directory::parent(),
            token: atom::directory::token(),
            busy: false,
            working_for: 0,
            inbox: Vec::new(),
        });
        let (arrived_tx, mut arrived) = tokio::sync::mpsc::channel(64);
        let (stopped_tx, mut stopped) = tokio::sync::mpsc::channel(1);

        // The note beside the socket, so the tree can be read off the directory: a session that
        // finds this one there can tell whose subagent it is without asking it, and without
        // trusting what it would have said.
        atom::directory::announce(&me);
        let at = atom::directory::listening_at(&me);

        // Bound *here*, and only then announced. Spawning the accept loop and saying "listening"
        // in one breath announces a future: the bind is several awaits away, and a parent that
        // started sending on the strength of it met its own session as "nothing is listening".
        let listener = match atom::serving::listening_on(&at).await {
            Ok(listener) => listener,
            Err(why) => {
                eprintln!("atom serve: {}: {why}", at.display());
                std::process::exit(1);
            }
        };
        say(&Heard::Listening {
            at: at.display().to_string(),
        });
        tokio::spawn(async move {
            let _ = atom::serving::accept(
                listener,
                atom::serving::Serving {
                    about: about_rx,
                    arrived: arrived_tx,
                    stopped: stopped_tx,
                },
            )
            .await;
        });

        // The parent's side of the pipe, read on a thread because it is a blocking stdin and
        // everything else here is not.
        let (told_tx, mut told) = tokio::sync::mpsc::channel::<Told>(16);
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines().map_while(Result::ok) {
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<Told>(&line) {
                    // A line we cannot read is the parent's bug, not a reason to stop answering
                    // a socket other sessions are using.
                    Err(why) => eprintln!("atom serve: {why}"),
                    Ok(told) => {
                        if told_tx.blocking_send(told).is_err() {
                            return;
                        }
                    }
                }
            }
        });

        let mut inbox: Vec<atom::wire::Message> = Vec::new();
        loop {
            tokio::select! {
                Some(message) = arrived.recv() => {
                    say(&Heard::Message {
                        who: message.from.clone(),
                        sort: name_of(message.sort),
                        text: message.text.clone(),
                        about: message.about.clone(),
                    });
                    inbox.push(message);
                    about_tx.send_modify(|about| about.inbox.clone_from(&inbox));
                }
                // **`None` here is the parent letting go, and it is the whole lifetime rule.**
                // Matched rather than left to `else`, because a `select!` arm whose pattern
                // does not match is *disabled*, not taken: with `Some(..) = told.recv()` the
                // closed pipe silently dropped this branch and the loop went on waiting on the
                // socket. `atom serve` outlived the session that started it, kept a name in the
                // directory that answers and cannot act, and a sibling sending to it would be
                // told the message landed. Found by closing the pipe and looking.
                told = told.recv() => match told {
                    None => break,
                    Some(Told::Doing { busy, working_for, waiting }) => {
                        about_tx.send_modify(|about| {
                            about.busy = busy;
                            about.working_for = working_for;
                            // What the *harness* still holds unread, which is not the same as
                            // what has arrived here: it reads its inbox and acts on it, and a
                            // count that only ever grew would tell a sibling this session is
                            // falling behind when it is keeping up.
                            about.inbox.truncate(waiting.min(about.inbox.len()));
                        });
                    }
                },
                Some(()) = stopped.recv() => {
                    say(&Heard::Stopped);
                    break;
                }
                // Both remaining arms can close on their own — the serving task dropping its
                // senders — and either way there is nobody left to answer for.
                else => break,
            }
        }
        atom::directory::forget(&me);
        let _ = std::fs::remove_file(&at);
        Ok(())
    })
}

/// The wire name of a sort.
fn name_of(sort: atom::wire::Sort) -> String {
    serde_json::to_value(sort)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| "note".to_owned())
}

/// One line to the parent, flushed.
///
/// Flushed every time because the parent is waiting on it: a buffered "listening" that arrives
/// when the buffer happens to fill is a parent that hangs at startup for no reason it can see.
fn say(heard: &Heard) {
    let Ok(line) = serde_json::to_string(heard) else {
        return;
    };
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}
