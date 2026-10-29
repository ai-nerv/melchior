use super::Turn;
use std::collections::BTreeMap;
use std::io;
use std::time::Duration;
use tokio::sync::Barrier;

pub(super) struct Groups(BTreeMap<String, Barrier>);

impl Groups {
    pub fn new(turns: &[Turn]) -> io::Result<Self> {
        let mut counts = BTreeMap::new();
        for turn in turns {
            if let Some(name) = &turn.rendezvous {
                if name.is_empty()
                    || name.len() > 32
                    || !name.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')
                    || turn.when.as_ref().is_some_and(|w| w.is_rule() && !w.once)
                {
                    return Err(io::Error::other("invalid or reusable rendezvous turn"));
                }
                *counts.entry(name.clone()).or_insert(0) += 1;
            }
        }
        if counts.len() > 16 || counts.values().any(|n| !(2..=16).contains(n)) {
            return Err(io::Error::other(
                "rendezvous needs 2..16 single-use participants per group, at most 16 groups",
            ));
        }
        Ok(Self(
            counts
                .into_iter()
                .map(|(k, n)| (k, Barrier::new(n)))
                .collect(),
        ))
    }

    pub async fn arrive(&self, name: &str) -> io::Result<()> {
        let barrier = self
            .0
            .get(name)
            .ok_or_else(|| io::Error::other("unknown rendezvous"))?;
        tokio::time::timeout(Duration::from_secs(5), barrier.wait())
            .await
            .map_err(|_| io::Error::other("rendezvous_timeout"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn turns(value: serde_json::Value) -> Vec<Turn> {
        serde_json::from_value(value).expect("fixture")
    }

    #[test]
    fn rejects_unbounded_invalid_and_unmatched_participants() {
        for fixture in [
            json!([{"rendezvous":"pair"}]),
            json!([{"rendezvous":""},{"rendezvous":""}]),
            json!([{"rendezvous":"PAIR"},{"rendezvous":"PAIR"}]),
            json!([{"rendezvous":"pair","when":{"body":"lead"}},{"rendezvous":"pair"}]),
            json!(vec![json!({"rendezvous":"pair"}); 17]),
        ] {
            assert!(Groups::new(&turns(fixture)).is_err());
        }
        assert!(
            Groups::new(&turns(json!([
                {"rendezvous":"pair","when":{"body":"left","once":true}},
                {"rendezvous":"pair","when":{"body":"right","once":true}}
            ])))
            .is_ok()
        );
    }

    #[tokio::test]
    async fn both_participants_must_arrive_before_either_returns() {
        let groups = Groups::new(&turns(json!([{"rendezvous":"pair"},{"rendezvous":"pair"}])))
            .expect("groups");
        let first = groups.arrive("pair");
        tokio::pin!(first);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut first)
                .await
                .is_err()
        );
        let (left, right) = tokio::join!(first, groups.arrive("pair"));
        left.expect("left released");
        right.expect("right released");
        assert!(groups.arrive("missing").await.is_err());
    }

    #[tokio::test]
    async fn missing_participant_fails_with_a_finite_deadline() {
        let groups = Groups::new(&turns(json!([{"rendezvous":"pair"},{"rendezvous":"pair"}])))
            .expect("groups");
        assert_eq!(
            groups
                .arrive("pair")
                .await
                .expect_err("missing peer")
                .to_string(),
            "rendezvous_timeout"
        );
    }
}
