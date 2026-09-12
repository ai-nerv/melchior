//! `melchior` — the agent layer, as a program.
//!
//! | | |
//! |---|---|
//! | `melchior serve` | binds this session's socket and answers for it, for as long as its parent lives |
//! | `melchior tool` | the tool a model calls, spoken over the harness's own pipe protocol |
//! | `melchior models` | what this machine could talk to, as cards |
//! | `melchior ask` | run a turn against one, streaming what it says |
//! | `melchior lua-api` | prints the client library, for redirecting into a config directory |
//!
//! Nothing here is a daemon: `serve` is a child that exits when its parent's pipe closes, and
//! `ask` is tied to the magi by [`tied`]. Up on stdin and down on stdout, one JSON object a line.

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
        Some("models") => melchior::mind::speaking::models(&flags(args)),
        // Before stdin is read: from there on the turn holds a connection with no timeout over it.
        Some("ask") => {
            tied::to_magi()?;
            melchior::mind::speaking::ask(&flags(args))
        }
        Some("needs") => melchior::mind::speaking::needs(&flags(args)),
        Some("configure") => melchior::mind::speaking::configure(&flags(args)),
        Some("auth") => signing(args),
        // `client` is the family's name for it; `lua-api` is what this program called it first.
        // Bare it is source, for redirecting into a file; framed when an encoding is asked for.
        Some("client" | "lua-api") => {
            let asked = flags(args);
            if asked.contains_key("json") || asked.contains_key("cbor") {
                let mut out = std::io::stdout().lock();
                melchior::mind::speaking::reply(
                    &mut out,
                    melchior::mind::speaking::As::asked(&asked),
                    &[melchior::CLIENT],
                )
            } else {
                print!("{}", melchior::CLIENT);
                Ok(())
            }
        }
        Some("fork") => fork(&flags(args)),
        Some("verbs") => melchior::mind::speaking::verbs(&flags(args)),
        Some("acknowledge") => melchior::mind::speaking::acknowledge(&flags(args)),
        Some("--help" | "-h" | "help") => {
            print!("{USAGE}");
            Ok(())
        }
        // A verb this program does not have is a machine's question, so it gets a machine's
        // answer: the reply shape, on stdout, at exit 0. Usage is for `--help`, where a person
        // asked; an argument parser's exit 2 cannot be told from the binary being absent.
        Some(other) => {
            let mut out = std::io::stdout().lock();
            melchior::mind::speaking::refuse(
                &mut out,
                melchior::mind::speaking::As::asked(&flags(args)),
                &format!("no such call: {other}"),
                melchior::wire::Fault::Refused,
            )
        }
        None => {
            eprint!("{USAGE}");
            std::process::exit(2);
        }
    }
}

/// What a person gets from `melchior --help`, and from a bare `melchior` on stderr.
const USAGE: &str = "\
usage: melchior serve | tool | fork | brief | models | ask | client | verbs

  serve     bind this session's socket and answer for it
  tool      the vocabulary a model calls, one exec per request
  brief     what to tell a model about the sessions a prompt named
  models    what this machine could talk to  [--json|--cbor]
  ask       run a turn, an Ask on stdin      [--json|--cbor]
  client    print the Lua client library     [--json|--cbor]
  verbs     what a session answers           [--json|--cbor]
";

mod ending;
mod tied;
mod tool;

/// What the parent tells us, one JSON object per line on stdin.
#[derive(serde::Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum Told {
    /// What this session is doing now, so `status` answers truthfully rather than plausibly.
    Doing {
        busy: bool,
        /// For how long, in seconds.
        #[serde(default)]
        working_for: u64,
        #[serde(default)]
        waiting: usize,
    },
    /// What the person said to a request this session was asked to answer.
    Answered {
        /// Which request, as [`Heard::Asked`] named it.
        id: String,
        accept: bool,
        /// Anything the accepting harness wants the adopted one to have, carried unread.
        #[serde(default)]
        handover: Option<String>,
    },
}

