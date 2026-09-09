//! Naming another session, and where it lives.
//!
//! # The layout
//!
//! ```text
//! $XDG_RUNTIME_DIR/melchior/
//!   myproject/             <- one directory per project
//!     alpha-rho            <- a socket, named by the id and nothing else
//!     iota-mu
//!     iota-mu.parent       <- "alpha-rho": who started it
//!     iota-mu.session      <- "alpha-rho": which run it belongs to
//!     iota-mu.role         <- "reviewer", and a line saying what that means
//!     iota-mu.sent         <- what it has sent lately, so a loop runs out of window
//!     .claims/             <- one file per piece of work somebody has taken
//!       the-parser
//!   other-project/
//!     beta-nu
//! ```
//!
//! Every name beside a socket carries a dot, and the claims directory begins with one. That is
//! not tidiness: [`listening`] keeps the entries with no dot in them and *dials each as a socket*,
//! so anything here that read as an id would be offered to a model as an agent and then swept as
//! a corpse when the dial failed.
//!
//! A project directory, and inside it a socket per session named by its id. No role in the
//! path, because a role is not part of a name — see [`crate::identity`]. Sessions in different
//! projects are not refused each other, they are *not in each other's directory*, which is the
//! project wall from [`policy`] enforced by the filesystem rather than by a check somebody could
//! forget to write.
//!
//! The `.parent` file beside a subagent's socket says who started it. That is what makes the
//! tree legible: a session that finds `iota-mu` in the directory can tell it is behind
//! `alpha-rho`'s door without asking it, and without trusting what it would have said.
//!
//! # What a caller's name is worth
//!
//! Every call says who is making it, and for `ask` and `tell` that claim is taken at face
//! value. It has to be: everything here runs as one user in one directory, so any process that
//! can open the socket could open it claiming anything, and a check that cannot be enforced is
//! worse than none — it reads like security to whoever comes along next.
//!
//! `stop` is the exception, because it is the one act the far end cannot decline. It carries the
//! secret handed to the session in [`TOKEN`] when it was started, which only whoever started it
//! ever held. A session nobody started holds none, so nothing can stop it.

pub mod claims;
pub mod roles;
pub mod screens;
pub mod sending;
pub mod sessions;

use crate::identity::Identity;
use crate::inherited::{ID, PARENT, PROJECT, TOKEN, said};

use crate::policy::{self, Whom};
use std::path::{Path, PathBuf};

/// What the model calls the tool that reaches other instances.
///
/// Named once, because it is said in three places: the tool registers under it, the briefing
/// tells the model to use it, and its own help repeats it.
pub const TOOL: &str = "agent";

/// Who started this session, if anybody did.
///
/// `None` for one somebody started at a terminal, which is most of them — and a session with no
/// parent is a *main*, which is the whole of what that word means here.
#[must_use]
pub fn parent() -> Option<String> {
    said(PARENT)
}

/// Who `me` answers to, as everybody else can see it.
///
/// **The note first, the environment second**, and the order is the whole point. A session
/// spawned as a child learns its parent from the environment and writes the note from it; a
/// session *adopted* while it runs is given a note by whoever accepted it, and there is no
/// environment to change — a variable cannot be set on a process that is already running.
///
/// Reading only the environment left a session that had been adopted still calling itself a
/// main. Every other session read the note and saw a child, so the tree disagreed with itself
/// depending on who was asked — and the rule that stops a session having two parents, which
/// tests exactly this, could be walked straight through.
#[must_use]
pub fn parent_of(me: &Identity) -> Option<String> {
    std::fs::read_to_string(kin_at(me))
        .ok()
        .map(|said| said.trim().to_owned())
        .filter(|said| !said.is_empty())
        .or_else(parent)
}

/// The secret this session was started with, if it was started by another.
#[must_use]
pub fn token() -> Option<String> {
    said(TOKEN)
}

/// Which session a spawned process belongs to, from its environment.
///
/// `None` outside a session — `melchior tool` run by hand from a shell, which should say so rather
/// than invent a name and send messages signed with it.
#[must_use]
pub fn mine() -> Option<Identity> {
    let project = said(PROJECT)?;
    let id = said(ID)?;
    // The one part with a sensible default. Project and id place a session and a wrong guess at
    // either would sign messages as somebody else; a role only says what it is for.
    //
    // The note first, the environment second, for the same reason [`parent_of`] reads its note
    // first: `assign` and `role` change what an agent is for *while it runs*, and no variable
    // can be set on a process already started. Every process of this session — the socket, and
    // a tool spawned per call — would otherwise go on signing as what it was spawned as.
    let role = roles::role_of(&Identity {
        project: project.clone(),
        role: roles::MAIN.to_owned(),
        id: id.clone(),
    })
    .name;
    Some(Identity { project, role, id })
}

