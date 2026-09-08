//! Who may speak to whom.
//!
//! Two walls, and everything else is a setting.
//!
//! **The project wall.** A session can see and reach only what is inside its own project's
//! runtime directory. Not "should not" — cannot: the directory it lists and the directory it
//! dials are the one it belongs to, so an magi in `~/work/other` is not refused, it is not
//! there. Nothing in this file can turn that off, which is the point of putting it in the
//! filesystem rather than in a check.
//!
//! **The instance wall.** Inside a project, an *instance* is a main and the subagents it
//! started. A main is that instance's front door and is reachable by the other mains; the
//! subagents behind it are private, so `beta-nu`'s worker cannot be reached — or even
//! usefully named — by `alpha-rho`'s.
//!
//! ```text
//!   alpha-rho  <--->  beta-nu          two mains, two instances
//!      |                 |
//!      +- iota-mu        +- tau-chi     each main's own subagents
//!      +- zeta-pi        +- xi-phi
//! ```
//!
//! # What the setting moves
//!
//! The default is the tightest thing that still works: **mains talk to mains**, and a subagent
//! talks to whoever started it. That second one is not a peer relationship and no setting
//! governs it — a subagent that cannot report back to its parent cannot raise `attention` or
//! `trouble`, which is most of why it can speak at all.
//!
//! | `magi.agent_talk` | and also |
//! |---|---|
//! | `"mains"` *(default)* | — |
//! | `"instance"` | siblings, and kin: anything else started under the same root |
//! | `"project"` | cousins, and a subagent reaching another instance's main |
//!
//! It only ever opens things. There is no level below `mains`, because a project where nothing
//! can talk is a project that did not need any of this.

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

/// Take an answer from somewhere other than the environment, before anything asks.
///
/// The same `OnceLock` shape a UI's settings use, and the same trap: the first *read* fills it,
/// so this has to run before any call is answered.
pub fn adopt(talk: Talk) {
    let _ = CHOSEN.set(talk);
}

/// How far a session may reach.
///
/// From the environment, because the two processes that ask are started separately: the one
/// holding the socket and the one a model calls. A setting only one of them could see would
/// leave a tool refusing what the socket allows.
///
/// Anything unreadable is the default rather than a guess — a typo must not open a wall.
#[must_use]
pub fn talk() -> Talk {
    *CHOSEN.get_or_init(|| {
        crate::inherited::said(crate::inherited::TALK)
            .and_then(|name| Talk::read(&name))
            .unwrap_or_default()
    })
}

/// Who a session is, as far as the tree is concerned.
///
/// Not what it calls itself — what can be found out about it from the project directory. The
/// parent is the whole of it: everything else in this file is derived from comparing two of
/// these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Whom {
    /// Which project it belongs to.
    pub project: String,
    /// Its id, which is what its socket is called.
    pub id: String,
    /// Who started it, or `None` if it is a main.
    pub parent: Option<String>,
    /// Which run it belongs to: the id of the root the whole of it was started under.
    ///
    /// `None` where no note said. That is not "belongs to nothing" — see [`Whom::session_root`].
    pub session: Option<String>,
}

impl Whom {
    /// Whether this is a main — an instance's front door — rather than somebody's subagent.
    #[must_use]
    pub fn is_main(&self) -> bool {
        self.parent.is_none()
    }

