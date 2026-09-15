//! Who may speak to whom. Two walls, and everything else is a setting.
//!
//! The project wall: a session can see and reach only what is inside its own project's runtime
//! directory. Not "should not" — cannot, because the directory it lists and the directory it
//! dials are the one it belongs to. Nothing in this file can turn that off.
//!
//! The instance wall: inside a project, an *instance* is a main and the subagents it started. A
//! main is that instance's front door and is reachable by the other mains; the subagents behind
//! it are private.
//!
//! ```text
//!   alpha-rho  <--->  beta-nu          two mains, two instances
//!      |                 |
//!      +- iota-mu        +- tau-chi     each main's own subagents
//!      +- zeta-pi        +- xi-phi
//! ```
//!
//! The default is the tightest thing that still works: mains talk to mains, and a subagent talks
//! to whoever started it, which no setting governs.
//!
//! | `magi.agent_talk` | and also |
//! |---|---|
//! | `"mains"` *(default)* | — |
//! | `"instance"` | siblings, and kin: anything else started under the same root |
//! | `"project"` | cousins, and a subagent reaching another instance's main |
//!
//! It only ever opens things; there is no level below `mains`.

use std::sync::OnceLock;

/// What a caller is asking to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// Read something: who it is, what it is doing, what it has been told.
    Ask,
    /// Put a message in its inbox.
    Tell,
    /// End it.
    Stop,
}

impl Reach {
    /// The verb, for saying so in a refusal.
    #[must_use]
    pub fn named(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Tell => "tell",
            Self::Stop => "stop",
        }
    }
}

/// How far a session may reach, as the config set it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Talk {
    /// Mains talk to mains. A subagent talks to its parent and no further.
    #[default]
    Mains,
    /// And siblings talk to each other.
    Instance,
    /// And anything in the project may reach anything else in it.
    Project,
}

impl Talk {
    /// Read what a config said, or `None` if it is not one of these.
    #[must_use]
    pub fn read(name: &str) -> Option<Self> {
        Some(match name.trim() {
            "mains" => Self::Mains,
            "instance" => Self::Instance,
            "project" => Self::Project,
            _ => return None,
        })
    }

    /// What it is called, for saying so in a refusal.
    #[must_use]
    pub fn named(self) -> &'static str {
        match self {
            Self::Mains => "mains",
            Self::Instance => "instance",
            Self::Project => "project",
        }
    }
}

/// What the config chose, filled once at startup.
static CHOSEN: OnceLock<Talk> = OnceLock::new();

/// Take an answer from somewhere other than the environment, before anything asks: the first
/// read fills the `OnceLock`, so this has to run before any call is answered.
pub fn adopt(talk: Talk) {
    let _ = CHOSEN.set(talk);
}

/// How far a session may reach. From the environment, because the two processes that ask are
/// started separately: the one holding the socket and the one a model calls. Anything unreadable
/// is the default rather than a guess, so a typo cannot open a wall.
#[must_use]
pub fn talk() -> Talk {
    *CHOSEN.get_or_init(|| {
        crate::inherited::said(crate::inherited::TALK)
            .and_then(|name| Talk::read(&name))
            .unwrap_or_default()
    })
}

/// Who a session is, as far as the tree is concerned: not what it calls itself, but what can be
/// found out about it from the project directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Whom {
    /// Which project it belongs to.
    pub project: String,
    /// Its id, which is what its socket is called.
    pub id: String,
    /// Who started it, or `None` if it is a main.
    pub parent: Option<String>,
    /// Which run it was *born* in: the run note it inherited at mint, kept as provenance and as the
    /// key its memory is filed under — never rewritten, not even by adoption. See [`Whom::session_root`].
    pub session: Option<String>,
    /// The top of its branch *now*, found by walking the parent notes to the root — so an adopted
    /// subtree belongs to whoever took it on. `None` when it was not walked (a hand-built peer, a
    /// session with no parent), and then [`Whom::tree_root`] falls back to the born run. Filled by
    /// [`crate::directory::whom`], which is the one place that reads the whole chain off the directory.
    pub root: Option<String>,
}