/// What `me` started, read off the project directory.
///
/// Not from anything the session remembers: a session that restarted forgot, and a child that
/// declined to leave its note would have made itself unstoppable by forgetting who its parent
/// was. The directory is the one place both facts survive.
///
/// Ids, not whole names. The directory knows where a session is, not what it calls itself: a
/// role is not on disk, so a full name built from here would be a name with a guess in it.
#[must_use]
pub fn children(me: &Identity) -> Vec<String> {
    listening(&me.project)
        .into_iter()
        .filter(|id| *id != me.id)
        .filter(|id| whom(&me.project, id).parent.as_deref() == Some(me.id.as_str()))
        .collect()
}

/// What has arrived for `me`, asked of `me`'s own socket.
///
/// A separate process cannot see the UI's memory, and the inbox lives there. So it asks — which
/// works because a session is allowed to ask itself anything, and because the answer then comes
/// from the one copy that is actually current rather than from a snapshot taken at spawn.
///
/// Empty when nothing answers, which is the ordinary case for a peer started outside a session.
#[must_use]
pub fn inbox_of(me: &Identity) -> Vec<crate::wire::Message> {
    let Ok(mut held) = dial(me, me) else {
        return Vec::new();
    };
    let Ok(reply) = held.call("inbox", Vec::new()) else {
        return Vec::new();
    };
    reply
        .result
        .first()
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or_default()
}

/// The secrets this session minted for the children it started, by id.
///
/// Asked of its own socket, for the same reason [`inbox_of`] is: the session holds them and this
/// process does not exist between calls. Read only by the session's own tool, and never written
/// to the directory — a sibling that could read one off disk would have authority over a session
/// it did not start.
///
/// Empty when nothing answers, which means `stop` is refused with "this session did not start
/// it". That is the right answer when the session cannot be reached: refusing to end something
/// on a guess.
#[must_use]
pub fn minted_by(me: &Identity) -> std::collections::BTreeMap<String, String> {
    let Ok(mut held) = dial(me, me) else {
        return std::collections::BTreeMap::new();
    };
    let Ok(reply) = held.call("minted", Vec::new()) else {
        return std::collections::BTreeMap::new();
    };
    reply
        .result
        .first()
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or_default()
}

/// Another instance, by name.
///
/// The same three parts a session wears on its status line, and the short forms fill in from
/// whoever is asking: `$iota-mu` is one in this project, `$review/iota-mu` says what it is for,
/// `$magi/review/iota-mu` says everything.
///
/// The last of those parses and then loses — [`policy`] refuses anything outside the asker's own
/// project, and the directory it would have to be found in is not one this session lists. It
/// reads rather than being rejected as a typo so the refusal can say *why*.
///
/// **The id is the part that finds it.** A role is what a session says about itself, so a role
/// in an address is a description, not a lookup: it is filled in and carried along, and never
/// consulted to work out which socket is meant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Address {
    /// Which project, or `None` to mean the asker's own.
    pub project: Option<String>,
    /// What it is for, or `None` to mean the asker's own.
    pub role: Option<String>,
    /// Which instance. Always given: this is the part that names one.
    pub id: String,
}

impl Address {
    /// Read `$iota-mu`, `$review/iota-mu` or `$magi/review/iota-mu`.
    ///
    /// The sigil is optional so this reads what the trigger hands over as well as what somebody
    /// wrote. Read from the right, because the id is the part that is always there and the rest
    /// fills in from the outside: `a/b/c/d` is not a deeper address, it is a typo.
    #[must_use]
    pub fn read(written: &str) -> Option<Self> {
        let body = written.strip_prefix('$').unwrap_or(written);
        let parts: Vec<&str> = body.split('/').filter(|part| !part.is_empty()).collect();
        match parts.as_slice() {
            [id] => Some(Self {
                project: None,
                role: None,
                id: (*id).to_owned(),
            }),
            [role, id] => Some(Self {
                project: None,
                role: Some((*role).to_owned()),
                id: (*id).to_owned(),
            }),
            [project, role, id] => Some(Self {
                project: Some((*project).to_owned()),
                role: Some((*role).to_owned()),
                id: (*id).to_owned(),
            }),
            _ => None,
        }
    }