/// What we tell the parent, one JSON object per line on stdout.
#[derive(serde::Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum Heard {
    /// Ready: the socket is bound and this session can be reached.
    Listening {
        at: String,
        /// And as whom, since the name may have been chosen here rather than handed down.
        #[serde(rename = "as")]
        named: String,
        /// Which run this session belongs to, as written in its `.session` note.
        run: String,
    },
    /// A message arrived for this session.
    Message {
        /// Who sent it, as `project/role/id`.
        who: String,
        sort: String,
        text: String,
        /// The message it answers, when it answers one.
        about: Option<String>,
    },
    /// Who else is in this project, pushed whenever that changes.
    Around {
        /// Every session listening, this one included.
        agents: Vec<Peer>,
    },
    /// Somebody with the right to stop this session did.
    Stopped,
    /// Another session is asking to become this one's child, and a person has to answer.
    Asked {
        id: String,
        /// Who is asking, as `project/role/id`.
        who: String,
        why: String,
    },
    /// A session this one asked to be taken on by has accepted. Carries what that session lent
    /// it, which goes to the harness and never into a transcript a model can read.
    Adopted {
        /// Who took this session on, as `project/role/id`.
        by: String,
        handover: Option<String>,
    },
}

/// One session listening in this project, as a harness needs it.
#[derive(serde::Serialize, Clone, PartialEq, Eq)]
struct Peer {
    id: String,
    /// What it says it is for, one word. `main` for an agent that never said.
    role: String,
    /// Where its harness draws it, or `null` for one that published no screen — read off the note
    /// left at announce time, see [`directory::screens`](melchior::directory::screens).
    ui: Option<String>,
    /// Who started it — the id in its `.parent` note — or `null` for a main, so a harness can draw
    /// the run as the tree it is.
    parent: Option<String>,
}

/// Everyone listening in `project`, as they go on the pipe.
fn around(project: &str) -> Vec<Peer> {
    melchior::directory::listening(project)
        .into_iter()
        .map(|id| Peer {
            role: melchior::directory::roles::role_in(project, &id)
                .map_or_else(|| melchior::directory::roles::MAIN.to_owned(), |it| it.name),
            ui: melchior::directory::screens::ui_in(project, &id)
                .map(|at| at.display().to_string()),
            parent: melchior::directory::parent_note(project, &id),
            id,
        })
        .collect()
}

/// `melchior fork` — a name and a secret for a session this one is about to start, printed as
/// JSON. `--role` and `--role-description` say what the child is for at birth, so it is never on
/// the roster described as `main` while a coordinator is routing by description.
fn fork(asked: &std::collections::BTreeMap<String, String>) -> std::io::Result<()> {
    let Some(me) = melchior::directory::mine() else {
        return Err(std::io::Error::other(
            "`fork` is asked by a session, of itself: nothing here says which session this is",
        ));
    };
    let mut held = melchior::directory::dial(&me, &me)?;
    let naming = match asked.get("role") {
        None => Vec::new(),
        Some(name) => vec![
            serde_json::Value::String(name.clone()),
            asked
                .get("role-description")
                .map_or(serde_json::Value::Null, |said| {
                    serde_json::Value::String(said.clone())
                }),
        ],
    };
    let reply = held.call("mint", naming)?;
    if !reply.ok {
        return Err(std::io::Error::other(
            reply.error.unwrap_or_else(|| "mint refused".to_owned()),
        ));
    }
    let Some(child) = reply.result.first() else {
        return Err(std::io::Error::other("mint answered nothing"));
    };
    println!("{child}");
    Ok(())
}

