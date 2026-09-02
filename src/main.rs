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
        Some("serve") => serve(&flags(args)),
        Some("tool") => tool::run(),
        Some("brief") => {
            let asked = flags(args);
            brief(asked.get("project").map(String::as_str), &asked);
            Ok(())
        }
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
            eprintln!("usage: atom serve | tool | brief | lua-api | verbs");
            std::process::exit(2);
        }
        None => {
            eprintln!("usage: atom serve | tool | brief | lua-api | verbs");
            eprintln!();
            eprintln!("  serve     bind this session's socket and answer for it");
            eprintln!("  tool      the vocabulary a model calls, one exec per request");
            eprintln!("  brief     what to tell a model about the sessions a prompt named");
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
    /// What the person said to a request this session was asked to answer.
    ///
    /// Comes back up rather than being decided here, because the question was never this
    /// process's to answer: it asks whether another session may act with this one's authority,
    /// and only somebody at a keyboard can say.
    Answered {
        /// Which request, as [`Heard::Asked`] named it.
        id: String,
        /// Whether they said yes.
        accept: bool,
        /// Anything the accepting harness wants the adopted one to have, carried unread.
        ///
        /// **Opaque on purpose.** What a harness lends a session it has taken on — permissions,
        /// in axon's case — is that harness's own idea. A layer that understood it would be a
        /// second place needing a change every time it changed. This goes in one side and comes
        /// out the other, and nothing here looks at it.
        #[serde(default)]
        handover: Option<String>,
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
        /// And as whom, since the name may have been chosen here rather than handed down.
        ///
        /// A parent that passed only `--project` has no other way to learn it, and it needs to:
        /// the name goes on its own screen and into what it signs.
        #[serde(rename = "as")]
        named: String,
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
    /// Who else is in this project, whenever that changes.
    ///
    /// Pushed rather than asked for, because the thing that wants it is a completion popup: a
    /// harness offering `$` on a keystroke cannot spawn a process or open a socket to answer it,
    /// and one that cached the answer at startup would offer a session that has since gone and
    /// miss the one that just arrived.
    ///
    /// It also keeps the layout here. A harness listing the directory for itself would be a
    /// second place that knows where sockets live and what a `.parent` file is called.
    Around {
        /// Every session listening, by id, this one included.
        names: Vec<String>,
    },
    /// Somebody with the right to stop this session did.
    Stopped,
    /// Another session is asking to become this one's child, and a person has to answer.
    ///
    /// Up the pipe rather than into the inbox, because the answer is not a model's to give: it
    /// decides whether another session may act with this one's authority. A model accepting on
    /// its own behalf would be granting itself a second pair of hands.
    Asked {
        /// The request, so an answer names one rather than "the last thing asked".
        id: String,
        /// Who is asking, as `project/role/id`.
        who: String,
        /// Why, in their words. The person answering has no other way to know.
        why: String,
    },
    /// A session this one asked to be taken on by has accepted.
    ///
    /// Its own line rather than the message that also arrives, because the two have different
    /// readers. The message is for the model — somebody said yes, here is who. This is for the
    /// harness, and carries what the accepting session lent it, which no model should see:
    /// permissions written into a transcript are permissions a model can read and reason about
    /// acquiring.
    Adopted {
        /// Who took this session on, as `project/role/id`.
        by: String,
        /// What they handed over, exactly as they wrote it.
        handover: Option<String>,
    },
}

/// Bind this session's socket and answer for it until the parent goes away.
///
/// Everything it needs to *be* somebody comes from the environment, the same way it did when
/// this ran inside a harness: `ATOM_PROJECT`, `ATOM_ROLE`, `ATOM_ID`, and the `AXON_*` names
/// they replaced. That is the one thing a separate process cannot work out for itself.
fn serve(asked: &std::collections::BTreeMap<String, String>) -> std::io::Result<()> {
    // Named here when the caller only says which project it is in, and that is the useful way
    // round: a harness choosing its own name is choosing out of a namespace it cannot see, and
    // the collision would surface as a failed bind after it had told everyone what it was
    // called. Whoever holds the directory should be the one that looks first.
    let me = match (atom::directory::mine(), asked.get("project")) {
        (Some(me), _) => me,
        (None, Some(project)) => atom::identity::free_in(project),
        (None, None) => {
            eprintln!(
                "atom serve: no session to be. Pass --project, or set {} and {} — one of them \
                 has to say which session this is answering for, and nothing on disk can be \
                 asked instead.",
                atom::directory::PROJECT,
                atom::directory::ID
            );
            std::process::exit(2);
        }
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let (about_tx, about_rx) = tokio::sync::watch::channel(atom::answering::About {
            me: me.clone(),
            parent: atom::directory::parent_of(&me),
            token: atom::directory::token(),
            busy: false,
            working_for: 0,
            inbox: Vec::new(),
        });
        let (arrived_tx, mut arrived) = tokio::sync::mpsc::channel(64);
        let (asked_tx, mut asked) = tokio::sync::mpsc::channel(16);
        let (adopted_tx, mut adopted) = tokio::sync::mpsc::channel(4);
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
            named: me.full(),
        });
        tokio::spawn(async move {
            let _ = atom::serving::accept(
                listener,
                atom::serving::Serving {
                    about: about_rx,
                    arrived: arrived_tx,
                    asked: asked_tx,
                    adopted: adopted_tx,
                    stopped: stopped_tx,
                },
            )
            .await;
        });

        // Who else is in the project, watched on a slow tick. There is no event to hook: the
        // directory is the registry, and a session appears in it by binding a socket that this
        // process has no reason to be told about. Two seconds is well under how long it takes
        // somebody to notice a name is missing from a completion popup, and the check is a
        // directory listing.
        let mut around: Vec<String> = Vec::new();
        let mut sweep = tokio::time::interval(std::time::Duration::from_secs(2));
        sweep.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

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
        // Requests put to this session and not yet answered. Held here rather than in the
        // serving task because the answer arrives on the *pipe*, from the person, long after the
        // connection that carried the question has closed.
        let mut pending: Vec<atom::wire::Request> = Vec::new();
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
                // Held here until the person answers. Kept rather than answered on the spot,
                // because the socket call that carried it has already been replied to: the
                // caller was told the question was put, not what the answer was.
                Some(request) = asked.recv() => {
                    say(&Heard::Asked {
                        id: request.id.clone(),
                        who: request.from.clone(),
                        why: request.why.clone(),
                    });
                    pending.push(request);
                }
                // Straight up the pipe, never into the inbox. What a parent lends a session it
                // has taken on is for the harness; a model that could read it in its own
                // transcript could reason about acquiring more.
                Some((by, handover)) = adopted.recv() => {
                    say(&Heard::Adopted { by, handover });
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
                    // What the person said. Taking a session on is written to the directory
                    // *here*, by the side that consented — the asker writing its own note would
                    // be a session appointing its own parent, which is the one thing the whole
                    // handshake exists to prevent.
                    Some(Told::Answered { id, accept, handover }) => {
                        let Some(at) = pending.iter().position(|held| held.id == id) else {
                            continue;
                        };
                        let request = pending.remove(at);
                        if accept && let Some(them) = atom::identity::Identity::read(&request.from)
                        {
                            atom::directory::adopted(&them, &me.id);
                        }
                        // Told either way, and told by us: the asker has been waiting since its
                        // call was answered with "the question has been put", and a silence it
                        // could not tell from a refusal would leave it waiting for good.
                        atom::directory::answer_request(
                            &request.from,
                            &me,
                            accept,
                            handover.as_deref(),
                        );
                    }
                },
                Some(()) = stopped.recv() => {
                    say(&Heard::Stopped);
                    break;
                }
                _ = sweep.tick() => {
                    // Re-read on the tick, because being adopted happens to this session from
                    // outside it: whoever accepted wrote the note, and no variable can be set on
                    // a process already running. Left as it stood at startup, a session that had
                    // been taken on would go on telling callers it was a main, and its own `kin`
                    // would disagree with every other reading of the same directory.
                    let mine = atom::directory::parent_of(&me);
                    if mine != about_tx.borrow().parent {
                        about_tx.send_modify(|about| about.parent.clone_from(&mine));
                    }
                    let now = atom::directory::listening(&me.project);
                    // Only on a change. A line every two seconds for the life of a session is a
                    // pipe nobody can read a log out of, and a parent that has to diff it.
                    if now != around {
                        around = now;
                        say(&Heard::Around { names: around.clone() });
                    }
                }
                // Both remaining arms can close on their own — the serving task dropping its
                // senders — and either way there is nobody left to answer for.
                else => break,
            }
        }
        atom::directory::forget(&me);
        let _ = std::fs::remove_file(&at);
        // And the directory itself, if this was the last session in the project. It refuses
        // while anybody else is still there, so whoever leaves last does it.
        atom::directory::leave(&me.project);
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