    /// The address as somebody would type it.
    #[must_use]
    pub fn written(&self) -> String {
        let mut out = String::from("$");
        if let Some(project) = &self.project {
            out.push_str(project);
            out.push('/');
        }
        if let Some(role) = &self.role {
            out.push_str(role);
            out.push('/');
        }
        out.push_str(&self.id);
        out
    }

    /// The full name, filling the gaps in from whoever is asking.
    #[must_use]
    pub fn against(&self, asker: &Identity) -> Identity {
        Identity {
            project: self
                .project
                .clone()
                .unwrap_or_else(|| asker.project.clone()),
            role: self.role.clone().unwrap_or_else(|| asker.role.clone()),
            id: self.id.clone(),
        }
    }
}

/// The directory a project's sockets live in.
#[must_use]
pub fn home(project: &str) -> PathBuf {
    runtime().join(crate::NAME).join(safe(project))
}

/// Where a session's socket is, by the only two parts of a name that place it.
///
/// No role. An id is already unique inside a project, so a role in the path would be a second
/// key for the same door -- and a session that changed what it was for would move.
#[must_use]
pub fn socket(project: &str, id: &str) -> PathBuf {
    home(project).join(safe(id))
}

/// Where a socket for `me` is put, so an instance can be reached by name.
#[must_use]
pub fn listening_at(me: &Identity) -> PathBuf {
    socket(&me.project, &me.id)
}

/// Where the note saying who started `me` is put.
#[must_use]
pub fn kin_at(me: &Identity) -> PathBuf {
    home(&me.project).join(format!("{}.parent", safe(&me.id)))
}

/// The directory sockets live in.
fn runtime() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

/// Flatten one name into one path segment.
///
/// A project is the working directory's name and a directory can be called anything, including
/// `..`. Anything that is not a letter, digit, dash or underscore becomes a dash, so a project
/// called `../../etc` cannot name a directory outside the one this chose.
fn safe(name: &str) -> String {
    let flattened: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    // A name of nothing would name the parent directory itself.
    if flattened.is_empty() {
        "-".to_owned()
    } else {
        flattened
    }
}

/// Leave the notes saying who started this session, which run it is part of, and what it is for.
///
/// A main writes no parent, and that absence is what says it is one. It writes a run all the
/// same — see [`sessions::began`] for why the two are not symmetrical.
///
/// The role is handed in rather than taken off `me`. An [`Identity`] carries only the name, and
/// the description is half the record: resolving it is the caller's, because the sources are a
/// flag, an environment variable and a config file, and this module knows about none of them.
///
/// `ui` is where the harness draws this agent, which melchior cannot work out for itself and
/// which nothing else in this directory can be asked for — see [`screens`]. `None` for a
/// session with no screen to offer, and that is what a bare `melchior serve` is.
///
/// # Errors
/// When a note cannot be written, naming the path and what the operating system said.
///
/// **This used to be `let _ = write(…)` three times over.** When `/tmp` hit its quota, every
/// session came up as a *main*: no `.parent`, no `.session`, a crew of one, and every relation
/// in [`policy`] reading "another instance's main" — because the notes those answers are read
/// off had never been written and nothing anywhere said so. The symptom is indistinguishable
/// from a genuine bug in the session model, and it was chased as one.
///
/// A session that cannot write these is not a session with a cosmetic fault. The `.parent` note
/// is what makes a subagent a subagent, so a child that could not write one is a main as far as
/// every peer is concerned, with a main's reach — the failure grants authority. The caller's
/// business is to refuse to come up rather than to serve under a name it cannot back.
pub fn announce(me: &Identity, role: &roles::Role, ui: Option<&Path>) -> Result<(), String> {
    sessions::began(me)?;
    roles::began(me, role)?;
    screens::began(me, ui)?;
    let Some(parent) = parent() else {
        return Ok(());
    };
    wrote(&kin_at(me), &parent)
}

/// Write one note beside a socket, and say what went wrong if it could not be.
///
/// The one place the notes are written, so there is one answer to "why is this session
/// misdescribed" rather than one per file. The path is in the message because the two ways this
/// fails — a full filesystem and a runtime directory that is not writable — are told apart by
/// looking at it.
pub(crate) fn wrote(path: &Path, said: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|why| format!("{}: {why}", dir.display()))?;
    }
    std::fs::write(path, said).map_err(|why| format!("{}: {why}", path.display()))
}

