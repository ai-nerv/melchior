//! Answering another instance: one pure function taking a [`Call`], what this instance knows, and
//! who the far end worked out to be, and returning a [`Reply`]. The socket loop above it frames
//! bytes and reads the caller's place in the tree off the directory; it decides nothing.

pub mod keeping;
mod roles;

use crate::identity::Identity;
use crate::policy::Reach;
use crate::policy::{self, Whom};
use crate::wire::{Call, Message, Reply, VERBS};

/// What this instance is willing to say about itself, gathered by the caller and handed in: the
/// session is behind a lock in the UI thread, and a socket handler that reached into it would hold
/// that lock while a peer decides how fast to read.
#[derive(Debug, Clone)]
pub struct About {
    /// Who this is.
    pub me: Identity,
    /// Who started it, or `None` if it is a main.
    pub parent: Option<String>,
    /// The secret it was started with, which a `stop` has to quote back.
    pub token: Option<String>,
    /// Whether a turn is running.
    pub busy: bool,
    /// How long it has been running, in seconds.
    pub working_for: u64,
    /// What has arrived and not been read.
    pub inbox: Vec<Message>,
    /// The secret handed to each session this one started, by id, written only by `mint` and kept
    /// off disk: a secret a sibling could read off the directory would buy the reader authority
    /// over a session it did not start.
    pub minted: std::collections::BTreeMap<String, String>,
}

impl About {
    /// This session's place in the tree.
    #[must_use]
    pub fn whom(&self) -> Whom {
        Whom {
            project: self.me.project.clone(),
            id: self.me.id.clone(),
            parent: self.parent.clone(),
            // Read rather than held, so this answers the same as what a caller reads off the
            // directory about us.
            session: crate::directory::sessions::session_of(&self.me),
        }
    }
}

/// What a reply asks the caller to do afterwards, beyond sending it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Then {
    Nothing,
    /// Put this in the inbox.
    Keep(Message),
    /// Put this in front of the person, and hold it until they answer: whether one session may
    /// direct another is the one thing this layer never decides for itself.
    Ask(crate::wire::Request),
    /// A parent has taken this session on, and handed it this.
    ///
    /// Carried up to the harness rather than into the inbox: what a parent lends is not something
    /// a model should read.
    Adopted {
        /// Who took it on, as `project/role/id`.
        by: String,
        /// What they handed over, unread by anything here.
        handover: Option<String>,
    },
    /// A child has been named and its secret minted; hold on to it.
    ///
    /// Carried up rather than written here: this function decides and the loop that owns the state
    /// records, and the secret never goes near the directory.
    Minted {
        /// The child's id, which is what `stop` names.
        id: String,
        /// What it will be started with, and what a `stop` has to quote back.
        token: String,
    },
    /// This session has been told what it is for — by itself, or by the one that started it.
    ///
    /// Carried up rather than written where it is decided, for the same reason [`Then::Minted`]
    /// is: the loop owns the note, and a connection handler writing it too would be a second
    /// writer for one file.
    Named(crate::directory::roles::Role),
    /// End this instance.
    Stop,
}

