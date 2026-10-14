//! Handing in a report, and asking for one. A report is a file and not a message: a message is
//! capped so an inbox stays readable, and an agent with a long result was cutting it into a dozen
//! of them, which arrived in its lead's conversation interleaved with everybody else's.
//!
//! One file per agent, in the project's runtime directory, so it outlives the agent that wrote it
//! and is gone when the machine is. Its lead is told once that it is there, and reads it when it
//! wants to, a page at a time.

use super::{Answer, Standing};
use crate::policy::{self, Reach, Relation, Whom};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;

#[derive(serde::Serialize, serde::Deserialize)]
struct Stored {
    format: String,
    revision: String,
    digest: String,
    report: String,
    /// Who started the writer and which run it was in, so a reader can be placed against it
    /// after it has gone.
    #[serde(default)]
    parent: Option<String>,
    #[serde(default)]
    session: Option<String>,
}

fn stored(body: String) -> Stored {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let digest = format!("{:x}", Sha256::digest(body.as_bytes()));
    let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let revision = format!(
        "{:x}",
        Sha256::digest(format!("{}:{serial}:{now}:{digest}", std::process::id()).as_bytes())
    );
    Stored {
        format: "melchior-report-v1".into(),
        revision,
        digest,
        report: body,
        parent: None,
        session: None,
    }
}

fn keep(at: &std::path::Path, value: &Stored) -> std::io::Result<()> {
    let temporary = at.with_extension(format!("{}.tmp", value.revision));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temporary)?;
        let bytes = serde_json::to_vec(value).map_err(std::io::Error::other)?;
        file.write_all(&bytes)?;
        std::fs::rename(&temporary, at)
    })();
    let _ = std::fs::remove_file(temporary);
    result
}

/// How many lines one reading gives back, and how many bytes at most: under what a harness will
/// carry as one tool result, so a long report is paged rather than cut in the middle.
const PAGE_LINES: usize = 600;
const PAGE_BYTES: usize = 36_000;

/// Where `id`'s report is kept.
fn kept_at(project: &str, id: &str) -> std::path::PathBuf {
    let id: String = id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    crate::directory::home(project).join(format!("{id}.report"))
}

/// `report`: with `who`, read that agent's; without, hand in this session's own.
pub fn report(arguments: &Value, standing: &Standing) -> Answer {
    let text = |key: &str| {
        arguments
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|said| !said.is_empty())
    };
    let message = arguments
        .get("message")
        .and_then(Value::as_str)
        .filter(|said| !said.trim().is_empty());
    match (text("who"), message) {
        (Some(_), Some(_)) => Answer::refused(
            "`report` with `who` reads that agent's report, and with `message` hands in this \
             session's own. Give one or the other.",
        ),
        (Some(who), None) => read(who, text("about"), standing),
        (None, Some(said)) => hand_in(said, standing),
        (None, None) => Answer::refused(
            "`report` needs the report: the whole of it in `message`, however long. Nothing \
             else is read for it — not a file, not standard input.",
        ),
    }
}

/// Keep this session's report, and tell whoever started it that it is there.
fn hand_in(said: &str, standing: &Standing) -> Answer {
    let me = standing.identity();
    let body = said.to_owned();
    let stored = Stored {
        parent: standing.parent.clone(),
        session: crate::directory::sessions::session_of(&me),
        ..stored(body.clone())
    };
    let at = kept_at(&me.project, &me.id);
    if let Err(why) = crate::places::ensure(&crate::directory::home(&me.project))
        .map_err(std::io::Error::other)
        .and_then(|()| keep(&at, &stored))
    {
        return Answer::refused(format!("the report could not be kept: {why}"));
    }
    let size = format!(
        "{} lines, {} characters",
        body.lines().count(),
        body.chars().count()
    );
    let told = standing.parent.as_deref().map(|lead| {
        let note = serde_json::json!({
            "verb": "send",
            "who": lead,
            "sort": "report",
            "message": serde_json::json!({
                "revision": stored.revision,
                "notice": format!("`{}` has handed in its report ({size}).", me.id),
            }).to_string(),
        });
        match super::doing::decide("send", lead, &note, standing) {
            Ok(wanted) => !super::doing::perform(&wanted, standing).failed,
            Err(_) => false,
        }
    });
    Answer::said(match told {
        Some(true) => format!(
            "Handed in ({size}). Whoever started this session has been told it is there and reads \
             it when it wants to — do not also send it as messages."
        ),
        Some(false) => format!(
            "Kept ({size}), but whoever started this session could not be told. It can still \
             read it with verb `report`, who `{}`.",
            me.id
        ),
        None => format!("Kept ({size}). Nobody started this session, so nobody was told."),
    })
}

