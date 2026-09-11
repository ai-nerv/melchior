//! Naming another session, and where it lives.
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
//! Every name beside a socket carries a dot, and the claims directory begins with one, because
//! [`listening`] dials each dotless entry as a socket. `stop` carries the secret from [`TOKEN`],
//! which only whoever started the session ever held; `ask` and `tell` take a name at face value.

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
pub const TOOL: &str = "agent";

/// Who started this session, if anybody did. A session with no parent is a *main*.
#[must_use]
pub fn parent() -> Option<String> {
    said(PARENT)
}

/// Who `me` answers to, as everybody else can see it. The note first, the environment second: a
/// session adopted while it runs is given a note by whoever accepted it, and no variable can be
/// set on a process that is already running.
#[must_use]
pub fn parent_of(me: &Identity) -> Option<String> {
    std::fs::read_to_string(kin_at(me))
        .ok()
        .map(|said| said.trim().to_owned())
        .filter(|said| !said.is_empty())
        .or_else(parent)
}

/// How deep a tree of agents may go: a root and this many generations under it. A bound, so a
/// session that keeps spawning children cannot grow the tree without end — the breadth of it is
/// the operating system's to cap, but the depth is counted here where the parentage is known.
pub const MAX_DEPTH: u32 = 8;

/// How many ancestors `me` has, counting up the parent notes and stopping at [`MAX_DEPTH`] — a root
/// is `0`. Read off the directory, so it is the same number any other session would compute.
#[must_use]
pub fn depth_of(me: &Identity) -> u32 {
    let mut depth = 0;
    let mut above = parent_of(me);
    while let Some(id) = above {
        depth += 1;
        if depth >= MAX_DEPTH {
            break;
        }
        above = whom(&me.project, &id).parent;
    }
    depth
}

#[must_use]
pub fn token() -> Option<String> {
    said(TOKEN)
}

/// Which session a spawned process belongs to, from its environment. `None` outside one.
#[must_use]
pub fn mine() -> Option<Identity> {
    let project = said(PROJECT)?;
    let id = said(ID)?;
    // The note first, the environment second, as in `parent_of`: `assign` and `role` change what
    // an agent is for while it runs, and no variable can be set on a process already started.
    let role = roles::role_of(&Identity {
        project: project.clone(),
        role: roles::MAIN.to_owned(),
        id: id.clone(),
    })
    .name;
    Some(Identity { project, role, id })
}

/// What `me` started, read off the project directory. Ids, not whole names: a role is not on
/// disk, so a full name built from here would be a name with a guess in it.
#[must_use]
pub fn children(me: &Identity) -> Vec<String> {
    listening(&me.project)
        .into_iter()
        .filter(|id| *id != me.id)
        .filter(|id| whom(&me.project, id).parent.as_deref() == Some(me.id.as_str()))
        .collect()
}

/// What has arrived for `me`, asked of `me`'s own socket because the inbox lives in the UI's
/// memory. Empty when nothing answers.
#[must_use]
pub fn inbox_of(me: &Identity) -> Vec<crate::wire::Message> {
    let Ok(mut held) = dial(me, me) else {
        return Vec::new();
    };
    let Ok(reply) = held.call("inbox", Vec::new()) else {
        return Vec::new();
    };
    // A row is a message. Reading the first row as the whole inbox is the other half of the
    // mistake FAMILY.md names, and it is what a consumer holding a stale client would do.
    reply
        .result
        .iter()
        .filter_map(|value| serde_json::from_value(value.clone()).ok())
        .collect()
}

/// The secrets this session minted for the children it started, by id. Never written to the
/// directory: a sibling that could read one off disk would have authority over a session it did
/// not start. Empty when nothing answers, which refuses `stop`.
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

/// Another instance, by name: `$iota-mu`, `$review/iota-mu` or `$magi/review/iota-mu`, the short
/// forms filling in from whoever is asking. The id is the part that finds it — a role in an
/// address is carried along and never consulted to work out which socket is meant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Address {
    pub project: Option<String>,
    pub role: Option<String>,
    pub id: String,
}

