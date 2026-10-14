//! The adoption a session has asked for and not been answered on: `<id>.adopting`, a `0600` note
//! naming who was asked and the nonce the answer has to quote. Written by the asker and read only
//! by the asker's own `serve`, so an `adopted` that cannot quote it was never asked for.

use super::{home, safe};
use crate::identity::Identity;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

/// One adoption asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asked {
    /// The id of the session asked to become the parent.
    pub to: String,
    /// What its answer must quote.
    pub nonce: String,
}

/// Where `me`'s note is.
#[must_use]
pub fn at(me: &Identity) -> PathBuf {
    home(&me.project).join(format!("{}.adopting", safe(&me.id)))
}

/// Record that `me` asked `to`, replacing any earlier asking, and hand back the nonce to send.
///
/// # Errors
/// When the nonce or the note cannot be made.
pub fn asked(me: &Identity, to: &str) -> Result<String, String> {
    let nonce = crate::identity::secret();
    let path = at(me);
    crate::places::ensure(&home(&me.project))?;
    let temporary = path.with_extension(format!("adopting.{nonce}"));
    let wrote = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .and_then(|mut file| file.write_all(format!("{to}\n{nonce}\n").as_bytes()))
        .and_then(|()| std::fs::rename(&temporary, &path));
    let _ = std::fs::remove_file(&temporary);
    wrote.map_err(|why| format!("{}: {why}", path.display()))?;
    Ok(nonce)
}

/// What `me` asked for, if anything.
#[must_use]
pub fn read(me: &Identity) -> Option<Asked> {
    parse(&std::fs::read_to_string(at(me)).ok()?)
}

fn parse(text: &str) -> Option<Asked> {
    let mut lines = text.lines().map(str::trim);
    let to = lines.next().filter(|it| !it.is_empty())?.to_owned();
    let nonce = lines.next().filter(|it| !it.is_empty())?.to_owned();
    Some(Asked { to, nonce })
}

/// Take the note, once: true only for the one caller whose `to` and `nonce` match it. A rename
/// moves it aside first, so two answers quoting it cannot both be taken.
#[must_use]
pub fn consume(me: &Identity, to: &str, nonce: &str) -> bool {
    let matches = |asked: &Asked| asked.to == to && crate::identity::same(&asked.nonce, nonce);
    if !read(me).is_some_and(|asked| matches(&asked)) {
        return false;
    }
    let path = at(me);
    let taken = path.with_extension(format!("adopting.taken.{}", std::process::id()));
    if std::fs::rename(&path, &taken).is_err() {
        return false;
    }
    let held = std::fs::read_to_string(&taken)
        .ok()
        .and_then(|text| parse(&text));
    let took = held.as_ref().is_some_and(matches);
    if !took {
        let _ = std::fs::hard_link(&taken, &path);
    }
    let _ = std::fs::remove_file(&taken);
    took
}

/// Withdraw `me`'s asking of `to`, and only of `to`: what a session does on taking `to` on itself.
pub fn withdraw(me: &Identity, to: &str) {
    if read(me).is_some_and(|asked| asked.to == to) {
        let _ = std::fs::remove_file(at(me));
    }
}

/// Whether `parent` may take `child` on as the directory stands now: both mains, no ring, the
/// grafted branch inside [`super::MAX_DEPTH`], and room under `parent` for one more beside the
/// `also` it has started and not yet seen come up.
///
/// # Errors
/// The reason, as the refusal says it.
pub fn adoptable(parent: &Identity, child: &Identity, also: usize) -> Result<(), String> {
    if parent.project != child.project || parent.id == child.id {
        return Err(format!("`{}` cannot take itself on", child.id));
    }
    if let Some(above) = super::parent_note(&parent.project, &parent.id) {
        return Err(format!(
            "`{}` already answers to `{above}`, so it cannot take another session on",
            parent.id
        ));
    }
    if let Some(above) = super::parent_note(&child.project, &child.id) {
        return Err(format!(
            "`{}` already answers to `{above}`, and only a main may be taken on",
            child.id
        ));
    }
    if super::root_of(&parent.project, &parent.id).as_deref() == Some(child.id.as_str()) {
        return Err(format!("`{}` is above `{}` already", child.id, parent.id));
    }
    let deep = super::depth_of(parent) + 1 + super::height_below(&child.project, &child.id);
    if deep >= super::MAX_DEPTH {
        return Err(format!(
            "taking `{}` on would make a branch {deep} deep, and a tree of agents goes {} levels \
             and no further",
            child.id,
            super::MAX_DEPTH
        ));
    }
    let running = super::children(parent).len() + also;
    if running >= super::MAX_CHILDREN {
        return Err(format!(
            "`{}` already has {running} children, the most one may hold at once",
            parent.id
        ));
    }
    Ok(())
}

/// Take the note down for a session that is gone.
pub fn forget_in(project: &str, id: &str) {
    let _ = std::fs::remove_file(home(project).join(format!("{}.adopting", safe(id))));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Project;
    use std::os::unix::fs::MetadataExt;

    fn me(project: &Project) -> Identity {
        Identity {
            project: project.to_string(),
            role: "main".to_owned(),
            id: "psi-eta".to_owned(),
        }
    }

    #[test]
    fn an_asking_is_kept_where_only_this_user_reads_it() {
        let project = Project::new("melchior-pending", "kept");
        let nonce = asked(&me(&project), "beta-nu").expect("asked");
        assert_eq!(nonce.len(), 32);
        let mode = std::fs::metadata(at(&me(&project)))
            .expect("the note")
            .mode();
        assert_eq!(mode & 0o777, 0o600, "{mode:o}");
        assert_eq!(
            read(&me(&project)),
            Some(Asked {
                to: "beta-nu".to_owned(),
                nonce
            })
        );
    }

    #[test]
    fn only_the_answer_quoting_the_nonce_from_the_one_asked_takes_it_and_only_once() {
        let project = Project::new("melchior-pending", "once");
        let nonce = asked(&me(&project), "beta-nu").expect("asked");
        assert!(!consume(&me(&project), "beta-nu", "guessed"), "a guess");
        assert!(
            !consume(&me(&project), "gamma-xi", &nonce),
            "the wrong session"
        );
        assert!(
            read(&me(&project)).is_some(),
            "a wrong answer takes nothing"
        );
        assert!(consume(&me(&project), "beta-nu", &nonce));
        assert!(!consume(&me(&project), "beta-nu", &nonce), "taken twice");
    }

    #[test]
    fn withdrawing_one_asking_leaves_another_alone() {
        let project = Project::new("melchior-pending", "withdraw");
        asked(&me(&project), "beta-nu").expect("asked");
        withdraw(&me(&project), "gamma-xi");
        assert!(read(&me(&project)).is_some());
        withdraw(&me(&project), "beta-nu");
        assert!(read(&me(&project)).is_none());
    }
}