/// Answer one call.
///
/// `caller` is read off the project directory rather than taken from the frame, so a session
/// cannot describe its own place in the tree. `None` means it did not say who it was, and then
/// only `verbs` and `client` are answered.
#[must_use]
pub fn answer(call: &Call, about: &About, caller: Option<&Whom>) -> (Reply, Then) {
    // Answered before any permission check, so a client never has to guess the vocabulary.
    if call.call == "verbs" {
        // A listing is the rows, and every row carries a `door`: both are FAMILY.md's.
        let mut listed = Reply::rows(
            VERBS
                .iter()
                .map(|(verb, about)| serde_json::json!({"verb": verb, "about": about, "door": "socket"}))
                .collect::<Vec<_>>(),
        );
        // What a plugin writes against, on the self-description and nowhere else.
        listed.surface = Some(crate::wire::SURFACE);
        return (listed, Then::Nothing);
    }
    // Hands over the client library, before the permission check as `verbs` is. Safe to answer a
    // stranger: it is a file this crate ships, and it says nothing about *this* session.
    if call.call == "client" {
        return (Reply::of(serde_json::json!(crate::CLIENT)), Then::Nothing);
    }
    let Some(caller) = caller else {
        return (
            Reply::refused("say who is calling: every call but `verbs` needs a `from`"),
            Then::Nothing,
        );
    };
    let me = about.whom();
    // Two relations, because the question has a direction: `relation` is how the caller stands to
    // this session, which is what `kin` reports, and `theirs` is how this session stands to the
    // caller, which is what decides whether they may. Deciding on the first would let a child stop
    // its parent.
    let relation = policy::between(&me, caller);
    let theirs = policy::between(caller, &me);
    // Settled by relation before the ladder, because no `Reach` says "nobody but this session":
    // `Stop` would refuse the only legitimate caller, since a session may not stop itself, and
    // `Tell` would let a parent mint children in its child's name and hold the secrets.
    if matches!(call.call.as_str(), "mint" | "minted") && relation != policy::Relation::Myself {
        return (
            Reply::refused(format!(
                "`{}` is this session's own: a name and the secret that ends the session wearing \
                 it are not something another instance may ask for",
                call.call
            )),
            Then::Nothing,
        );
    }

    // Settled by relation before the ladder, the way `mint` is: `Ask` would let a cousin rename us
    // at the `project` setting.
    if call.call == "role"
        && let Some(refusal) = roles::refused(relation, theirs)
    {
        return (refusal, Then::Nothing);
    }

    let wanted = match call.call.as_str() {
        "identity" | "kin" | "status" | "inbox" | "needs" => Reach::Ask,
        // Already settled above: only this session and its parent ask this, and they have.
        "role" => Reach::Ask,
        // Already settled above: only this session asks these, and it has.
        "mint" | "minted" => Reach::Ask,

        // Reaching as far as a message does, and no further.
        "tell" | "adopt" | "adopted" => Reach::Tell,
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
        // Read-only, so it answers anyone the walls allow. `configure` has no verb here: it runs
        // Lua, and a socket that runs things is remote code execution.
        // One row per declaration, which is the case FAMILY.md tells the story about.
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
        // One row: a map of id to secret is looked up by key, not walked.
        "minted" => (Reply::of(serde_json::json!(about.minted)), Then::Nothing),
        "role" => roles::set(call, about),
        // Names a child and mints its secret; starting the process is the harness's job, so
        // nothing is spawned here.
        "mint" => {
            // A tree that has reached its depth spawns no deeper: the child would be one level
            // past what [`crate::directory::MAX_DEPTH`] allows.
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
            // Nor more children at once than [`crate::directory::MAX_CHILDREN`]; ending one and
            // letting it be swept makes room for another.
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
            let secret = crate::identity::secret();
            // Ours, not the child's, and handed down unchanged however deep the tree gets: a child
            // that worked out its own run would start a second one under every coordinator.
            let run = about.whom().session.unwrap_or_else(|| about.me.id.clone());
            // What the child will be started as, if the caller said. Named at birth rather than
            // assigned once it is up, or it sits on the roster described as `main` in between.
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
                    // Everything the child inherits, named so a harness does not have to know any
                    // of it. The role travels as one string, name then description, so the child
                    // writes its own note from it.
                    "environment": {
                        crate::inherited::PROJECT: child.project,
                        crate::inherited::ROLE: role.written(),
                        crate::inherited::ID: child.id,
                        crate::inherited::PARENT: about.me.id,
                        crate::inherited::TOKEN: secret,
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
            // Project and id come from the connection, never from an argument: a message that
            // could name its own sender is one anybody can forge into anybody's inbox. The role is
            // the exception, because it grants nothing.
            let role = Identity::read(call.from.as_deref().unwrap_or_default())
                .map_or_else(|| "main".to_owned(), |claimed| claimed.role);
            let from = Identity {
                project: caller.project.clone(),
                role,
                id: caller.id.clone(),
            }
            .full();
            // Believed as sent: the count is refused at the *sender*, and what stops a caller that
            // reports zero on every pass of a ring is `keeping::AT_MOST`.
            let hops = call
                .args
                .get(3)
                .and_then(serde_json::Value::as_u64)
                .and_then(|deep| u32::try_from(deep).ok())
                .unwrap_or_default();
            let message = Message::sent(&from, &text, sort, about_what).carried(hops);
            // The id is minted here, where the message lands, and said out loud so the sender can
            // name what it sent; it is the same id the far end quotes in `about`.
            (
                Reply::of(serde_json::json!({"id": message.id})),
                Then::Keep(message),
            )
        }
        // The other half of the handshake, arriving at the session that asked. Believed only from
        // the session the *directory* says is this one's parent, and that note is written by
        // whoever accepted, so a session that never accepted anything gets nowhere.
        "adopted" => {
            if about.parent.as_deref() != Some(caller.id.as_str()) {
                return (
                    Reply::refused(format!(
                        "`{}` is not this session's parent, so it has nothing to hand over",
                        caller.id
                    )),
                    Then::Nothing,
                );
            }
            (
                Reply::done(),
                Then::Adopted {
                    by: text_at(call, 0).unwrap_or_else(|| caller.id.clone()),
                    handover: text_at(call, 1),
                },
            )
        }
        // Asked, never granted here: the reply says the question has been put, not that it was
        // answered.
        "adopt" => {
            // A main, and only a main: two lines of authority over one session would make "who may
            // direct this" unanswerable.
            if about.parent.is_some() {
                return (
                    Reply::refused(format!(
                        "`{}` already answers to `{}`, so it cannot take another session on",
                        about.me.full(),
                        about.parent.as_deref().unwrap_or_default()
                    )),
                    Then::Nothing,
                );
            }
            if !caller.is_main() {
                return (
                    Reply::refused(
                        "only a main may ask to be adopted: a session that already has a parent \
                         would be changing who directs it behind that parent's back",
                    ),
                    Then::Nothing,
                );
            }
            let why = text_at(call, 0).unwrap_or_default();
            let from = Identity {
                project: caller.project.clone(),
                role: Identity::read(call.from.as_deref().unwrap_or_default())
                    .map_or_else(|| "main".to_owned(), |claimed| claimed.role),
                id: caller.id.clone(),
            }
            .full();
            let request = crate::wire::Request::made(&from, &why);
            (
                Reply::of(serde_json::json!({
                    "asked": request.id,
                    "of": about.me.full(),
                })),
                Then::Ask(request),
            )
        }
        "stop" => {
            // The relation says the caller is the one that started this session; the secret says
            // it is actually them, because a name is free to claim and this is not.
            let Some(mine) = about.token.as_deref() else {
                return (
                    Reply::refused(
                        "this session was not started by another, so nothing may stop it",
                    ),
                    Then::Nothing,
                );
            };
            if call.token.as_deref() != Some(mine) {
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

/// A string argument, if there is one there.
fn text_at(call: &Call, at: usize) -> Option<String> {
    call.args.get(at)?.as_str().map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn about() -> About {
        About {
            me: Identity {
                project: "magi".to_owned(),
                role: "main".to_owned(),
                id: "alpha-rho".to_owned(),
            },
            parent: None,
            token: None,
            busy: false,
            working_for: 0,
            inbox: Vec::new(),
            minted: std::collections::BTreeMap::new(),
        }
    }

    fn whom(project: &str, id: &str, parent: Option<&str>) -> Whom {
        Whom {
            project: project.to_owned(),
            id: id.to_owned(),
            parent: parent.map(ToOwned::to_owned),
            session: None,
        }
    }

    fn call(verb: &str) -> Call {
        Call {
            call: verb.to_owned(),
            ..Call::default()
        }
    }

    #[test]
    fn verbs_is_answered_before_anybody_has_said_who_they_are() {
        let (reply, then) = answer(&call("verbs"), &about(), None);
        assert!(reply.ok, "{reply:?}");
        assert_eq!(reply.n, reply.result.len());
        assert_eq!(then, Then::Nothing);
    }

    #[test]
    fn everything_else_needs_a_caller() {
        for verb in ["identity", "status", "inbox", "tell", "stop"] {
            let (reply, then) = answer(&call(verb), &about(), None);
            assert!(!reply.ok, "{verb} answered a stranger");
            assert_eq!(then, Then::Nothing);
        }
    }

    #[test]
    fn a_main_answers_another_main_in_the_same_project() {
        let them = whom("magi", "beta-nu", None);
        let (reply, _) = answer(&call("status"), &about(), Some(&them));
        assert!(reply.ok, "{reply:?}");
    }

    #[test]
    fn nothing_from_another_project_is_answered() {
        let them = whom("other", "beta-nu", None);
        for verb in ["identity", "status", "inbox", "tell", "stop"] {
            let (reply, _) = answer(&call(verb), &about(), Some(&them));
            assert!(!reply.ok, "{verb} crossed the wall");
            let why = reply.error.unwrap_or_default();
            assert!(why.contains("projects"), "{why}");
        }
    }

    #[test]
    fn a_cousin_is_refused_at_the_default_and_told_what_would_help() {
        let mut about = about();
        about.parent = Some("beta-nu".to_owned());
        let them = whom("magi", "tau-chi", Some("gamma-xi"));
        let (reply, _) = answer(&call("status"), &about, Some(&them));
        assert!(!reply.ok);
        let why = reply.error.unwrap_or_default();
        assert!(why.contains("agent_talk"), "{why}");
    }

    #[test]
    fn a_message_is_stamped_with_who_actually_sent_it() {
        let them = whom("magi", "beta-nu", None);
        let call = Call {
            call: "tell".to_owned(),
            args: vec![serde_json::json!("the parser is done")],
            from: Some("magi/main/somebody-else".to_owned()),
            token: None,
        };
        let (reply, then) = answer(&call, &about(), Some(&them));
        assert!(reply.ok, "{reply:?}");
        let Then::Keep(message) = then else {
            panic!("it was not kept: {then:?}");
        };
        assert_eq!(message.from, "magi/main/beta-nu");
    }

    #[test]
    fn a_message_carries_the_sort_it_was_sent_as() {
        let them = whom("magi", "beta-nu", None);
        let call = Call {
            call: "tell".to_owned(),
            args: vec![
                serde_json::json!("I am stuck"),
                serde_json::json!("trouble"),
            ],
            ..Call::default()
        };
        let (_, then) = answer(&call, &about(), Some(&them));
        let Then::Keep(message) = then else {
            panic!("it was not kept");
        };
        assert!(message.sort.interrupts(), "{:?}", message.sort);
    }

    #[test]
    fn a_main_nobody_started_cannot_be_stopped_by_anything() {
        let them = whom("magi", "beta-nu", None);
        let (reply, then) = answer(&call("stop"), &about(), Some(&them));
        assert!(!reply.ok);
        assert_eq!(then, Then::Nothing);
    }

    #[test]
    fn a_parent_that_knows_the_secret_may_stop_its_child() {
        let mut about = about();
        about.parent = Some("beta-nu".to_owned());
        about.token = Some("s3cret".to_owned());
        let parent = whom("magi", "beta-nu", None);
        let call = Call {
            call: "stop".to_owned(),
            token: Some("s3cret".to_owned()),
            ..Call::default()
        };
        let (reply, then) = answer(&call, &about, Some(&parent));
        assert!(reply.ok, "{reply:?}");
        assert_eq!(then, Then::Stop);
    }

    #[test]
    fn the_name_alone_is_not_enough_to_stop_anything() {
        // Any process of this user can connect claiming to be the parent.
        let mut about = about();
        about.parent = Some("beta-nu".to_owned());
        about.token = Some("s3cret".to_owned());
        let pretending = whom("magi", "beta-nu", None);
        for token in [None, Some("guessed".to_owned())] {
            let call = Call {
                call: "stop".to_owned(),
                token,
                ..Call::default()
            };
            let (reply, then) = answer(&call, &about, Some(&pretending));
            assert!(!reply.ok, "a stop went through without the secret");
            assert_eq!(then, Then::Nothing);
        }
    }

    #[test]
    fn a_sibling_holding_the_secret_still_may_not_stop_it() {
        // The secret is proof of identity, not a permission.
        let mut about = about();
        about.parent = Some("beta-nu".to_owned());
        about.token = Some("s3cret".to_owned());
        let sibling = whom("magi", "zeta-pi", Some("beta-nu"));
        let call = Call {
            call: "stop".to_owned(),
            token: Some("s3cret".to_owned()),
            ..Call::default()
        };
        let (reply, then) = answer(&call, &about, Some(&sibling));
        assert!(!reply.ok, "{reply:?}");
        assert_eq!(then, Then::Nothing);
    }

    #[test]
    fn kin_says_how_the_caller_stands_and_whether_it_may_stop() {
        let mut about = about();
        about.parent = Some("beta-nu".to_owned());
        let parent = whom("magi", "beta-nu", None);
        let (reply, _) = answer(&call("kin"), &about, Some(&parent));
        assert!(reply.ok, "{reply:?}");
        assert_eq!(reply.result[0]["relation"], "parent");
        assert_eq!(reply.result[0]["may_stop"], true);
    }

    #[test]
    fn a_verb_nothing_answers_says_so_rather_than_dropping_the_connection() {
        let them = whom("magi", "beta-nu", None);
        let (reply, _) = answer(&call("nope"), &about(), Some(&them));
        assert!(!reply.ok);
        assert!(
            reply.error.unwrap_or_default().contains("nope"),
            "it did not say which"
        );
    }

    /// The session a listed verb is put to, and the caller entitled to put it. Three of them are
    /// answered only for a particular caller, so one parent-shaped caller would not do.
    fn entitled(verb: &str) -> (About, Whom) {
        let mut about = about();
        match verb {
            // Asked by the session itself, over the socket it bound.
            "mint" | "minted" => (about, whom("magi", "alpha-rho", None)),
            // A main asking a main, so this one is deliberately left without a parent.
            "adopt" => (about, whom("magi", "beta-nu", None)),
            _ => {
                about.parent = Some("beta-nu".to_owned());
                about.token = Some("s3cret".to_owned());
                (about, whom("magi", "beta-nu", None))
            }
        }
    }

    #[test]
    fn every_verb_the_family_is_told_about_is_one_that_answers() {
        for (name, _) in VERBS {
            let (about, caller) = entitled(name);
            let call = Call {
                call: (*name).to_owned(),
                args: vec![serde_json::json!("something")],
                token: Some("s3cret".to_owned()),
                from: None,
            };
            let (reply, _) = answer(&call, &about, Some(&caller));
            assert!(reply.ok, "{name} is listed and refuses: {reply:?}");
        }
    }

    #[test]
    fn a_verb_answered_only_for_one_caller_is_still_advertised() {
        // Refusing a caller is not the same as not having the verb, and only the list says which.
        let listed: Vec<&str> = VERBS.iter().map(|(name, _)| *name).collect();
        for verb in ["mint", "minted", "adopt", "adopted"] {
            assert!(listed.contains(&verb), "`{verb}` is answered and unlisted");
        }
    }
}

/// Being adopted is asked for, never taken.
#[cfg(test)]
#[path = "answering/adopting.rs"]
mod adopting;

/// What a parent lends is taken only from a parent.
#[cfg(test)]
#[path = "answering/handover.rs"]
mod handover;
