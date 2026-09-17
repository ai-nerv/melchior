//! Handing in a report, and asking for one. A report is a file and not a message: a message is
//! capped so an inbox stays readable, and an agent with a long result was cutting it into a dozen
//! of them, which arrived in its lead's conversation interleaved with everybody else's.
//!
//! One file per agent, in the project's runtime directory, so it outlives the agent that wrote it
//! and is gone when the machine is. Its lead is told once that it is there, and reads it when it
//! wants to, a page at a time.

use super::{Answer, Standing};
use serde_json::Value;

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
    match (text("who"), text("message")) {
        (Some(_), Some(_)) => Answer::refused(
            "`report` with `who` reads that agent's report, and with `message` hands in this \
             session's own. Give one or the other.",
        ),
        (Some(who), None) => read(who, text("about"), standing),
        (None, said) => hand_in(said, text("about"), standing),
    }
}

/// Keep this session's report, and tell whoever started it that it is there.
fn hand_in(said: Option<&str>, path: Option<&str>, standing: &Standing) -> Answer {
    let me = standing.identity();
    let body = match (said, path) {
        (Some(said), _) => said.to_owned(),
        (None, Some(path)) => match std::fs::read_to_string(path) {
            Ok(body) => body,
            Err(why) => return Answer::refused(format!("could not read `{path}`: {why}")),
        },
        (None, None) => {
            return Answer::refused(
                "`report` needs the report: the whole of it in `message`, however long, or the \
                 path of a file holding it in `about`.",
            );
        }
    };
    let at = kept_at(&me.project, &me.id);
    if let Err(why) = std::fs::create_dir_all(crate::directory::home(&me.project))
        .and_then(|()| std::fs::write(&at, &body))
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
            "sort": "attention",
            "message": format!(
                "`{}` has handed in its report ({size}). Read it with the `agent` tool: verb \
                 `report`, who `{}`.",
                me.id, me.id
            ),
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
    let lines: Vec<&str> = body.lines().collect();
    let first = from
        .and_then(|n| n.parse::<usize>().ok())
        .map_or(0, |n| n.saturating_sub(1))
        .min(lines.len());
    let mut bytes = 0;
    let taken = lines[first..]
        .iter()
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
        lines[first..last].join("\n")
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
    fn handing_in_again_replaces_what_was_there() {
        let project = Project::new("melchior-reporting", "again");
        let scout = standing(&project, "pi-mu");
        report(&serde_json::json!({"message": "first draft"}), &scout);
        report(&serde_json::json!({"message": "final"}), &scout);
        let lead = standing(&project, "iota-omega");
        let read = report(&serde_json::json!({"who": "pi-mu"}), &lead);
        assert!(read.said.ends_with("final"), "{}", read.said);
    }
}