/// Take the notes back down.
pub fn forget(me: &Identity) {
    let _ = std::fs::remove_file(kin_at(me));
    sessions::ended(me);
    roles::ended(me);
    screens::ended(me);
    sending::ended(me);
    claims::forget_in(&me.project, &me.id);
}

/// Record that `them` now answers to `parent`.
///
/// Written by the session that *consented*, never by the one that asked. The note is what every
/// other session reads to work out the tree, so a session that could write its own would be
/// appointing its own parent — which is the whole thing the handshake exists to prevent.
///
/// It says nothing about what the child may then do. That is handed over separately, by the
/// parent's harness to the child's, so a forged note buys the forger a word and no authority.
///
/// `parent` is an **id**, not a full name: it is what every reader of this note compares against
/// — [`children`] and [`policy::between`] both test it against a bare id — and a full name here
/// matches nothing. Written that way once, the adopted session read as a *cousin* to the very
/// session that had just accepted it, which was then refused for reaching another instance's
/// subagent.
///
/// # Errors
/// When the note cannot be written. Adoption that failed silently is worse than one that was
/// refused: the person at the keyboard said yes, both sides were told it happened, and the
/// directory every other agent reads goes on describing the child as somebody's main.
pub fn adopted(them: &Identity, parent: &str) -> Result<(), String> {
    wrote(&kin_at(them), parent)
}

/// Tell whoever asked what was decided.
///
/// Sent as an ordinary message, because that is what it is once the decision is made — and it
/// goes into their inbox where a model will read it. Told either way: a refusal that arrived as
/// silence is one a session cannot tell from an answer that never came, and it would wait for
/// good.
/// `handover` is whatever the accepting harness wants the adopted one to have, carried unread.
/// It goes by a separate call from the message, because the message ends up in a transcript a
/// model reads and this must not: what a harness lends a session it has taken on is not something
/// a model should be able to read, reason about, or ask for more of.
pub fn answer_request(who: &str, me: &Identity, accept: bool, handover: Option<&str>) {
    let Some(them) = Identity::read(who) else {
        return;
    };
    let Ok(mut held) = dial(&them, me) else {
        return;
    };
    let said = if accept {
        format!("`{}` accepted: it is this session's parent now.", me.full())
    } else {
        format!("`{}` declined to take this session on.", me.full())
    };
    let _ = held.call(
        "tell",
        vec![
            serde_json::Value::String(said),
            serde_json::Value::String("answer".to_owned()),
        ],
    );
    if accept {
        // Second, and only on a yes. The order matters no more than that both arrive; what
        // matters is that they are two calls, so the payload never lands in an inbox.
        let _ = held.call(
            "adopted",
            vec![
                serde_json::Value::String(me.full()),
                handover.map_or(serde_json::Value::Null, |said| {
                    serde_json::Value::String(said.to_owned())
                }),
            ],
        );
    }
}

/// A name nothing in `project` is already listening under.
///
/// The two halves: [`crate::identity::free_of`] knows how to pick a name, and this module knows
/// which names are in use. They were one function in `identity`, which is what made that module
/// depend on this one — and this one already depends on it, for the type.
#[must_use]
pub fn free_in(project: &str) -> Identity {
    crate::identity::free_of(project, &listening(project))
}

/// Open a connection to the session named `them`, as `me`.
///
/// Here rather than on [`Held`](crate::asking::Held), which is where it was. A connection knows
/// how to speak to a socket; *which* socket a name means is this module's whole subject, and
/// having the constructor resolve it made `asking` depend on `directory` and `directory` depend
/// on `asking` — one of three cycles among these four modules, and the one that made the other
/// two hard to see.
///
/// # Errors
/// When nothing is listening under that name, which is the ordinary answer for a session that
/// has ended: the socket file outlives the process that made it.
pub fn dial(them: &Identity, me: &Identity) -> std::io::Result<crate::asking::Held> {
    crate::asking::Held::at(&listening_at(them), me)
}

/// What is known about a session in `project`, read off the directory.
///
/// Read rather than asked, so a session cannot describe its own place in the tree. The answer is
/// the same whether it is running, busy, or wedged.
#[must_use]
pub fn whom(project: &str, id: &str) -> Whom {
    let kin = home(project).join(format!("{}.parent", safe(id)));
    Whom {
        project: project.to_owned(),
        id: id.to_owned(),
        parent: std::fs::read_to_string(kin)
            .ok()
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty()),
        session: sessions::session_in(project, id),
    }
}

