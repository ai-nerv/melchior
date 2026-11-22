use super::*;
use serde_json::json;

#[test]
fn actual_cached_tier_prices_settle_and_enforce_the_next_request_cap() {
    for (ending, rate, expected) in [
        (
            Ending::Whole,
            json!({"input":3_000_000,"output":0}),
            3_000_000,
        ),
        (
            Ending::Cached,
            json!({"input":2_000_000,"output":0,"cache_read":500_000}),
            500_000,
        ),
    ] {
        let world = World::new("tiered-prices", ending);
        let mut tier = rate;
        tier["input_tokens_above"] = json!(0);
        world.discovered(json!({"input":1_000_000,"output":0,"cache_read":100_000,"tiers":[tier]}));
        world.capped("melchior.spend_cap_usd=0.5");
        let envs = [("MAGI_MELCHIOR_SESSION", "tiered-cost-run")];
        let first = world.ask(&envs);
        assert_eq!(last(&first)["event"], "stop", "{first:?}");
        assert_eq!(world.ledger("tiered-cost-run").spent(), expected);
        let final_usage = first
            .iter()
            .rev()
            .find(|event| event["event"] == "spent")
            .expect("usage");
        assert_eq!(final_usage["usage"]["estimated_micros"], expected);
        let second = world.ask(&envs);
        assert_eq!(last(&second)["event"], "failed");
        assert!(
            last(&second)["message"]
                .as_str()
                .expect("reason")
                .contains("cap")
        );
        assert_eq!(world.asked(), 1);
    }
}

#[test]
fn actual_retry_tokens_do_not_select_an_aggregate_context_tier() {
    let world = World::new("tiered-retries", Ending::TierRetry);
    world.discovered(json!({"input":1_000_000,"output":0,
        "tiers":[{"input_tokens_above":1,"input":10_000_000}]}));
    let said = world.ask(&[("MAGI_MELCHIOR_SESSION", "tiered-retry-run")]);
    assert_eq!(last(&said)["event"], "stop", "{said:?}");
    assert_eq!(world.asked(), 2);
    assert_eq!(world.ledger("tiered-retry-run").spent(), 2_000_000);
    let final_usage = said
        .iter()
        .rev()
        .find(|event| event["event"] == "spent")
        .expect("usage");
    assert_eq!(final_usage["usage"]["input"], 2);
    assert_eq!(final_usage["usage"]["estimated_micros"], 2_000_000);
}

#[test]
fn a_final_provider_charge_replaces_only_its_own_tier_estimate() {
    let world = World::new("tiered-settlement", Ending::Settled);
    world.discovered(json!({"input":1_000_000,"output":0,
        "tiers":[{"input_tokens_above":0,"input":3_000_000}]}));
    let said = world.ask(&[("MAGI_MELCHIOR_SESSION", "tiered-settlement-run")]);
    assert_eq!(last(&said)["event"], "stop", "{said:?}");
    assert!(
        said.iter()
            .any(|event| event["event"] == "spent"
                && event["usage"]["estimated_micros"] == 3_000_000)
    );
    assert_eq!(world.ledger("tiered-settlement-run").spent(), 1_000_000);
    let final_usage = said
        .iter()
        .rev()
        .find(|event| event["event"] == "spent")
        .expect("usage");
    assert_eq!(final_usage["usage"]["cost_micros"], 1_000_000);
    assert_eq!(final_usage["usage"]["estimated_micros"], 0);
}