impl Address {
    /// Read `$iota-mu`, `$review/iota-mu` or `$magi/review/iota-mu`, with or without the sigil.
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

    /// The full name, with the gaps filled in: the project from whoever is asking, the role from
    /// the target's own note. An unqualified name borrowing the asker's role reported every
    /// message as landing in an inbox belonging to somebody with a role the target does not have.
    #[must_use]
    pub fn against(&self, asker: &Identity) -> Identity {
        let project = self
            .project
            .clone()
            .unwrap_or_else(|| asker.project.clone());
        let role = self.role.clone().unwrap_or_else(|| {
            roles::role_in(&project, &self.id).map_or_else(|| roles::MAIN.to_owned(), |it| it.name)
        });
        Identity {
            project,
            role,
            id: self.id.clone(),
        }
    }
}

#[must_use]
pub fn home(project: &str) -> PathBuf {
    runtime().join(crate::NAME).join(safe(project))
}

/// Where a session's socket is. No role in the path: an id is already unique inside a project,
/// and a session that changed what it was for would move.
#[must_use]
pub fn socket(project: &str, id: &str) -> PathBuf {
    home(project).join(safe(id))
}

#[must_use]
pub fn listening_at(me: &Identity) -> PathBuf {
    socket(&me.project, &me.id)
}

/// Where the note saying who started `me` is put.
#[must_use]
pub fn kin_at(me: &Identity) -> PathBuf {
    home(&me.project).join(format!("{}.parent", safe(&me.id)))
}

fn runtime() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

/// Flatten one name into one path segment, so a project called `../../etc` cannot name a
/// directory outside the one this chose.
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
/// A main writes no parent, and that absence is what says it is one. `ui` is where the harness
/// draws this agent, `None` for a session with no screen to offer.
///
/// # Errors
/// When a note cannot be written. A child that could not write `.parent` reads as a main to every
/// peer, with a main's reach, so the caller must refuse to come up rather than serve under it.
pub fn announce(me: &Identity, role: &roles::Role, ui: Option<&Path>) -> Result<(), String> {
    sessions::began(me)?;
    roles::began(me, role)?;
    screens::began(me, ui)?;
    let Some(parent) = parent() else {
        return Ok(());
    };
    wrote(&kin_at(me), &parent)
}

/// Write one note beside a socket, naming the path in the error so a full filesystem and an
/// unwritable runtime directory are told apart.
pub(crate) fn wrote(path: &Path, said: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|why| format!("{}: {why}", dir.display()))?;
    }
    std::fs::write(path, said).map_err(|why| format!("{}: {why}", path.display()))
}

pub fn forget(me: &Identity) {
    let _ = std::fs::remove_file(kin_at(me));
    sessions::ended(me);
    roles::ended(me);
    screens::ended(me);
    sending::ended(me);
    claims::forget_in(&me.project, &me.id);
}

/// Record that `them` now answers to `parent`, written by the session that consented rather than
/// the one that asked. `parent` is an id, not a full name: [`children`] and [`policy::between`]
/// both compare it against a bare id.
pub fn adopted(them: &Identity, parent: &str) -> Result<(), String> {
    wrote(&kin_at(them), parent)
}

/// Tell whoever asked what was decided, either way, as an ordinary message. `handover` goes by a
/// separate call so what a harness lends an adopted session never lands in an inbox a model reads.
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
#[must_use]
pub fn free_in(project: &str) -> Identity {
    crate::identity::free_of(project, &listening(project))
}

/// Open a connection to the session named `them`, as `me`.
///
/// # Errors
/// When nothing is listening under that name: the socket file outlives the process that made it.
pub fn dial(them: &Identity, me: &Identity) -> std::io::Result<crate::asking::Held> {
    crate::asking::Held::at(&listening_at(them), me)
}

/// What is known about a session, read off the directory rather than asked of the session itself.
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