/// Every session currently listening in `project`.
///
/// Read from the directory rather than from a registry somebody has to keep up to date: a
/// process that died did not get to remove itself from a list, and a socket file that nothing
/// answers is discovered on the first call rather than trusted forever.
///
/// Only this project's, because there is no argument for any other and no way to ask for one.
#[must_use]
pub fn listening(project: &str) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(home(project)) else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        // The `.parent` notes sit beside the sockets. An id is two Greek words and a dash, so
        // anything with a dot in it is not one.
        .filter(|name| !name.contains('.') && !name.is_empty())
        .filter(|id| {
            // Dialled, not merely found. This is what the paragraph above promises and what the
            // code did not do: a name was listed because a file was there, so every session that
            // crashed — and every one from a build that named its socket differently — stayed in
            // the roster for good. A model was then offered names nobody answered, and found out
            // one failed send at a time.
            if answers(&socket(project, id)) {
                return true;
            }
            forget_id(project, id);
            false
        })
        .collect();
    out.sort();
    out
}

/// Whether anything is serving at `path`.
///
/// Connecting is the whole test, and the only one that cannot be raced: a path is not a session,
/// and the file outlives the process that made it. A listener answers from the moment it is
/// bound — the kernel queues the connection whether or not anybody has called accept — so this
/// is never a false negative against a session that is merely busy.
#[must_use]
pub fn answers(path: &Path) -> bool {
    std::os::unix::net::UnixStream::connect(path).is_ok()
}

/// Take a dead session out of the directory: its socket, and the note beside it.
///
/// Swept where the roster is read, because the sessions needing a sweep are exactly the ones
/// that never got to run their own exit path. Anything already gone is not an error — two
/// sessions may notice the same corpse at once.
fn forget_id(project: &str, id: &str) {
    let _ = std::fs::remove_file(socket(project, id));
    let _ = std::fs::remove_file(home(project).join(format!("{}.parent", safe(id))));
    sessions::forget_in(project, id);
    roles::forget_in(project, id);
    screens::forget_in(project, id);
    sending::forget_in(project, id);
    // And what it had taken. A process that died did not get to let go of its work, and a claim
    // nobody can be asked about is a piece of work nobody will ever do again.
    claims::forget_in(project, id);
}

/// Last one out turns off the lights: drop the project's directory if nothing is left in it.
///
/// `remove_dir` refuses a directory that still holds something, which is exactly the test — no
/// listing, and no race against a session binding as this one leaves. Without it a machine
/// collects an empty directory per project, which is how the runtime directory filled up.
pub fn leave(project: &str) {
    // The claims directory first, and by the same test: `remove_dir` refuses one that still holds
    // a claim, so a run somebody is still working in keeps its directory. Without this the empty
    // one left inside would keep the project's alive for good.
    claims::leave(project);
    let _ = std::fs::remove_dir(home(project));
}

/// Everyone `me` may actually reach, with how they stand to it.
///
/// The list the model is shown. Filtered here rather than at the point of calling, so a session
/// is never told about something it would then be refused — which reads as a broken tool.
#[must_use]
pub fn reachable(me: &Whom) -> Vec<(Whom, policy::Relation)> {
    listening(&me.project)
        .into_iter()
        .filter(|id| *id != me.id)
        .map(|id| {
            let them = whom(&me.project, &id);
            let relation = policy::between(me, &them);
            (them, relation)
        })
        .filter(|(_, relation)| policy::may(me, *relation, policy::Reach::Ask))
        .collect()
}

/// Whether a path is one this process may listen on.
///
/// Belt and braces against the flattening above: a socket path is built from a name that came off a wire,
/// and a name is not a promise. Two levels below the runtime root and no more, so neither half
/// can climb.
#[must_use]
pub fn inside(path: &Path) -> bool {
    let root = runtime().join(crate::NAME);
    path.parent()
        .and_then(Path::parent)
        .is_some_and(|grand| grand == root)
        && path
            .components()
            .all(|part| part.as_os_str() != std::ffi::OsStr::new(".."))
}

/// A name parses, resolves, and cannot escape the project it belongs to.
#[cfg(test)]
mod tests {
    use super::*;

    fn asker() -> Identity {
        Identity {
            project: "magi".to_owned(),
            role: "main".to_owned(),
            id: "alpha-rho".to_owned(),
        }
    }