impl Whom {
    /// Whether this is a main — an instance's front door — rather than somebody's subagent.
    #[must_use]
    pub fn is_main(&self) -> bool {
        self.parent.is_none()
    }

    /// Which run this belongs to: a session with no note is its own root, not one belonging to
    /// nothing, since two of those would otherwise share `None` and read as one run.
    #[must_use]
    pub fn session_root(&self) -> &str {
        self.session.as_deref().unwrap_or(&self.id)
    }

    /// The top of its branch as the tree stands now: the walked-up root when one was found, else
    /// the born run. This is what decides run membership, so an adopted subtree joins the run of
    /// whoever took it on while its memory stays filed under the run it was born in.
    #[must_use]
    pub fn tree_root(&self) -> &str {
        self.root.as_deref().unwrap_or_else(|| self.session_root())
    }
}

/// How two sessions stand to each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    /// The same session. Reached by name rather than by knowing it was itself.
    Myself,
    /// It started me.
    Parent,
    /// I started it.
    Child,
    /// We were both started by the same session.
    Sibling,
    /// Somewhere else in the same run: started under the same root, at whatever depth.
    Kin,
    /// Another session in this project that nobody started: a front door of its own.
    Root,
    /// Another instance's subagent. Behind somebody else's front door.
    Cousin,
    /// A different project. Beyond the wall, and normally not even visible.
    Elsewhere,
}

impl Relation {
    /// The one-word form, for a chip on a block or a caller that wants to branch on it.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Myself => "myself",
            Self::Parent => "parent",
            Self::Child => "child",
            Self::Sibling => "sibling",
            Self::Kin => "kin",
            // The word on the wire stays `main`; the Rust name is this crate's alone.
            Self::Root => "main",
            Self::Cousin => "cousin",
            Self::Elsewhere => "elsewhere",
        }
    }

    /// What to call it in a sentence, for saying why.
    #[must_use]
    pub fn named(self) -> &'static str {
        match self {
            Self::Myself => "this session",
            Self::Parent => "the session that started this one",
            Self::Child => "a subagent this session started",
            Self::Sibling => "a sibling subagent",
            Self::Kin => "another agent in this session",
            Self::Root => "another instance's main",
            Self::Cousin => "another instance's subagent",
            Self::Elsewhere => "in another project",
        }
    }
}

/// How `them` stands to `me`.
#[must_use]
pub fn between(me: &Whom, them: &Whom) -> Relation {
    if me.project != them.project {
        return Relation::Elsewhere;
    }
    if me.id == them.id {
        return Relation::Myself;
    }
    if me.parent.as_deref() == Some(them.id.as_str()) {
        return Relation::Parent;
    }
    if them.parent.as_deref() == Some(me.id.as_str()) {
        return Relation::Child;
    }
    // Both have a parent and it is the same one: `None == None` would make every pair of mains
    // siblings, which is why the parent is unwrapped rather than compared as an option.
    if let (Some(mine), Some(theirs)) = (me.parent.as_deref(), them.parent.as_deref())
        && mine == theirs
    {
        return Relation::Sibling;
    }
    // The top of the branch as it stands now, walked up the parent notes: a subtree adopted by a
    // new root reads as kin to it, which is what makes a graft one tree rather than a link between
    // two. Before any adoption this is the run they were born in, so nothing else moves.
    if me.tree_root() == them.tree_root() {
        return Relation::Kin;
    }
    if them.is_main() {
        return Relation::Root;
    }
    Relation::Cousin
}

/// Whether `me` may do `reach` to something standing in `relation` to it, and the one place that
/// answers "may I".
#[must_use]
pub fn may(me: &Whom, relation: Relation, reach: Reach) -> bool {
    may_at(me, relation, reach, talk())
}

/// The same question with the setting handed in, so every level can be tested.
#[must_use]
pub fn may_at(me: &Whom, relation: Relation, reach: Reach, talk: Talk) -> bool {
    if reach == Reach::Stop {
        // The caller must also hold the secret handed down at spawn, checked where this is
        // answered.
        return relation == Relation::Child;
    }
    match relation {
        // The project wall, at every setting.
        Relation::Elsewhere => false,
        Relation::Myself | Relation::Parent | Relation::Child => true,
        Relation::Sibling => matches!(talk, Talk::Instance | Talk::Project),
        Relation::Kin => matches!(talk, Talk::Instance | Talk::Project),
        // A subagent knocking on another instance's front door crosses the instance wall.
        Relation::Root => me.is_main() || talk == Talk::Project,
        Relation::Cousin => talk == Talk::Project,
    }
}