/// Bind this session's socket and answer for it until the parent lets go.
///
/// `--project` when nothing in the environment says which session this is. `--role` and
/// `--role-description` are one source taken whole, outranking `MAGI_MELCHIOR_ROLE` and the
/// config. `--ui <path>` is where the harness draws this session, written to a note beside the
/// socket — see [`directory::screens`](melchior::directory::screens).
fn serve(asked: &std::collections::BTreeMap<String, String>) -> std::io::Result<()> {
    tied::to_magi()?;

    let me = match (melchior::directory::mine(), asked.get("project")) {
        (Some(me), _) => me,
        (None, Some(project)) => melchior::directory::free_in(project),
        (None, None) => {
            eprintln!(
                "melchior serve: no session to be. Pass --project, or set {} and {} — one of them \
                 has to say which session this is answering for, and nothing on disk can be \
                 asked instead.",
                melchior::inherited::PROJECT,
                melchior::inherited::ID
            );
            std::process::exit(2);
        }
    };

    // Flag, then environment, then config, then `main`, and whichever speaks first is taken
    // whole: a name from one and a sentence from another describes a role nobody declared.
    let role = melchior::directory::roles::resolve(
        asked
            .get("role")
            .map(|written| {
                melchior::directory::roles::Role::new(
                    written,
                    asked.get("role-description").map(String::as_str),
                )
            })
            .or_else(|| {
                asked
                    .get("role-description")
                    .map(|said| melchior::directory::roles::Role::new("", Some(said)))
            }),
        melchior::directory::roles::inherited(),
        melchior::mind::setup::assigned("role")
            .as_ref()
            .and_then(serde_json::Value::as_str)
            .and_then(melchior::directory::roles::Role::read),
    );
    let me = melchior::identity::Identity {
        role: role.name.clone(),
        ..me
    };
    let ui = asked.get("ui").map(std::path::PathBuf::from);

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        // Before anything is bound or written down, so a signal arriving early finds nothing.
        let mut ending = ending::Ending::watching()?;
        let (about_tx, about_rx) = tokio::sync::watch::channel(melchior::answering::About {
            me: me.clone(),
            parent: melchior::directory::parent_of(&me),
            token: melchior::directory::token(),
            busy: false,
            working_for: 0,
            inbox: Vec::new(),
            minted: std::collections::BTreeMap::new(),
            adopted_token: None,
        });
        let (arrived_tx, mut arrived) = tokio::sync::mpsc::channel(64);
        let (asked_tx, mut asked) = tokio::sync::mpsc::channel(16);
        let (adopted_tx, mut adopted) = tokio::sync::mpsc::channel(4);
        let (stopped_tx, mut stopped) = tokio::sync::mpsc::channel(1);
        // Children this session named. The secret a `stop` has to quote back is held here only.
        let (minted_tx, mut minted) = tokio::sync::mpsc::channel::<(String, String)>(4);
        let (named_tx, mut named) =
            tokio::sync::mpsc::channel::<melchior::directory::roles::Role>(4);

        // The note beside the socket, so the tree can be read off the directory. Refused rather
        // than served when it will not land: without it a session comes up as a main, with a
        // main's reach.
        if let Err(why) = melchior::directory::announce(&me, &role, ui.as_deref()) {
            eprintln!("melchior serve: {why}");
            std::process::exit(1);
        }
        let at = melchior::directory::listening_at(&me);

        // Bound here, and only then announced: saying "listening" first announces a future.
        let listener = match melchior::serving::listening_on(&at).await {
            Ok(listener) => listener,
            Err(why) => {
                eprintln!("melchior serve: {}: {why}", at.display());
                std::process::exit(1);
            }
        };
        say(&Heard::Listening {
            at: at.display().to_string(),
            named: me.full(),
            // Read back from the note `announce` just wrote, so the directory holds one answer.
            run: melchior::directory::sessions::session_of(&me).unwrap_or_else(|| me.id.clone()),
        });
        tokio::spawn(async move {
            let _ = melchior::serving::accept(
                listener,
                melchior::serving::Serving {
                    about: about_rx,
                    arrived: arrived_tx,
                    asked: asked_tx,
                    adopted: adopted_tx,
                    stopped: stopped_tx,
                    minted: minted_tx.clone(),
                    named: named_tx.clone(),
                },
            )
            .await;
        });

        // Who else is in the project, on a slow tick. There is no event to hook: the directory is
        // the registry, and a session appears in it by binding a socket.
        let mut listed: Vec<Peer> = Vec::new();
        let mut sweep = tokio::time::interval(std::time::Duration::from_secs(2));
        sweep.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        // The parent's side of the pipe, read on a thread because it is a blocking stdin.
        let (told_tx, mut told) = tokio::sync::mpsc::channel::<Told>(16);
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines().map_while(Result::ok) {
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<Told>(&line) {
                    // A line we cannot read is the parent's bug, not a reason to stop answering.
                    Err(why) => eprintln!("melchior serve: {why}"),
                    Ok(told) => {
                        if told_tx.blocking_send(told).is_err() {
                            return;
                        }
                    }
                }
            }
        });

        let mut inbox: Vec<melchior::wire::Message> = Vec::new();
        // Requests not yet answered. Held here because the answer arrives on the pipe, long after
        // the connection that carried the question has closed.
        let mut pending: Vec<melchior::wire::Request> = Vec::new();
        loop {
            tokio::select! {
                Some(message) = arrived.recv() => {
                    say(&Heard::Message {
                        who: message.from.clone(),
                        sort: name_of(message.sort),
                        text: message.text.clone(),
                        about: message.about.clone(),
                    });
                    // Bounded, so a handoff that has come back round runs out of somewhere to go.
                    melchior::answering::keeping::kept(&mut inbox, message);
                    about_tx.send_modify(|about| about.inbox.clone_from(&inbox));
                }
                // Held until the person answers: the call that carried it was already replied to.
                Some(request) = asked.recv() => {
                    say(&Heard::Asked {
                        id: request.id.clone(),
                        who: request.from.clone(),
                        why: request.why.clone(),
                    });
                    pending.push(request);
                }
                // Straight up the pipe, never into the inbox: a handover is not a model's to read.
                Some((by, handover, secret)) = adopted.recv() => {
                    // The stop secret stays here for the loop; only `by`/`handover` reach the harness.
                    if let Some(secret) = secret {
                        about_tx.send_modify(|about| about.adopted_token = Some(secret));
                    }
                    say(&Heard::Adopted { by, handover });
                }
                // A child this session named. The secret is held here and said nowhere else.
                Some((id, token)) = minted.recv() => {
                    about_tx.send_modify(|about| {
                        about.minted.insert(id, token);
                    });
                }
                // The note first, then the copy this session answers with: peers route by the
                // note, and a peer that saw the old one would route by it.
                Some(role) = named.recv() => {
                    match melchior::directory::roles::given(&me.project, &me.id, &role) {
                        Ok(()) => about_tx.send_modify(|about| about.me.role.clone_from(&role.name)),
                        Err(why) => eprintln!("melchior: {} is still {}: {why}", me.id, me.role),
                    }
                }
                // `None` is the parent letting go. Matched rather than left to `else`: a
                // `select!` arm whose pattern does not match is disabled, not taken.
                told = told.recv() => match told {
                    None => break,
                    Some(Told::Doing { busy, working_for, waiting }) => {
                        about_tx.send_modify(|about| {
                            about.busy = busy;
                            about.working_for = working_for;
                            // What the harness still holds unread, which is not the same as what
                            // has arrived here.
                            about.inbox.truncate(waiting.min(about.inbox.len()));
                        });
                    }
                    // Written by the side that consented: an asker writing its own note would be
                    // a session appointing its own parent.
                    Some(Told::Answered { id, accept, handover }) => {
                        let Some(at) = pending.iter().position(|held| held.id == id) else {
                            continue;
                        };
                        let request = pending.remove(at);
                        // The acceptance is downgraded rather than reported as one when the note
                        // will not write.
                        let mut accept = accept;
                        // The secret that lets this session stop the one it takes on, minted only
                        // when the note wrote, held beside `mint`'s and handed over on the call.
                        let mut secret: Option<String> = None;
                        if accept && let Some(them) = melchior::identity::Identity::read(&request.from) {
                            if let Err(why) = melchior::directory::adopted(&them, &me.id) {
                                eprintln!("melchior: {} was not taken on: {why}", request.from);
                                accept = false;
                            } else {
                                let minted = melchior::identity::secret();
                                about_tx.send_modify(|about| {
                                    about.minted.insert(them.id.clone(), minted.clone());
                                });
                                secret = Some(minted);
                            }
                        }
                        // Told either way: a silence the asker could not tell from a refusal
                        // would leave it waiting for good.
                        melchior::directory::answer_request(
                            &request.from,
                            &me,
                            accept,
                            handover.as_deref(),
                            secret.as_deref(),
                        );
                    }
                },
                Some(()) = stopped.recv() => {
                    say(&Heard::Stopped);
                    break;
                }
                _ = sweep.tick() => {
                    // Re-read on the tick: being adopted happens to this session from outside it,
                    // and no variable can be set on a process already running.
                    let mine = melchior::directory::parent_of(&me);
                    if mine != about_tx.borrow().parent {
                        about_tx.send_modify(|about| about.parent.clone_from(&mine));
                    }
                    // Compared whole: a peer that changed role or published a screen is a change.
                    let now = around(&me.project);
                    if now != listed {
                        listed = now;
                        say(&Heard::Around { agents: listed.clone() });
                    }
                }
                () = ending.came() => break,
                else => break,
            }
        }
        melchior::directory::forget(&me);
        let _ = std::fs::remove_file(&at);
        // And the directory itself, which refuses while anybody else is still in the project.
        melchior::directory::leave(&me.project);
        Ok(())
    })
}

