//! Answering another instance: one function taking a [`Call`], what this instance knows, and who
//! the far end was pinned as, and returning a [`Reply`] and what the loop should do next.

pub mod keeping;
mod roles;
mod taking;

use crate::identity::Identity;
use crate::policy::Reach;
use crate::policy::{self, Whom};
use crate::wire::{Call, Message, Reply, VERBS};

/// What this instance is willing to say about itself, gathered by the loop and handed in.
#[derive(Debug, Clone)]
pub struct About {
    pub me: Identity,
    /// Who started it, or `None` if it is a main.
    pub parent: Option<String>,
    /// The secret a `stop` has to quote back, handed to `serve` on its stdin.
    pub token: Option<String>,
    pub busy: bool,
    /// How long it has been running, in seconds.
    pub working_for: u64,
    /// The coarse phase the harness reports.
    pub phase: Option<String>,
    pub cause: Option<String>,
    /// What it has spent, a row per model, as the harness last said.
    pub spent: Vec<serde_json::Value>,
    pub inbox: Vec<Message>,
    /// Each started session's secret, by id; never written to disk.
    pub minted: std::collections::BTreeMap<String, String>,
    /// The secret a parent minted on adopting this session.
    pub adopted_token: Option<String>,
}

impl About {
    /// This session's place in the tree, read off the directory as a caller would read it.
    #[must_use]
    pub fn whom(&self) -> Whom {
        Whom {
            project: self.me.project.clone(),
            id: self.me.id.clone(),
            parent: self.parent.clone(),
            session: crate::directory::sessions::session_of(&self.me),
            root: crate::directory::run_of(&self.me.project, &self.me.id),
        }
    }
}

/// What a reply asks the loop to do afterwards, beyond sending it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Then {
    Nothing,
    /// Put this in the inbox.
    Keep(Message),
    /// Put this in front of the person, and hold it until they answer.
    Ask(crate::wire::Request),
    /// A parent has taken this session on: what it lends goes to the harness, never the inbox.
    Adopted {
        /// Who took it on, as `project/role/id`.
        by: String,
        handover: Option<String>,
        /// The stop secret the new parent minted.
        secret: Option<String>,
    },
    /// A child has been named and its secret minted, for the loop to hold.
    Minted {
        id: String,
        token: String,
    },
    /// This session has been told what it is for, for the loop to write down.
    Named(crate::directory::roles::Role),
    /// End this instance.
    Stop,
}