/// Why it was refused, and which setting would have allowed it, said by name.
#[must_use]
pub fn refusal(me: &Whom, relation: Relation, reach: Reach) -> String {
    refusal_at(me, relation, reach, talk())
}

/// The same, with the setting handed in.
#[must_use]
pub fn refusal_at(me: &Whom, relation: Relation, reach: Reach, talk: Talk) -> String {
    let verb = reach.named();
    if relation == Relation::Elsewhere {
        return format!(
            "that is {}, and magi does not reach across projects",
            relation.named()
        );
    }
    if reach == Reach::Stop {
        return format!(
            "only the session that started one may stop it, and that is {}",
            relation.named()
        );
    }
    let needed = match relation {
        Relation::Sibling | Relation::Kin => Talk::Instance,
        Relation::Cousin => Talk::Project,
        Relation::Root if !me.is_main() => Talk::Project,
        _ => Talk::Project,
    };
    format!(
        "this session may not {verb} {} while `magi.agent_talk` is \"{}\"; it would need \"{}\"",
        relation.named(),
        talk.named(),
        needed.named()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::tests_support::{born_in, main_of, under};

    #[test]
    fn a_main_has_no_parent_and_a_subagent_does() {
        assert!(main_of("magi", "alpha-rho").is_main());
        assert!(!under("magi", "iota-mu", "alpha-rho").is_main());
    }

    #[test]
    fn two_mains_in_one_project_are_each_other_s_front_door() {
        let me = main_of("magi", "alpha-rho");
        let them = main_of("magi", "beta-nu");
        assert_eq!(between(&me, &them), Relation::Root);
    }

    #[test]
    fn the_spawn_link_reads_the_same_from_both_ends() {
        let parent = main_of("magi", "alpha-rho");
        let child = under("magi", "iota-mu", "alpha-rho");
        assert_eq!(between(&parent, &child), Relation::Child);
        assert_eq!(between(&child, &parent), Relation::Parent);
    }

    #[test]
    fn two_subagents_of_one_parent_are_siblings() {
        let one = under("magi", "iota-mu", "alpha-rho");
        let other = under("magi", "zeta-pi", "alpha-rho");
        assert_eq!(between(&one, &other), Relation::Sibling);
    }

    #[test]
    fn two_subagents_of_different_parents_are_cousins_not_siblings() {
        let mine = under("magi", "iota-mu", "alpha-rho");
        let theirs = under("magi", "tau-chi", "beta-nu");
        assert_eq!(between(&mine, &theirs), Relation::Cousin);
    }

    #[test]
    fn two_agents_of_one_run_are_kin_however_deep_they_sit() {
        let mine = born_in("magi", "iota-mu", "alpha-rho", "alpha-rho");
        let far = born_in("magi", "zeta-pi", "tau-chi", "alpha-rho");
        assert_eq!(between(&mine, &far), Relation::Kin);
        assert_eq!(between(&far, &mine), Relation::Kin);

        let root = Whom {
            session: Some("alpha-rho".to_owned()),
            ..main_of("magi", "alpha-rho")
        };
        assert_eq!(between(&root, &far), Relation::Kin);
    }

    #[test]
    fn a_cousin_is_now_only_somebody_from_another_run() {
        let mine = born_in("magi", "iota-mu", "alpha-rho", "alpha-rho");
        let theirs = born_in("magi", "tau-chi", "beta-nu", "beta-nu");
        assert_eq!(between(&mine, &theirs), Relation::Cousin);

        let ours = born_in("magi", "zeta-pi", "tau-chi", "alpha-rho");
        assert_ne!(between(&mine, &ours), Relation::Cousin);
    }

    #[test]
    fn kin_is_the_setting_a_sibling_needs_and_not_the_one_a_cousin_does() {
        let mine = born_in("magi", "iota-mu", "alpha-rho", "alpha-rho");
        assert!(!may_at(&mine, Relation::Kin, Reach::Tell, Talk::Mains));
        assert!(may_at(&mine, Relation::Kin, Reach::Tell, Talk::Instance));
        assert!(!may_at(
            &mine,
            Relation::Cousin,
            Reach::Tell,
            Talk::Instance
        ));
    }

    #[test]
    fn a_session_with_no_note_is_its_own_run_rather_than_one_that_belongs_to_nothing() {
        let me = main_of("magi", "alpha-rho");
        let them = main_of("magi", "beta-nu");
        assert_eq!(me.session_root(), "alpha-rho");
        assert_ne!(between(&me, &them), Relation::Kin);
    }

    #[test]
    fn every_rung_is_on_the_list_the_matrix_tests_walk() {
        use super::tests_support::{EVERY, rung};
        for (at, relation) in EVERY.iter().enumerate() {
            assert_eq!(rung(*relation), at, "{relation:?} is out of place");
        }
        assert_eq!(
            EVERY.len(),
            rung(Relation::Elsewhere) + 1,
            "a rung is missing"
        );
    }

    #[test]
    fn two_mains_are_not_siblings_for_both_having_no_parent() {
        let me = main_of("magi", "alpha-rho");
        let them = main_of("magi", "beta-nu");
        assert_ne!(between(&me, &them), Relation::Sibling);
    }

    #[test]
    fn a_different_project_is_elsewhere_whoever_is_asking() {
        let me = main_of("magi", "alpha-rho");
        for them in [
            main_of("other", "beta-nu"),
            under("other", "tau-chi", "beta-nu"),
        ] {
            assert_eq!(between(&me, &them), Relation::Elsewhere);
        }
    }

    #[test]
    fn nothing_reaches_across_projects_at_any_setting() {
        let me = main_of("magi", "alpha-rho");
        for reach in [Reach::Ask, Reach::Tell, Reach::Stop] {
            assert!(!may(&me, Relation::Elsewhere, reach), "{reach:?} escaped");
        }
    }

    #[test]
    fn the_spawn_link_works_without_any_setting() {
        let child = under("magi", "iota-mu", "alpha-rho");
        assert_eq!(talk(), Talk::Mains, "the default moved");
        assert!(may(&child, Relation::Parent, Reach::Tell));
        assert!(may(&child, Relation::Parent, Reach::Ask));
    }

    #[test]
    fn mains_reach_each_other_at_the_default() {
        let me = main_of("magi", "alpha-rho");
        assert!(may(&me, Relation::Root, Reach::Ask));
        assert!(may(&me, Relation::Root, Reach::Tell));
    }

    #[test]
    fn siblings_and_cousins_are_refused_at_the_default() {
        let child = under("magi", "iota-mu", "alpha-rho");
        assert!(!may(&child, Relation::Sibling, Reach::Tell));
        assert!(!may(&child, Relation::Cousin, Reach::Tell));
        assert!(
            !may(&child, Relation::Root, Reach::Tell),
            "a subagent knocking on another instance's door crosses the wall"
        );
    }

    #[test]
    fn only_the_parent_may_stop_a_session() {
        let me = main_of("magi", "alpha-rho");
        for relation in super::tests_support::EVERY {
            if relation == Relation::Child {
                continue;
            }
            assert!(!may(&me, relation, Reach::Stop), "{relation:?} could stop");
        }
        assert!(may(&me, Relation::Child, Reach::Stop));
    }

    #[test]
    fn a_setting_that_is_not_one_is_not_read_as_a_looser_one() {
        assert_eq!(Talk::read("everything"), None);
        assert_eq!(Talk::read("Project"), None);
        assert_eq!(Talk::read("project"), Some(Talk::Project));
    }

    #[test]
    fn a_refusal_names_the_setting_that_would_have_allowed_it() {
        let child = under("magi", "iota-mu", "alpha-rho");
        let said = refusal(&child, Relation::Sibling, Reach::Tell);
        assert!(said.contains("agent_talk"), "{said}");
        assert!(said.contains("instance"), "{said}");
    }

    #[test]
    fn a_refusal_across_projects_does_not_offer_a_setting_that_would_help() {
        let me = main_of("magi", "alpha-rho");
        let said = refusal(&me, Relation::Elsewhere, Reach::Ask);
        assert!(!said.contains("agent_talk"), "{said}");
        assert!(said.contains("across projects"), "{said}");
    }
}

#[cfg(test)]
mod levels {
    use super::tests_support::{main_of, under};
    use super::*;

    #[test]
    fn instance_opens_siblings_and_leaves_the_instance_wall_standing() {
        let child = under("magi", "iota-mu", "alpha-rho");
        assert!(may_at(
            &child,
            Relation::Sibling,
            Reach::Tell,
            Talk::Instance
        ));
        assert!(
            !may_at(&child, Relation::Cousin, Reach::Tell, Talk::Instance),
            "a cousin is behind another front door"
        );
        assert!(!may_at(&child, Relation::Root, Reach::Tell, Talk::Instance));
    }

    #[test]
    fn project_opens_everything_inside_the_project() {
        let child = under("magi", "iota-mu", "alpha-rho");
        for relation in [Relation::Sibling, Relation::Cousin, Relation::Root] {
            assert!(
                may_at(&child, relation, Reach::Tell, Talk::Project),
                "{relation:?} was still refused"
            );
        }
    }

    #[test]
    fn no_setting_opens_the_project_wall_or_widens_who_may_stop() {
        let me = main_of("magi", "alpha-rho");
        for talk in [Talk::Mains, Talk::Instance, Talk::Project] {
            assert!(!may_at(&me, Relation::Elsewhere, Reach::Ask, talk));
            assert!(!may_at(&me, Relation::Root, Reach::Stop, talk));
            assert!(!may_at(&me, Relation::Sibling, Reach::Stop, talk));
        }
    }

    #[test]
    fn each_step_only_ever_adds() {
        let who = [
            main_of("magi", "alpha-rho"),
            under("magi", "iota-mu", "alpha-rho"),
        ];
        for me in &who {
            for relation in super::tests_support::EVERY {
                for reach in [Reach::Ask, Reach::Tell] {
                    let tight = may_at(me, relation, reach, Talk::Mains);
                    let middle = may_at(me, relation, reach, Talk::Instance);
                    let loose = may_at(me, relation, reach, Talk::Project);
                    assert!(!tight || middle, "{relation:?} closed at instance");
                    assert!(!middle || loose, "{relation:?} closed at project");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests_support {
    use super::{Relation, Whom};

    /// Every rung, in order, for the tests that must walk all of them.
    pub const EVERY: [Relation; 8] = [
        Relation::Myself,
        Relation::Parent,
        Relation::Child,
        Relation::Sibling,
        Relation::Kin,
        Relation::Root,
        Relation::Cousin,
        Relation::Elsewhere,
    ];

    /// Where a rung sits in [`EVERY`]. A match rather than a lookup, so a rung added to the enum
    /// stops compiling here until somebody says where it goes.
    pub fn rung(relation: Relation) -> usize {
        match relation {
            Relation::Myself => 0,
            Relation::Parent => 1,
            Relation::Child => 2,
            Relation::Sibling => 3,
            Relation::Kin => 4,
            Relation::Root => 5,
            Relation::Cousin => 6,
            Relation::Elsewhere => 7,
        }
    }

    pub fn main_of(project: &str, id: &str) -> Whom {
        Whom {
            project: project.to_owned(),
            id: id.to_owned(),
            parent: None,
            session: None,
            root: None,
        }
    }

    pub fn under(project: &str, id: &str, parent: &str) -> Whom {
        Whom {
            project: project.to_owned(),
            id: id.to_owned(),
            parent: Some(parent.to_owned()),
            session: None,
            root: None,
        }
    }

    /// A subagent that says which run it was started under, however deep it sits.
    pub fn born_in(project: &str, id: &str, parent: &str, session: &str) -> Whom {
        Whom {
            session: Some(session.to_owned()),
            ..under(project, id, parent)
        }
    }
}