    /// Which run this belongs to, with the missing case decided.
    ///
    /// A session with no note is **its own root**, not a session belonging to nothing. Two of
    /// them would otherwise share the answer `None` and read as one run — every main started
    /// before this note existed becoming kin to every other main in the project, which is the
    /// instance wall gone. Answering with the id makes the absent case degrade to what it
    /// described before there was a note: a tree of one.
    #[must_use]
    pub fn session_root(&self) -> &str {
        self.session.as_deref().unwrap_or(&self.id)
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
    ///
    /// The rung the tree always had and could never name. A grandchild and its grandparent, or
    /// two agents four generations apart under one root, were `Cousin` — the same word as an
    /// agent from a run this one has nothing to do with. `Cousin` now means only that.
    Kin,
    /// Another session in this project that nobody started: a front door of its own.
    ///
    /// Named for the tree rather than for the word on a status line. `Main` sat beside
    /// `me.is_main()` and `role: "main"` — three things spelt the same, of which only this one
    /// meant "has no parent" — and a reader had to work out which was which every time.
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
            // The word stays `main`. It is what `kin` has always answered and what a sibling
            // reads off the wire; the rename above is Rust's, and changing the wire with it
            // would break a consumer to make a variant read better here.
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
    // Both have a parent and it is the same one. `None == None` would make every pair of mains
    // siblings, which is why the parent is unwrapped rather than compared as an option.
    if let (Some(mine), Some(theirs)) = (me.parent.as_deref(), them.parent.as_deref())
        && mine == theirs
    {
        return Relation::Sibling;
    }
    // Compared by root and not by walking parentage, which is what makes this survive adoption:
    // the note that says which run an agent belongs to is written once and never moved, so a
    // grandchild handed to a new parent is still in the run it did its work in.
    if me.session_root() == them.session_root() {
        return Relation::Kin;
    }
    if them.is_main() {
        return Relation::Root;
    }
    Relation::Cousin
}

/// Whether `me` may do `reach` to something standing in `relation` to it.
///
/// The one place that answers "may I", so a verb added later cannot quietly forget to check.
#[must_use]
pub fn may(me: &Whom, relation: Relation, reach: Reach) -> bool {
    may_at(me, relation, reach, talk())
}

/// The same question with the setting handed in, so every level can be tested.
#[must_use]
pub fn may_at(me: &Whom, relation: Relation, reach: Reach, talk: Talk) -> bool {
    if reach == Reach::Stop {
        // The one act the far end cannot decline, so it is the one narrowed to the spawn link.
        // Even then it is not enough on its own: the caller has to hold the secret handed down
        // when the session was started, which is checked where the call is answered.
        return relation == Relation::Child;
    }
    match relation {
        // Never, at any setting. This is the project wall, and it is the reason there is no
        // level above `project`.
        Relation::Elsewhere => false,
        Relation::Myself | Relation::Parent | Relation::Child => true,
        Relation::Sibling => matches!(talk, Talk::Instance | Talk::Project),
        // The same row as a sibling, because `instance` already meant "the run I am in" and a
        // sibling is only the shallowest case of that. A fourth level for the deeper ones would
        // be a level whose whole content is "and two rungs further down", and the matrix is the
        // one place in this crate where an extra dimension is paid for everywhere.
        Relation::Kin => matches!(talk, Talk::Instance | Talk::Project),
        // A main is a front door to the other mains at every setting. A subagent knocking on
        // somebody else's front door is crossing the instance wall, so it waits for `project`.
        Relation::Root => me.is_main() || talk == Talk::Project,
        Relation::Cousin => talk == Talk::Project,
    }
}

/// Why it was refused, and what would have allowed it.
///
/// Says the setting by name, because "refused" with no way forward is how somebody concludes
/// the feature is broken rather than switched off.
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

/// The walls hold, and the setting only ever opens things.
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
        // The instance wall. Both are in one project and neither is behind the other's door.
        let mine = under("magi", "iota-mu", "alpha-rho");
        let theirs = under("magi", "tau-chi", "beta-nu");
        assert_eq!(between(&mine, &theirs), Relation::Cousin);
    }

    #[test]
    fn two_agents_of_one_run_are_kin_however_deep_they_sit() {
        // The rung the tree always had and could never name. Neither is the other's parent,
        // child or sibling, and before there was a note they read as `cousin` — the same word
        // as an agent from a run this one has nothing to do with.
        let mine = born_in("magi", "iota-mu", "alpha-rho", "alpha-rho");
        let far = born_in("magi", "zeta-pi", "tau-chi", "alpha-rho");
        assert_eq!(between(&mine, &far), Relation::Kin);
        assert_eq!(between(&far, &mine), Relation::Kin);

        // And the root of the run reaches down to it, which was `cousin` too: an orchestrator
        // was a stranger to its own grandchildren.
        let root = Whom {
            session: Some("alpha-rho".to_owned()),
            ..main_of("magi", "alpha-rho")
        };
        assert_eq!(between(&root, &far), Relation::Kin);
    }

    #[test]
    fn a_cousin_is_now_only_somebody_from_another_run() {
        // The half that makes the split worth having: same project, different root, and the
        // word `cousin` finally means that and nothing else.
        let mine = born_in("magi", "iota-mu", "alpha-rho", "alpha-rho");
        let theirs = born_in("magi", "tau-chi", "beta-nu", "beta-nu");
        assert_eq!(between(&mine, &theirs), Relation::Cousin);

        // And the pair that used to answer the same word does not any more. Both halves,
        // because a `between` that never produced `Kin` would still pass the line above.
        let ours = born_in("magi", "zeta-pi", "tau-chi", "alpha-rho");
        assert_ne!(between(&mine, &ours), Relation::Cousin);
    }

    #[test]
    fn kin_is_the_setting_a_sibling_needs_and_not_the_one_a_cousin_does() {
        // Two rungs that read alike and are not: `instance` opens the run, `project` opens the
        // wall around it. A `Kin` that waited for `project` would make the roster unusable at
        // the setting whose whole name is the run.
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
        // Every main started before the note existed answers `None`. Read as one shared value
        // they would all be kin to each other, and the instance wall would be gone for exactly
        // the sessions that predate the thing meant to draw it.
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
        // The bug an option comparison would have written: `None == None` makes every pair of
        // mains siblings, and then `"instance"` quietly becomes `"project"` for them.
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
        // The project wall, and the reason it is enforced by the directory as well as here.
        let me = main_of("magi", "alpha-rho");
        for reach in [Reach::Ask, Reach::Tell, Reach::Stop] {
            assert!(!may(&me, Relation::Elsewhere, reach), "{reach:?} escaped");
        }
    }

    #[test]
    fn the_spawn_link_works_without_any_setting() {
        // The default is `mains`, and a subagent that cannot report back to its parent cannot
        // raise attention or trouble, which is most of why it can speak.
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
        // The failure that matters: a typo must not open the walls.
        assert_eq!(Talk::read("everything"), None);
        assert_eq!(Talk::read("Project"), None);
        assert_eq!(Talk::read("project"), Some(Talk::Project));
    }

    #[test]
    fn a_refusal_names_the_setting_that_would_have_allowed_it() {
        // Otherwise somebody concludes the feature is broken rather than switched off.
        let child = under("magi", "iota-mu", "alpha-rho");
        let said = refusal(&child, Relation::Sibling, Reach::Tell);
        assert!(said.contains("agent_talk"), "{said}");
        assert!(said.contains("instance"), "{said}");
    }

    #[test]
    fn a_refusal_across_projects_does_not_offer_a_setting_that_would_help() {
        // There is none, and suggesting one would be a lie.
        let me = main_of("magi", "alpha-rho");
        let said = refusal(&me, Relation::Elsewhere, Reach::Ask);
        assert!(!said.contains("agent_talk"), "{said}");
        assert!(said.contains("across projects"), "{said}");
    }
}

/// Each setting opens exactly what it says it does, and nothing beyond it.
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
        // The two things a config cannot buy.
        let me = main_of("magi", "alpha-rho");
        for talk in [Talk::Mains, Talk::Instance, Talk::Project] {
            assert!(!may_at(&me, Relation::Elsewhere, Reach::Ask, talk));
            assert!(!may_at(&me, Relation::Root, Reach::Stop, talk));
            assert!(!may_at(&me, Relation::Sibling, Reach::Stop, talk));
        }
    }

    #[test]
    fn each_step_only_ever_adds() {
        // A looser setting that refused something a tighter one allowed would be a trap.
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

/// Builders both test modules use.
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

    /// Where a rung sits in [`EVERY`].
    ///
    /// A match rather than a lookup, and that is the whole of why it exists: a rung added to
    /// the enum stops compiling here until somebody says where it goes, so the matrix walk
    /// below stays exhaustive instead of merely long. `Kin` was added to an enum whose tests
    /// listed six of seven relations by hand.
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
        }
    }

    pub fn under(project: &str, id: &str, parent: &str) -> Whom {
        Whom {
            project: project.to_owned(),
            id: id.to_owned(),
            parent: Some(parent.to_owned()),
            session: None,
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
