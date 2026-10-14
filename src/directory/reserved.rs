//! Names handed out and not answering yet: `<id>.reserved`, made with `O_EXCL` so two sessions
//! naming children at the same moment cannot both be handed one name. A session takes its own
//! down once it has bound; one that never came up is forgotten after [`FRESH`].

use super::{home, safe};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

/// How long a name stays held for a session that has not come up.
pub const FRESH: std::time::Duration = std::time::Duration::from_secs(300);

/// Where the reservation for `id` is.
#[must_use]
pub fn at(project: &str, id: &str) -> PathBuf {
    home(project).join(format!("{}.reserved", safe(id)))
}

/// Hold `id` in `project`. True when this call made the reservation, false when somebody holds it.
///
/// # Errors
/// When the project directory or the reservation cannot be made for another reason.
pub fn reserve(project: &str, id: &str) -> Result<bool, String> {
    crate::places::ensure(&home(project))?;
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(at(project, id))
    {
        Ok(_) => Ok(true),
        Err(why) if why.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(why) => Err(format!("{}: {why}", at(project, id).display())),
    }
}

/// Whether `id` is held and the hold is still fresh.
#[must_use]
pub fn fresh(project: &str, id: &str) -> bool {
    std::fs::metadata(at(project, id))
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|made| made.elapsed().ok())
        .is_some_and(|age| age < FRESH)
}

/// Every name held in `project`, letting go of the holds that have gone stale.
#[must_use]
pub fn taken(project: &str) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(home(project)) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".reserved").map(ToOwned::to_owned)
        })
        .filter(|id| {
            let held = fresh(project, id);
            if !held {
                release(project, id);
            }
            held
        })
        .collect()
}

/// Let go of `id`: it has bound, or it is not coming.
pub fn release(project: &str, id: &str) {
    let _ = std::fs::remove_file(at(project, id));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Project;

    #[test]
    fn a_name_is_held_by_one_reservation_at_a_time() {
        let project = Project::new("melchior-reserved", "once");
        assert_eq!(reserve(&project, "iota-mu"), Ok(true));
        assert_eq!(reserve(&project, "iota-mu"), Ok(false), "held twice");
        assert!(fresh(&project, "iota-mu"));
        assert_eq!(taken(&project), ["iota-mu"]);
        release(&project, "iota-mu");
        assert!(taken(&project).is_empty());
    }

    #[test]
    fn names_handed_out_together_are_all_different() {
        let project = Project::new("melchior-reserved", "burst");
        let name = project.to_string();
        let picked: Vec<String> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..16)
                .map(|_| scope.spawn(|| crate::directory::free_in(&name).id))
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().expect("a name"))
                .collect()
        });
        let mut unique = picked.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(
            unique.len(),
            picked.len(),
            "a name went out twice: {picked:?}"
        );
    }
}

#[cfg(test)]
mod beside {
    use super::*;
    use crate::scratch::Project;

    #[test]
    fn a_child_is_never_named_after_the_session_naming_it() {
        let project = Project::new("melchior-reserved", "beside");
        let me = crate::directory::free_in(&project).id;
        release(&project, &me);
        for _ in 0..64 {
            let child = crate::directory::free_beside(&project, std::slice::from_ref(&me)).id;
            assert_ne!(child, me);
            release(&project, &child);
        }
    }
}
