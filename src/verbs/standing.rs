//! What the tool knows about the session it is answering for. Split from [`super`] under THE
//! RULE, which caps a file at 800 lines.
//!
//! A session's name is `project/role/id` and the role never reaches [`crate::policy`]: it is what
//! a session says it is for and may be set by the session itself, so one that could pick its own
//! role could pick `main` and claim a main's reach. [`Whom`] has no field for a role, and the
//! tests below are what would notice if that stopped being true.

use crate::identity::Identity;
use crate::policy::{self, Relation, Whom};
use crate::wire::Message;

/// What the tool needs from the session in order to answer, as a copy taken when the call started:
/// a tool runs on the turn thread and the session is the UI's.
#[derive(Debug, Clone, Default)]
pub struct Standing {
    /// Who this session is.
    pub me: String,
    /// Who started it, if anybody.
    pub parent: Option<String>,
    /// The ids of what it started, which is what it may stop. Ids rather than whole names, because
    /// that is what the directory holds and a role is not part of an address.
    pub forked: Vec<String>,
    /// The secret handed to each of them at spawn, by id, which a `stop` has to quote back.
    pub minted: std::collections::BTreeMap<String, String>,
    /// What has arrived.
    pub inbox: Vec<Message>,
}

impl Standing {
    /// This session as an identity, for filling the gaps in a short name.
    #[must_use]
    pub fn identity(&self) -> Identity {
        Identity::read(&self.me).unwrap_or_else(|| Identity {
            project: String::new(),
            role: crate::directory::roles::MAIN.to_owned(),
            id: String::new(),
        })
    }

    /// Where this session sits in the tree: project and id, and no role. The run comes off the
    /// note on disk rather than out of this struct, because a tool process is spawned per call and
    /// the note is the one copy every other agent reads.
    #[must_use]
    pub fn whom(&self) -> Whom {
        let me = self.identity();
        let session = crate::directory::sessions::session_of(&me);
        Whom {
            project: me.project,
            id: me.id,
            parent: self.parent.clone(),
            session,
        }
    }

    /// How `them` stands to this session. What this session started comes first; the rest is read
    /// off the project directory, never from what the far end says about itself.
    #[must_use]
    pub fn stands(&self, them: &Identity) -> Relation {
        let me = self.whom();
        if me.project != them.project {
            return Relation::Elsewhere;
        }
        // By id: matching whole names would miss a child that renamed itself.
        if self.forked.contains(&them.id) {
            return Relation::Child;
        }
        policy::between(&me, &crate::directory::whom(&them.project, &them.id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{Reach, Talk};

    fn wearing(role: &str) -> Standing {
        Standing {
            me: format!("magi/{role}/iota-mu"),
            parent: Some("alpha-rho".to_owned()),
            forked: Vec::new(),
            minted: std::collections::BTreeMap::new(),
            inbox: Vec::new(),
        }
    }

    #[test]
    fn a_session_that_calls_itself_main_does_not_stand_where_a_main_stands() {
        let honest = wearing("scratch").whom();
        let claiming = wearing("main").whom();
        assert_eq!(honest, claiming, "the role reached the place that decides");
        assert!(
            !claiming.is_main(),
            "calling itself `main` made it a main, and a main reaches every other main"
        );
    }

    #[test]
    fn no_role_changes_a_relation_or_a_permission_at_any_setting() {
        let them = Whom {
            project: "magi".to_owned(),
            id: "beta-nu".to_owned(),
            parent: None,
            session: None,
        };
        for role in ["main", "scratch", "coordinator", "reviewer"] {
            let me = wearing(role).whom();
            assert_eq!(
                policy::between(&me, &them),
                policy::between(&wearing("scratch").whom(), &them),
                "`{role}` bought a different relation"
            );
            for relation in [
                Relation::Myself,
                Relation::Parent,
                Relation::Child,
                Relation::Sibling,
                Relation::Kin,
                Relation::Root,
                Relation::Cousin,
                Relation::Elsewhere,
            ] {
                for reach in [Reach::Ask, Reach::Tell, Reach::Stop] {
                    for talk in [Talk::Mains, Talk::Instance, Talk::Project] {
                        assert_eq!(
                            policy::may_at(&me, relation, reach, talk),
                            policy::may_at(&wearing("scratch").whom(), relation, reach, talk),
                            "`{role}` bought {reach:?} on {relation:?} at {talk:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_module_that_decides_never_reads_a_role() {
        let source = include_str!("../policy.rs");
        let code: String = source
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for reached in [".role", "role:", "roles::", "role_of", "role_in"] {
            assert!(
                !code.contains(reached),
                "`{reached}` appears in policy: a role has started deciding something"
            );
        }
    }

    #[test]
    fn what_this_session_started_is_still_matched_by_id_rather_than_by_name() {
        let mut standing = wearing("main");
        standing.forked.push("zeta-pi".to_owned());
        let renamed = Identity {
            project: "magi".to_owned(),
            role: "whatever-it-fancies".to_owned(),
            id: "zeta-pi".to_owned(),
        };
        assert_eq!(standing.stands(&renamed), Relation::Child);
    }
}