/// Why this session may not read `id`'s report, if it may not: the same wall an `ask` meets, with
/// `id` placed by the tree as it is now or, for an agent that has gone, as its report recorded.
/// A report written in a run the agent now answering under that id was never part of is not its.
fn barred(id: &str, stored: Option<&Stored>, standing: &Standing) -> Option<Answer> {
    let me = standing.whom();
    let live = crate::directory::whom(&me.project, id);
    let kept = stored.and_then(|stored| stored.session.clone());
    if let (Some(kept), Some(now)) = (&kept, &live.session)
        && kept != now
    {
        return Some(Answer::refused(format!(
            "`{id}` has handed in no report in the run it is part of now."
        )));
    }
    let them = Whom {
        parent: live
            .parent
            .clone()
            .or_else(|| stored.and_then(|stored| stored.parent.clone())),
        session: live.session.clone().or(kept),
        ..live
    };
    let relation = if id == me.id {
        Relation::Myself
    } else if standing.forked.iter().any(|child| child == id) {
        Relation::Child
    } else {
        policy::between(&me, &them)
    };
    (!policy::may(&me, relation, Reach::Ask))
        .then(|| Answer::refused(policy::refusal(&me, relation, Reach::Ask)))
}

/// One page of `who`'s report, starting at the line `from` names.
fn read(who: &str, from: Option<&str>, standing: &Standing) -> Answer {
    let me = standing.identity();
    let id = who.rsplit('/').next().unwrap_or(who);
    let Ok(body) = std::fs::read_to_string(kept_at(&me.project, id)) else {
        return Answer::refused(format!(
            "`{id}` has handed in no report. `status` says whether it is still working; one that \
             has finished without reporting can be asked to with `ask`."
        ));
    };
    let stored = serde_json::from_str::<Stored>(&body)
        .ok()
        .filter(|stored| stored.format == "melchior-report-v1");
    if let Some(refused) = barred(id, stored.as_ref(), standing) {
        return refused;
    }
    let (body, revision, digest) = match stored {
        Some(stored) => (stored.report, stored.revision, stored.digest),
        None => {
            let digest = format!("{:x}", Sha256::digest(body.as_bytes()));
            (body, digest.clone(), digest)
        }
    };
    if from == Some("json") {
        return Answer::said(
            serde_json::json!({
                "agent": id,
                "revision": revision,
                "digest": digest,
                "report": body,
            })
            .to_string(),
        );
    }
    let lines: Vec<&str> = body.lines().collect();
    let first = from
        .and_then(|n| n.parse::<usize>().ok())
        .map_or(0, |n| n.saturating_sub(1))
        .min(lines.len());
    let mut bytes = 0;
    let taken = lines
        .iter()
        .skip(first)
        .take(PAGE_LINES)
        .take_while(|line| {
            bytes += line.len() + 1;
            bytes <= PAGE_BYTES || bytes == line.len() + 1
        })
        .count();
    let last = first + taken;
    let more = if last < lines.len() {
        format!(
            " More follows: call `report` again with who `{id}` and about `{}`.",
            last + 1
        )
    } else {
        " That is the end of it.".to_owned()
    };
    Answer::said(format!(
        "`{id}`'s report, lines {}–{last} of {}.{more}\n\n{}",
        first + 1,
        lines.len(),
        lines
            .iter()
            .skip(first)
            .take(taken)
            .copied()
            .collect::<Vec<_>>()
            .join("\n")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Project;

    fn standing(project: &str, id: &str) -> Standing {
        Standing {
            me: format!("{project}/main/{id}"),
            parent: None,
            forked: Vec::new(),
            minted: std::collections::BTreeMap::new(),
            inbox: Vec::new(),
        }
    }

    #[test]
    fn a_report_of_any_length_is_handed_in_whole_and_read_back_in_pages() {
        let project = Project::new("melchior-reporting", "paged");
        let scout = standing(&project, "delta-kappa");
        let body: String = (1..=1500).map(|n| format!("finding {n}\n")).collect();
        let kept = report(&serde_json::json!({"message": body}), &scout);
        assert!(!kept.failed, "{}", kept.said);
        assert!(kept.said.contains("1500 lines"), "{}", kept.said);

        let lead = standing(&project, "iota-omega");
        let page = report(&serde_json::json!({"who": "delta-kappa"}), &lead);
        assert!(
            page.said.contains("lines 1–600 of 1500"),
            "{:.120}",
            page.said
        );
        assert!(page.said.contains("finding 600\n") || page.said.ends_with("finding 600"));
        assert!(!page.said.contains("finding 601"));
        assert!(page.said.contains("about `601`"), "{:.200}", page.said);

        let rest = report(
            &serde_json::json!({"who": "delta-kappa", "about": "1201"}),
            &lead,
        );
        assert!(
            rest.said.contains("lines 1201–1500 of 1500"),
            "{:.120}",
            rest.said
        );
        assert!(rest.said.contains("That is the end of it"));
        assert!(rest.said.ends_with("finding 1500"));
    }

    #[test]
    fn asking_for_a_report_nobody_handed_in_says_so() {
        let project = Project::new("melchior-reporting", "none");
        let lead = standing(&project, "iota-omega");
        let none = report(&serde_json::json!({"who": "pi-mu"}), &lead);
        assert!(none.failed);
        assert!(none.said.contains("handed in no report"), "{}", none.said);
    }

    #[test]
    fn a_report_is_one_or_the_other_and_never_empty() {
        let project = Project::new("melchior-reporting", "shape");
        let scout = standing(&project, "pi-mu");
        assert!(report(&serde_json::json!({}), &scout).failed);
        assert!(report(&serde_json::json!({"who": "x", "message": "y"}), &scout).failed);
        // The name is an address and never a path.
        let lead = standing(&project, "iota-omega");
        assert!(report(&serde_json::json!({"who": "../../etc/passwd"}), &lead).failed);
    }

    #[test]
    fn a_report_is_never_read_from_a_file_or_standard_input() {
        let project = Project::new("melchior-reporting", "paths");
        let scout = standing(&project, "pi-mu");
        let secret = crate::scratch::Scratch::file("melchior-reporting", "paths", "secret.txt");
        std::fs::write(&secret, "the private key").expect("a file");
        for about in [secret.display().to_string(), "-".to_owned()] {
            let out = report(&serde_json::json!({"about": about}), &scout);
            assert!(out.failed, "`{about}` was taken as a report: {}", out.said);
        }
        let lead = standing(&project, "iota-omega");
        let read = report(&serde_json::json!({"who": "pi-mu", "about": "json"}), &lead);
        assert!(read.failed, "something was kept: {}", read.said);
    }

    #[test]
    fn handing_in_again_replaces_what_was_there() {
        let project = Project::new("melchior-reporting", "again");
        let scout = standing(&project, "pi-mu");
        report(&serde_json::json!({"message": "first draft"}), &scout);
        report(&serde_json::json!({"message": "final"}), &scout);
        let lead = standing(&project, "iota-omega");
        let read = report(&serde_json::json!({"who": "pi-mu"}), &lead);
        assert!(read.said.ends_with("final"), "{}", read.said);
    }

    #[test]
    fn a_report_is_read_only_across_the_walls_an_ask_crosses() {
        let project = Project::new("melchior-reporting", "walled");
        let mut scout = standing(&project, "gamma-xi");
        scout.parent = Some("alpha-rho".to_owned());
        let them = crate::identity::Identity::read(&scout.me).expect("a name");
        crate::directory::adopted(&them, "alpha-rho").expect("the note");
        assert!(!report(&serde_json::json!({"message": "mine"}), &scout).failed);

        let stranger = standing(&project, "iota-omega");
        let refused = report(&serde_json::json!({"who": "gamma-xi"}), &stranger);
        assert!(refused.failed, "another main read a subagent's report");
        assert!(refused.said.contains("agent_talk"), "{}", refused.said);

        let mut lead = standing(&project, "alpha-rho");
        lead.forked.push("gamma-xi".to_owned());
        let read = report(&serde_json::json!({"who": "gamma-xi"}), &lead);
        assert!(!read.failed, "{}", read.said);
    }

    #[test]
    fn a_report_left_by_an_earlier_agent_of_the_same_name_is_not_this_one_s() {
        let project = Project::new("melchior-reporting", "recycled");
        let scout = standing(&project, "pi-mu");
        let them = crate::identity::Identity::read(&scout.me).expect("a name");
        std::fs::write(crate::directory::sessions::session_at(&them), "pi-mu-1").expect("note");
        report(&serde_json::json!({"message": "old work"}), &scout);
        std::fs::write(crate::directory::sessions::session_at(&them), "pi-mu-2").expect("note");
        let lead = standing(&project, "iota-omega");
        let read = report(&serde_json::json!({"who": "pi-mu"}), &lead);
        assert!(
            read.failed,
            "a predecessor's report was read: {}",
            read.said
        );
    }

    #[test]
    fn old_plain_text_reports_remain_readable() {
        let project = Project::new("melchior-reporting", "legacy");
        std::fs::create_dir_all(crate::directory::home(&project)).expect("report directory");
        let body = "Legacy findings.\n";
        std::fs::write(kept_at(&project, "pi-mu"), body).expect("legacy report file");
        let lead = standing(&project, "iota-omega");
        let read = report(&serde_json::json!({"who": "pi-mu", "about": "json"}), &lead);
        let value: Value = serde_json::from_str(&read.said).expect("report JSON");
        assert_eq!(value["report"], body);
        assert_eq!(value["revision"], value["digest"]);
    }
}