/// Answer one call.
///
/// `caller` is who the connection was pinned as when it was accepted, placed in the tree off the
/// project directory; nothing in the frame names it. `None` means nobody could be named, and then
/// only `verbs`, `client` and an `adopted` that proves itself are answered.
#[must_use]
pub fn answer(call: &Call, about: &About, caller: Option<&Whom>) -> (Reply, Then) {
    // The vocabulary and the library answer anybody; `adopted` proves itself with its nonce.
    if call.call == "verbs" {
        let mut listed = Reply::rows(
            VERBS
                .iter()
                .map(|(verb, about)| serde_json::json!({"verb": verb, "about": about, "door": "socket"}))
                .collect::<Vec<_>>(),
        );
        listed.surface = Some(crate::wire::SURFACE);
        return (listed, Then::Nothing);
    }
    if call.call == "client" {
        return (Reply::of(serde_json::json!(crate::CLIENT)), Then::Nothing);
    }
    if call.call == "adopted" {
        return taking::adopted(call, about);
    }
    let Some(caller) = caller else {
        return (
            Reply::refused(
                "nobody this session can name is calling: a caller is named by the session that \
                 started it, never by its call",
            ),
            Then::Nothing,
        );
    };
    let me = about.whom();
    // `relation` is how the caller stands to this session, what `kin` reports; `theirs` is how
    // this session stands to the caller, what decides whether it may.
    let relation = policy::between(&me, caller);
    let theirs = policy::between(caller, &me);
    if matches!(call.call.as_str(), "mint" | "minted" | "inbox")
        && relation != policy::Relation::Myself
    {
        return (
            Reply::refused(format!(
                "`{}` is this session's own: its children's names and secrets, and what was sent \
                 to it, are not something another instance may ask for",
                call.call
            )),
            Then::Nothing,
        );
    }

    // A parent proves itself with the secret it minted.
    if call.call == "role"
        && let Some(refusal) = roles::refused(relation, theirs, proven(call, about))
    {
        return (refusal, Then::Nothing);
    }

    // The model's coordination vocabulary, as `melchior tool` runs it, for this session only.
    if call.call == "tool" {
        if relation != policy::Relation::Myself {
            return (
                Reply::refused(
                    "`tool` is this session's own coordination surface; another instance may not \
                     run it as you",
                ),
                Then::Nothing,
            );
        }
        let standing = crate::verbs::Standing {
            inbox: about.inbox.clone(),
            forked: crate::directory::children(&about.me),
            parent: about.parent.clone(),
            minted: about.minted.clone(),
            me: about.me.full(),
        };
        let asked = call.args.first().cloned().unwrap_or_default();
        let answered = crate::verbs::answer(&asked, &standing);
        return (
            if answered.failed {
                Reply::refused(answered.said)
            } else {
                Reply::of(serde_json::json!(answered.said))
            },
            Then::Nothing,
        );
    }

    let wanted = match call.call.as_str() {
        "identity" | "kin" | "status" | "inbox" | "needs" | "role" | "mint" | "minted" => {
            Reach::Ask
        }
        "tell" | "adopt" => Reach::Tell,
        "stop" => Reach::Stop,

        other => {
            return (
                Reply::refused(format!("no such call: {other}")),
                Then::Nothing,
            );
        }
    };
    if !policy::may(caller, theirs, wanted) {
        return (
            Reply::refused(policy::refusal(caller, theirs, wanted)),
            Then::Nothing,
        );
    }
    match call.call.as_str() {
        // One row per declaration.
        "needs" => (
            Reply::rows(
                crate::mind::setup::needs()
                    .iter()
                    .map(|need| serde_json::json!(need))
                    .collect(),
            ),
            Then::Nothing,
        ),
        // The description comes off the note rather than out of `About`, which holds only the name.
        "identity" => (
            Reply::of(serde_json::json!({
                "project": about.me.project,
                "role": about.me.role,
                "description": crate::directory::roles::role_in(&about.me.project, &about.me.id)
                    .and_then(|role| role.description),
                "id": about.me.id,
                "full": about.me.full(),
                "parent": about.parent,
                "main": about.parent.is_none(),
            })),
            Then::Nothing,
        ),
        "kin" => (
            Reply::of(serde_json::json!({
                "relation": relation.word(),
                "means": relation.named(),
                "may_stop": policy::may(caller, theirs, Reach::Stop),
            })),
            Then::Nothing,
        ),
        "status" => (
            Reply::of(serde_json::json!({
                "busy": about.busy,
                "working_for": about.working_for,
                "waiting": about.inbox.len(),
                "phase": about.phase,
                "cause": about.cause,
                "spent": about.spent,
            })),
            Then::Nothing,
        ),
        "inbox" => (
            Reply::rows(
                about
                    .inbox
                    .iter()
                    .map(|message| serde_json::json!(message))
                    .collect(),
            ),
            Then::Nothing,
        ),
        // One row: a map of id to secret.
        "minted" => (Reply::of(serde_json::json!(about.minted)), Then::Nothing),
        "role" => roles::set(call, about),
        // Names a child and mints its secret; the harness starts the process.
        "mint" => {
            if crate::directory::depth_of(&about.me) + 1 >= crate::directory::MAX_DEPTH {
                return (
                    Reply::refused(format!(
                        "this session is {} deep and may start no child: a tree of agents goes \
                         {} levels and no further",
                        crate::directory::depth_of(&about.me),
                        crate::directory::MAX_DEPTH
                    )),
                    Then::Nothing,
                );
            }
            let running = crate::directory::children(&about.me).len();
            if running >= crate::directory::MAX_CHILDREN {
                return (
                    Reply::refused(format!(
                        "this session already has {running} children running, the most one may \
                         start at once"
                    )),
                    Then::Nothing,
                );
            }
            let child = crate::directory::free_in(&about.me.project);
            let secret = match crate::identity::secret() {
                Ok(secret) => secret,
                Err(why) => {
                    return (
                        Reply::no(
                            format!("no secret could be minted: {why}"),
                            crate::wire::Fault::Failed,
                        ),
                        Then::Nothing,
                    );
                }
            };
            // This session's run, handed down unchanged however deep the tree gets.
            let run = about.whom().session.unwrap_or_else(|| about.me.id.clone());
            let role = match roles::asked(call) {
                Ok(role) => role,
                Err(refusal) => return (refusal, Then::Nothing),
            };
            let minted = Then::Minted {
                id: child.id.clone(),
                token: secret.clone(),
            };
            (
                Reply::of(serde_json::json!({
                    "project": child.project,
                    "role": role.name,
                    "description": role.description,
                    "id": child.id,
                    "full": format!("{}/{}/{}", child.project, role.name, child.id),
                    "parent": about.me.id,
                    "token": secret.clone(),
                    "session": run.clone(),
                    // Everything the child inherits but its secret, which is `token` and never in
                    // an environment.
                    "environment": {
                        crate::inherited::PROJECT: child.project,
                        crate::inherited::ROLE: role.written(),
                        crate::inherited::ID: child.id,
                        crate::inherited::PARENT: about.me.id,
                        crate::inherited::SESSION: run,
                    },
                })),
                minted,
            )
        }
        "tell" => {
            let Some(text) = text_at(call, 0) else {
                return (Reply::refused("tell takes what to say"), Then::Nothing);
            };
            let sort = text_at(call, 1)
                .and_then(|name| crate::wire::Sort::read(&name))
                .unwrap_or_default();
            let about_what = text_at(call, 2);
            // From the connection and the role's note, never from an argument.
            let role = crate::directory::roles::role_in(&caller.project, &caller.id).map_or_else(
                || crate::directory::roles::MAIN.to_owned(),
                |role| role.name,
            );
            let from = Identity {
                project: caller.project.clone(),
                role,
                id: caller.id.clone(),
            }
            .full();
            // Believed as sent; `keeping::AT_MOST` bounds a ring whose members report zero.
            let hops = call
                .args
                .get(3)
                .and_then(serde_json::Value::as_u64)
                .and_then(|deep| u32::try_from(deep).ok())
                .unwrap_or_default();
            let message = Message::sent(&from, &text, sort, about_what).carried(hops);
            // The id is minted where the message lands, and said back to the sender.
            (
                Reply::of(serde_json::json!({"id": message.id})),
                Then::Keep(message),
            )
        }
        // Asked, never granted here: the reply says the question has been put.
        "adopt" => taking::adopt(call, about, caller),
        // The relation says the caller started this session; the secret says it is them.
        "stop" => {
            if about.token.is_none() && about.adopted_token.is_none() {
                return (
                    Reply::refused(
                        "this session was not started by another, so nothing may stop it",
                    ),
                    Then::Nothing,
                );
            }
            if !proven(call, about) {
                return (
                    Reply::refused("that is not the secret this session was started with"),
                    Then::Nothing,
                );
            }
            (Reply::done(), Then::Stop)
        }
        _ => unreachable!("the vocabulary was matched above"),
    }
}

/// Whether `call` quotes this session's stop secret: the one it was given at start, or the one a
/// parent minted on adopting it.
fn proven(call: &Call, about: &About) -> bool {
    let Some(quoted) = call.token.as_deref() else {
        return false;
    };
    [about.token.as_deref(), about.adopted_token.as_deref()]
        .into_iter()
        .flatten()
        .any(|mine| crate::identity::same(mine, quoted))
}

/// A string argument, if there is one there.
fn text_at(call: &Call, at: usize) -> Option<String> {
    call.args.get(at)?.as_str().map(ToOwned::to_owned)
}

/// The vocabulary, answered to the caller each verb is for.
#[cfg(test)]
#[path = "answering/basics.rs"]
mod basics;

/// Being adopted is asked for, never taken.
#[cfg(test)]
#[path = "answering/adopting.rs"]
mod adopting;

/// What a parent lends is taken only from a parent.
#[cfg(test)]
#[path = "answering/handover.rs"]
mod handover;