/// `--name value` pairs, as a caller wrote them.
///
/// Repeats accumulate under one key, separated by newlines, because `--name a --name b` is how
/// a shell says "these several" and dropping all but the last would silently brief on one.
fn flags(args: impl Iterator<Item = String>) -> std::collections::BTreeMap<String, String> {
    let mut out: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    let mut args = args.peekable();
    while let Some(flag) = args.next() {
        let Some(key) = flag.strip_prefix("--") else {
            continue;
        };
        let (key, value) = match key.split_once('=') {
            Some((key, value)) => (key.to_owned(), value.to_owned()),
            None => (key.to_owned(), args.next().unwrap_or_default()),
        };
        if value.is_empty() {
            continue;
        }
        out.entry(key)
            .and_modify(|held| {
                held.push('\n');
                held.push_str(&value);
            })
            .or_insert(value);
    }
    out
}

/// What a harness should put in front of a model about the sessions a prompt named.
///
/// Printed rather than returned, because the caller is a program that runs this and reads what
/// it said. It is the one piece of the surface that is *about* a prompt, and it still does not
/// read one: scanning for a name means knowing what a prompt, a cursor and a sigil table are,
/// and none of those are atom's. It is handed the names.
fn brief(project: Option<&str>, asked: &std::collections::BTreeMap<String, String>) {
    let named: Vec<String> = asked
        .get("name")
        .map(|names| names.lines().map(ToOwned::to_owned).collect())
        .unwrap_or_default();
    if named.is_empty() {
        return;
    }
    let me = atom::directory::mine().or_else(|| project.map(atom::identity::free_in));
    let standing = me.map(|me| atom::verbs::Standing {
        inbox: atom::directory::inbox_of(&me),
        forked: atom::directory::children(&me),
        parent: atom::directory::parent_of(&me),
        minted: std::collections::BTreeMap::new(),
        me: me.full(),
    });
    print!(
        "{}",
        atom::briefing::about(&named, &standing.unwrap_or_default())
    );
}