    #[test]
    fn a_bare_name_is_somebody_in_this_project() {
        let address = Address::read("$iota-mu").expect("an address");
        assert_eq!(address.id, "iota-mu");
        assert_eq!(address.against(&asker()).full(), "magi/main/iota-mu");
    }

    #[test]
    fn a_two_part_name_gives_the_role() {
        let address = Address::read("$review/iota-mu").expect("an address");
        assert_eq!(address.role.as_deref(), Some("review"));
        assert_eq!(address.project, None, "which fills in from the asker");
        assert_eq!(address.against(&asker()).full(), "magi/review/iota-mu");
    }

    #[test]
    fn a_three_part_name_gives_everything() {
        let address = Address::read("$other/review/eta-nu").expect("an address");
        assert_eq!(address.against(&asker()).full(), "other/review/eta-nu");
    }

    #[test]
    fn the_role_in_an_address_never_decides_which_socket_is_meant() {
        // A role is what a session says it is for. Two addresses that differ only there are the
        // same session, and a lookup that read the role would have made them two.
        let one = Address::read("$review/iota-mu").expect("an address");
        let other = Address::read("$scratch/iota-mu").expect("an address");
        let asker = asker();
        assert_eq!(
            listening_at(&one.against(&asker)),
            listening_at(&other.against(&asker))
        );
    }

    #[test]
    fn the_sigil_is_optional_so_a_trigger_token_reads_the_same() {
        assert_eq!(Address::read("iota-mu"), Address::read("$iota-mu"));
    }

    #[test]
    fn an_address_that_names_nobody_is_not_an_address() {
        assert_eq!(Address::read("$"), None);
        assert_eq!(Address::read(""), None);
        assert_eq!(Address::read("$a/b/c/d"), None, "four parts is a typo");
    }

    #[test]
    fn what_was_written_comes_back_out() {
        for written in ["$iota-mu", "$review/iota-mu", "$other/review/eta-nu"] {
            let address = Address::read(written).expect("an address");
            assert_eq!(address.written(), written);
        }
    }

    #[test]
    fn a_socket_is_named_by_the_id_and_nothing_else() {
        let path = listening_at(&asker());
        assert_eq!(path.file_name().expect("a name"), "alpha-rho");
        assert_eq!(
            path.parent()
                .expect("a project")
                .file_name()
                .expect("a name"),
            "magi"
        );
    }

    #[test]
    fn each_project_gets_its_own_directory() {
        // The project wall, put in the filesystem: another project's sessions are not refused,
        // they are somewhere this one never lists.
        let mine = home("magi");
        let theirs = home("other");
        assert_ne!(mine, theirs);
        assert_eq!(mine.parent(), theirs.parent());
    }

    #[test]
    fn a_project_name_cannot_climb_out_of_the_runtime_directory() {
        // A project is the working directory's name, and a directory can be called `..`.
        let escaping = Identity {
            project: "../../etc".to_owned(),
            role: "main".to_owned(),
            id: "../passwd".to_owned(),
        };
        let path = listening_at(&escaping);
        assert!(inside(&path), "it escaped to {path:?}");
        assert!(!path.to_string_lossy().contains(".."), "{path:?}");
    }

    #[test]
    fn a_name_of_nothing_does_not_name_the_directory_above() {
        assert_eq!(safe(""), "-");
        assert_eq!(safe("///"), "---");
    }

    #[test]
    fn the_parent_note_sits_beside_the_socket_and_is_not_mistaken_for_one() {
        let kin = kin_at(&asker());
        assert_eq!(kin.parent(), listening_at(&asker()).parent());
        let name = kin
            .file_name()
            .expect("a name")
            .to_string_lossy()
            .into_owned();
        assert!(name.contains('.'), "{name} would be listed as a session");
    }

    #[test]
    fn a_session_with_no_note_beside_it_is_a_main() {
        // Absence is the whole of what makes one, so the fallback has to be that and not an
        // error: a directory that cannot be read must not turn a main into a subagent.
        let unknown = whom("no-such-project-here", "iota-mu");
        assert!(unknown.is_main());
    }

    #[test]
    fn listening_answers_nothing_rather_than_failing_with_no_directory() {
        // Nothing has started here yet, which is every project until something does.
        assert!(listening("no-such-project-here").is_empty());
    }
}

/// A note written by a consenting parent is one every reader agrees with.
///
/// Split from this file under THE RULE, which caps a file at 800 lines.
#[cfg(test)]
#[path = "directory/adopting.rs"]
mod adopting;
