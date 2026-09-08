//! What the tool says back.
//!
//! The verbs that only report — no socket, no far end, nothing but this session reading
//! itself out. Split from [`super`] because the two answer different questions: that file is
//! about who may do what, and this one is about how it reads when they may.

use super::{Standing, TOOL, VERBS};
use crate::asking;
use crate::policy::{self, Reach, Relation};

/// Every verb, as the model should read it.
pub fn help(standing: &Standing) -> String {
    let rows: Vec<String> = VERBS
        .iter()
        .map(|(name, does)| format!("- `{name}` — {does}"))
        .collect();
    format!(
        "This session is `{me}`{born}.\n\n`{TOOL}` verbs:\n\n{rows}\n\nInstances are named \
         `project/role/id`. A bare id means one in this project, and there is no reaching \
         outside it.\n\nWho can be reached at all depends on where each of you sits: `list` says, and \
         it lists only what this session may actually reach. Only an instance this session \
         started can be stopped.",
        me = standing.me,
        born = standing
            .parent
            .as_ref()
            .map_or(String::new(), |who| format!(", started by `{who}`")),
        rows = rows.join("\n")
    )
}

/// Who is there, and how each of them stands to this session.
///
/// Only what can actually be reached. A model told about a cousin it will then be refused
/// spends the turn planning around a wall it was never going to get through, and the refusal
/// arrives too late to change the plan.
pub fn list(standing: &Standing) -> String {
    let me = standing.whom();
    let there = crate::directory::reachable(&me);
    if there.is_empty() {
        return format!(
            "Nothing else in `{}` can be reached from here. Either nothing else is running, \
             or what is running is behind another instance's door — see `whoami`.",
            me.project
        );
    }
    let me_named = standing.identity();
    let rows: Vec<String> = there
        .into_iter()
        .map(|(them, relation)| {
            let stoppable = if relation == Relation::Child {
                ", which this session may stop"
            } else {
                ""
            };
            let where_it_is = crate::directory::socket(&them.project, &them.id);
            // Asked rather than assumed. A socket file outlives the process that made it, so
            // the directory says who *was* here — and a model that sends to a name it read off
            // a stale entry is told the message landed when nothing received it.
            let alive = if asking::answers(&where_it_is, &me_named) {
                ""
            } else {
                " — not answering; its socket is what a crash left behind"
            };
            format!(
                "- `{}/{}` — {}{stoppable}{alive}",
                them.project,
                them.id,
                relation.named()
            )
        })
        .collect();
    format!(
        "In `{}`, reachable from here:\n\n{}",
        me.project,
        rows.join("\n")
    )
}

/// Everyone in this session's run: the root that started it and everything under it.
///
/// **The whole run, not what this session may reach**, and that is the one place this parts
/// company with [`list`]. The roster answers "who is on this job"; whether a given member may be
/// spoken to is a second question with its own answer, and a roster that hid the members
/// `agent_talk` refuses would have a coordinator planning around a crew it cannot see. Each row
/// that is out of reach says so and names the setting, which is the same courtesy a refusal gets.
///
/// What each member is *for* is not on disk. The directory records who started whom, and a role
/// invented here would be a second answer to a question nothing yet asks.
pub fn crew(standing: &Standing) -> String {
    let me = standing.whom();
    let held = crate::directory::sessions::crew(&me);
    if held.is_empty() {
        return format!(
            "Nothing in run `{}` is answering, not even this session — which means its own \
             socket is not up, and the roster is read through it.",
            me.session_root()
        );
    }
    let me_named = standing.identity();
    let setting = policy::talk();
    let rows: Vec<String> = held
        .iter()
        .map(|them| {
            let relation = policy::between(&me, them);
            // Only ours, and only because it is the one this process was told at startup. A
            // role read off a peer would be a role that peer chose for itself, and there is
            // nowhere on disk to read it from anyway.
            let role = if them.id == me.id {
                format!(" [{}]", me_named.role)
            } else {
                String::new()
            };
            let alive = if asking::answers(
                &crate::directory::socket(&them.project, &them.id),
                &me_named,
            ) {
                ""
            } else {
                " — not answering; its socket is what a crash left behind"
            };
            let refused = if relation == Relation::Myself || policy::may(&me, relation, Reach::Ask)
            {
                String::new()
            } else {
                format!(
                    " — out of reach while `magi.agent_talk` is \"{}\"",
                    setting.named()
                )
            };
            format!(
                "- `{}`{role} — {}{alive}{refused}",
                them.id,
                relation.named()
            )
        })
        .collect();
    format!(
        "Run `{}` in `{}`:\n\n{}\n\nWhat each of them is for is not recorded: the directory \
         says who started whom, not what they were started to do.",
        me.session_root(),
        me.project,
        rows.join("\n")
    )
}

/// What has been sent here.
pub fn inbox(standing: &Standing) -> String {
    if standing.inbox.is_empty() {
        return "Nothing has been sent to this session.".to_owned();
    }
    // Marked rather than sorted. Order is when things arrived, which is what makes a
    // conversation readable; the mark is what makes the urgent one findable in it.
    let rows: Vec<String> = standing
        .inbox
        .iter()
        .map(|message| {
            let mark = if message.sort.interrupts() { "! " } else { "" };
            let owed = if message.sort.expects_an_answer() {
                format!(" — answer with `reply`, `about: \"{}\"`", message.id)
            } else {
                String::new()
            };
            format!(
                "- {mark}`{}` [{}] {}{owed}",
                message.from,
                serde_json::to_value(message.sort)
                    .ok()
                    .and_then(|v| v.as_str().map(ToOwned::to_owned))
                    .unwrap_or_else(|| "note".to_owned()),
                message.text
            )
        })
        .collect();
    rows.join("\n")
}

/// This session's own name and place.
///
/// A subagent that does not know it is one cannot behave like one: it will not think to raise
/// `attention` at a parent it does not know it has, and it will try to `stop` siblings it has
/// no authority over. This is the first thing such a session should ask.
pub fn whoami(standing: &Standing) -> String {
    let mut said = format!("This session is `{}`.", standing.me);
    match &standing.parent {
        Some(who) => said.push_str(&format!(
            " It was started by `{who}`, which is the only session that can stop it — and the \
             one to raise `attention` at when this session needs a decision it cannot make."
        )),
        None => said.push_str(
            " Nothing started it, so it is a root session: no parent to escalate to, and \
             nothing can stop it from outside.",
        ),
    }
    if standing.forked.is_empty() {
        said.push_str(" It has started nothing, so there is nothing it may stop.");
    } else {
        said.push_str(&format!(
            " It started {}, which it may stop: {}.",
            standing.forked.len(),
            standing
                .forked
                .iter()
                .map(|child| format!("`{child}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    said
}
