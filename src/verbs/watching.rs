//! Watching another agent — asking to be told when its phase changes, beyond the parent and
//! children a session is signalled about anyway. A note, not a message: nothing is sent or dialled.

use super::{Answer, Standing};
use crate::directory::watches;

/// Start being told about `who`'s phase changes.
pub fn watch(arguments: &serde_json::Value, standing: &Standing) -> Answer {
    let me = standing.identity();
    let Some(target) = target(arguments) else {
        return Answer::refused("`watch` needs `who` — the id to be told about.".to_owned());
    };
    if target == me.id {
        return Answer::refused("a session hears about its own turns already.".to_owned());
    }
    watches::watch(&me.project, &me.id, &target);
    Answer::said(format!(
        "Watching `{target}`: its `working`/`finished`/`blocked` changes now reach this session, \
         the way a child's do. `unwatch` with the same id to stop."
    ))
}

/// Stop being told about `who`.
pub fn unwatch(arguments: &serde_json::Value, standing: &Standing) -> Answer {
    let me = standing.identity();
    let Some(target) = target(arguments) else {
        return Answer::refused(
            "`unwatch` needs `who` — the id to stop being told about.".to_owned(),
        );
    };
    watches::unwatch(&me.project, &me.id, &target);
    Answer::said(format!("No longer watching `{target}`."))
}

/// The bare id to watch, from `who` given as `id`, `role/id` or `project/role/id`.
fn target(arguments: &serde_json::Value) -> Option<String> {
    arguments
        .get("who")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|who| !who.is_empty())
        .map(|who| who.rsplit('/').next().unwrap_or(who).to_owned())
}
