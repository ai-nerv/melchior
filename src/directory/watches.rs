//! Which agents a session has asked to be told about, beyond the automatic parent and children.
//! One note per session, `<id>.watches`, a target id a line — read by the roster diff to decide
//! whose phase changes to signal.

use super::{home, safe};
use std::collections::BTreeSet;
use std::path::PathBuf;

fn at(project: &str, id: &str) -> PathBuf {
    home(project).join(format!("{}.watches", safe(id)))
}

/// The ids `id` explicitly watches, off its note. Empty for one watching nothing extra.
#[must_use]
pub fn watched_by(project: &str, id: &str) -> BTreeSet<String> {
    std::fs::read_to_string(at(project, id))
        .map(|said| {
            said.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Add `target` to what `by` watches.
pub fn watch(project: &str, by: &str, target: &str) {
    let mut set = watched_by(project, by);
    set.insert(target.to_owned());
    write(project, by, &set);
}

/// Drop `target` from what `by` watches.
pub fn unwatch(project: &str, by: &str, target: &str) {
    let mut set = watched_by(project, by);
    set.remove(target);
    write(project, by, &set);
}

fn write(project: &str, by: &str, set: &BTreeSet<String>) {
    let path = at(project, by);
    if set.is_empty() {
        let _ = std::fs::remove_file(&path);
        return;
    }
    let _ = std::fs::create_dir_all(home(project));
    let _ = std::fs::write(&path, set.iter().cloned().collect::<Vec<_>>().join("\n"));
}

/// Forget what a gone session watched.
pub fn forget_in(project: &str, id: &str) {
    let _ = std::fs::remove_file(at(project, id));
}