fn name_of(sort: melchior::wire::Sort) -> String {
    serde_json::to_value(sort)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| "note".to_owned())
}

/// One line to the parent, flushed every time because the parent is waiting on it.
fn say(heard: &Heard) {
    let Ok(line) = serde_json::to_string(heard) else {
        return;
    };
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

/// `--name value` pairs, as a caller wrote them. Repeats accumulate under one key separated by
/// newlines, because `--name a --name b` is how a shell says "these several".
fn flags(args: impl Iterator<Item = String>) -> std::collections::BTreeMap<String, String> {
    let mut out: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    let mut args = args.peekable();
    while let Some(flag) = args.next() {
        let Some(key) = flag.strip_prefix("--") else {
            continue;
        };
        let (key, value) = match key.split_once('=') {
            Some((key, value)) => (key.to_owned(), value.to_owned()),
            None => {
                // A flag with another flag after it, or with nothing after it, is a bare yes.
                let takes = args.peek().is_some_and(|next| !next.starts_with("--"));
                let value = if takes {
                    args.next().unwrap_or_default()
                } else {
                    "yes".to_owned()
                };
                (key.to_owned(), value)
            }
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

/// What a harness should put in front of a model about the sessions a prompt named, printed.
fn brief(project: Option<&str>, asked: &std::collections::BTreeMap<String, String>) {
    let named: Vec<String> = asked
        .get("name")
        .map(|names| names.lines().map(ToOwned::to_owned).collect())
        .unwrap_or_default();
    if named.is_empty() {
        return;
    }
    let me = melchior::directory::mine().or_else(|| project.map(melchior::directory::free_in));
    let standing = me.map(|me| melchior::verbs::Standing {
        inbox: melchior::directory::inbox_of(&me),
        forked: melchior::directory::children(&me),
        parent: melchior::directory::parent_of(&me),
        minted: std::collections::BTreeMap::new(),
        me: me.full(),
    });
    print!(
        "{}",
        melchior::briefing::about(&named, &standing.unwrap_or_default())
    );
}

/// `melchior auth login|logout|status`, on a runtime of its own for the loopback redirect.
fn signing(mut args: impl Iterator<Item = String>) -> std::io::Result<()> {
    let what = args.next().unwrap_or_else(|| "status".to_owned());
    let who = args.next().unwrap_or_default();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let done = match what.as_str() {
        "login" => runtime.block_on(melchior::mind::signing::login(&who)),
        "logout" => melchior::mind::signing::logout(&who),
        "status" => melchior::mind::signing::status(),
        other => {
            eprintln!("melchior auth: no such command: {other}");
            eprintln!("usage: melchior auth login <provider> | logout <provider> | status");
            std::process::exit(2);
        }
    };
    if let Err(why) = done {
        eprintln!("melchior auth: {why}");
        std::process::exit(1);
    }
    Ok(())
}

#[cfg(test)]
mod flag_tests {
    use super::flags;

    fn parse(args: &[&str]) -> std::collections::BTreeMap<String, String> {
        flags(args.iter().map(|a| (*a).to_owned()))
    }

    #[test]
    fn a_flag_with_a_value_takes_it() {
        assert_eq!(
            parse(&["--project", "magi"]).get("project"),
            Some(&"magi".to_owned())
        );
        assert_eq!(
            parse(&["--project=magi"]).get("project"),
            Some(&"magi".to_owned())
        );
    }

    #[test]
    fn a_bare_flag_is_a_yes_rather_than_nothing() {
        assert_eq!(parse(&["--cbor"]).get("cbor"), Some(&"yes".to_owned()));
    }

    #[test]
    fn a_bare_flag_does_not_swallow_the_one_after_it() {
        let asked = parse(&["--ready", "--cbor"]);
        assert_eq!(asked.get("ready"), Some(&"yes".to_owned()));
        assert_eq!(asked.get("cbor"), Some(&"yes".to_owned()), "{asked:?}");
    }

    #[test]
    fn repeats_still_accumulate() {
        assert_eq!(
            parse(&["--name", "a", "--name", "b"]).get("name"),
            Some(&"a\nb".to_owned())
        );
    }
}

/// The pipe is a wire between two repositories, so its spellings are pinned here.
#[cfg(test)]
#[path = "main/pipe.rs"]
mod pipe;