/// Every session currently listening in `project`, read off the directory and dial-tested, with
/// anything that no longer answers swept on the way past.
#[must_use]
pub fn listening(project: &str) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(home(project)) else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        // The notes sit beside the sockets, and an id is two Greek words and a dash.
        .filter(|name| !name.contains('.') && !name.is_empty())
        .filter(|id| {
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

/// Whether anything is serving at `path`. A listener answers from the moment it is bound, so this
/// is never a false negative against a session that is merely busy.
#[must_use]
pub fn answers(path: &Path) -> bool {
    std::os::unix::net::UnixStream::connect(path).is_ok()
}

/// Take a dead session out of the directory: its socket, the notes beside it, and its claims.
fn forget_id(project: &str, id: &str) {
    let _ = std::fs::remove_file(socket(project, id));
    let _ = std::fs::remove_file(home(project).join(format!("{}.parent", safe(id))));
    sessions::forget_in(project, id);
    roles::forget_in(project, id);
    screens::forget_in(project, id);
    sending::forget_in(project, id);
    claims::forget_in(project, id);
}

/// Drop the project's directory if nothing is left in it.
pub fn leave(project: &str) {
    // The claims directory first: an empty one left inside would keep the project's alive.
    claims::leave(project);
    let _ = std::fs::remove_dir(home(project));
}

/// Everyone `me` may actually reach, with how they stand to it, filtered so a session is never
/// told about something it would then be refused.
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

/// Whether a path is one this process may listen on: two levels below the runtime root and no
/// more, so neither half of a name that came off a wire can climb.
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
    fn an_unqualified_name_wears_the_target_s_role_and_never_the_asker_s() {
        let project = crate::scratch::Project::new("melchior-address", "role");
        roles::given(&project, "iota-mu", &roles::Role::new("reviewer", None)).expect("the note");
        let asker = Identity {
            project: project.to_string(),
            role: "coordinator".to_owned(),
            id: "alpha-rho".to_owned(),
        };
        let whole = Address::read("$iota-mu")
            .expect("an address")
            .against(&asker);
        assert_eq!(whole.role, "reviewer", "it reported the asker's own role");
        assert_eq!(whole.full(), format!("{project}/reviewer/iota-mu"));
    }

    #[test]
    fn a_target_nobody_has_described_is_a_main_rather_than_whatever_the_asker_is() {
        let project = crate::scratch::Project::new("melchior-address", "unknown");
        let asker = Identity {
            project: project.to_string(),
            role: "coordinator".to_owned(),
            id: "alpha-rho".to_owned(),
        };
        let whole = Address::read("$nobody-nowhere")
            .expect("an address")
            .against(&asker);
        assert_eq!(
            whole.role,
            roles::MAIN,
            "an unknown target borrowed the asker's role"
        );
    }

    #[test]
    fn a_role_written_into_the_name_still_wins_over_the_note() {
        let project = crate::scratch::Project::new("melchior-address", "written");
        roles::given(&project, "iota-mu", &roles::Role::new("reviewer", None)).expect("the note");
        let asker = Identity {
            project: project.to_string(),
            role: "main".to_owned(),
            id: "alpha-rho".to_owned(),
        };
        let whole = Address::read("$scratch/iota-mu")
            .expect("an address")
            .against(&asker);
        assert_eq!(whole.role, "scratch");
    }

    #[test]
    fn a_three_part_name_gives_everything() {
        let address = Address::read("$other/review/eta-nu").expect("an address");
        assert_eq!(address.against(&asker()).full(), "other/review/eta-nu");
    }

    #[test]
    fn the_role_in_an_address_never_decides_which_socket_is_meant() {
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
        // A directory that cannot be read must not turn a main into a subagent.
        let unknown = whom("no-such-project-here", "iota-mu");
        assert!(unknown.is_main());
    }

    #[test]
    fn listening_answers_nothing_rather_than_failing_with_no_directory() {
        assert!(listening("no-such-project-here").is_empty());
    }
}

/// A note written by a consenting parent is one every reader agrees with.
#[cfg(test)]
#[path = "directory/adopting.rs"]
mod adopting;
