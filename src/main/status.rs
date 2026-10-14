//! A peer's live status, asked of its own socket: whether it is in a turn and for how long, what is
//! waiting for it, its phase, and what it has spent.
//!
//! Split from [`super`] under THE RULE, which caps a file at 800 lines.

/// A peer's live status as its own socket reports it. Defaulted for one that did not answer in time.
#[derive(Default, Clone)]
pub(super) struct Status {
    pub busy: bool,
    pub working_for: u64,
    pub waiting: usize,
    pub phase: Option<String>,
    pub cause: Option<String>,
    /// What it has spent, a row per model, exactly as its harness said; empty from an older one.
    pub spent: Vec<serde_json::Value>,
}

/// How long a refusal is believed before the peer is asked again.
const REFUSAL_HELD: std::time::Duration = std::time::Duration::from_secs(30);

std::thread_local! {
    /// Peers that refused this session their status, when, and where each stood in the tree then.
    /// Asked again once the refusal is stale or either side has moved, as an adoption moves them.
    static REFUSED: std::cell::RefCell<
        std::collections::BTreeMap<String, (std::time::Instant, Placed)>,
    > = const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
}

/// Who started the peer, and who started this session, as the notes said.
type Placed = (Option<String>, Option<String>);

/// A peer's status, asked of its own socket. `None` when it did not answer within the dial's
/// patience, or refused — the roster then keeps what that peer said last.
pub(super) fn status_of(
    project: &str,
    id: &str,
    me: &melchior::identity::Identity,
) -> Option<Status> {
    let placed = (
        melchior::directory::parent_note(project, id),
        melchior::directory::parent_note(project, &me.id),
    );
    if REFUSED.with(|refused| still_refused(refused.borrow().get(id), &placed)) {
        return None;
    }
    let them = melchior::identity::Identity {
        project: project.to_owned(),
        role: String::new(),
        id: id.to_owned(),
    };
    // Bounded well inside the poll, so a stopped peer costs the roster a moment, not the tick.
    let at = melchior::directory::listening_at(&them);
    let reply = melchior::asking::Held::within(&at, me, std::time::Duration::from_secs(2))
        .ok()?
        .call("status", Vec::new())
        .ok()?;
    if !reply.ok {
        REFUSED.with(|refused| {
            refused
                .borrow_mut()
                .insert(id.to_owned(), (std::time::Instant::now(), placed));
        });
        return None;
    }
    REFUSED.with(|refused| refused.borrow_mut().remove(id));
    reply.result.first().map(read)
}

/// Whether a refusal held is still to be believed: recent, and with neither side moved since.
fn still_refused(held: Option<&(std::time::Instant, Placed)>, placed: &Placed) -> bool {
    held.is_some_and(|(at, then)| at.elapsed() < REFUSAL_HELD && then == placed)
}

/// A `status` reply, field by field; anything missing reads as nothing.
fn read(said: &serde_json::Value) -> Status {
    let word = |key: &str| {
        said.get(key)
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned)
    };
    Status {
        busy: said
            .get("busy")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        working_for: said
            .get("working_for")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        waiting: usize::try_from(
            said.get("waiting")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
        )
        .unwrap_or(0),
        phase: word("phase"),
        cause: word("cause"),
        spent: said
            .get("spent")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::{REFUSAL_HELD, read, still_refused};

    #[test]
    fn a_refusal_is_asked_again_once_stale_or_once_either_side_moved() {
        let now = std::time::Instant::now();
        let placed = (Some("alpha-rho".to_owned()), None);
        assert!(still_refused(Some(&(now, placed.clone())), &placed));
        assert!(!still_refused(None, &placed));
        let adopted = (Some("beta-nu".to_owned()), None);
        assert!(
            !still_refused(Some(&(now, placed.clone())), &adopted),
            "an adoption did not clear it"
        );
        if let Some(long_ago) = now.checked_sub(REFUSAL_HELD * 2) {
            assert!(!still_refused(Some(&(long_ago, placed.clone())), &placed));
        }
    }

    #[test]
    fn what_a_peer_spent_is_carried_as_it_said_it() {
        let said = serde_json::json!({
            "busy": true,
            "spent": [{"model": "a/b", "input": 12, "output": 3, "cost_micros": 150}],
        });
        let status = read(&said);
        assert!(status.busy);
        assert_eq!(status.spent.len(), 1);
        assert_eq!(status.spent[0]["model"], "a/b");
        assert_eq!(status.spent[0]["cost_micros"], 150);
    }

    #[test]
    fn a_peer_that_says_nothing_of_spending_spent_nothing() {
        let status = read(&serde_json::json!({"busy": false, "working_for": 3}));
        assert!(status.spent.is_empty());
        assert_eq!(status.working_for, 3);
    }
}
