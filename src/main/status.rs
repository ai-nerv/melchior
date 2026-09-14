//! A peer's live status, asked of its own socket: whether it is in a turn and for how long, what is
//! waiting for it, its phase, and what it has spent.
//!
//! Split from [`super`] under THE RULE, which caps a file at 800 lines.

/// A peer's live status as its own socket reports it. Defaulted for one that did not answer in time.
#[derive(Default)]
pub(super) struct Status {
    pub busy: bool,
    pub working_for: u64,
    pub waiting: usize,
    pub phase: Option<String>,
    pub cause: Option<String>,
    /// What it has spent, a row per model, exactly as its harness said; empty from an older one.
    pub spent: Vec<serde_json::Value>,
}

/// A peer's status, asked of its own socket. `None` when it did not answer within the dial's
/// patience — that peer simply reads as idle rather than stalling the roster.
pub(super) fn status_of(
    project: &str,
    id: &str,
    me: &melchior::identity::Identity,
) -> Option<Status> {
    let them = melchior::identity::Identity {
        project: project.to_owned(),
        role: String::new(),
        id: id.to_owned(),
    };
    let reply = melchior::directory::dial(&them, me)
        .ok()?
        .call("status", Vec::new())
        .ok()?;
    reply.result.first().map(read)
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
    use super::read;

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
