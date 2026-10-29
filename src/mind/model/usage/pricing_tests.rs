use super::*;
use serde_json::json;

fn cached() -> Usage {
    Usage {
        cache_read: 1_000_000,
        cache_write: 100_000,
        ..Usage::default()
    }
}

#[test]
fn absent_cached_rates_use_input_but_explicit_zero_stays_free() {
    for (raw, expected) in [
        (json!({"input": 3}), 3.3),
        (json!({"input": 3, "cache_read": 0, "cache_write": 0}), 0.0),
        (json!({"input": 3, "cache_read": 1, "cache_write": 5}), 1.5),
        (json!({}), 0.0),
    ] {
        let cost: Cost = serde_json::from_value(raw.clone()).expect("source prices");
        assert!((cached().price(cost) - expected).abs() < 1e-9, "{raw}");
        let reloaded: Cost = serde_json::from_value(serde_json::to_value(cost).expect("serialize"))
            .expect("reload prices");
        assert_eq!(cost, reloaded);
        assert!((cached().price(reloaded) - expected).abs() < 1e-9);
    }
}

#[test]
fn generated_unknown_rates_serialize_versioned_presence() {
    let cost = Cost {
        input: 3.0,
        ..Cost::default()
    };
    let value = serde_json::to_value(cost).expect("prices");
    assert_eq!(value["presence"]["version"], 1);
    assert_eq!(value["presence"]["cache_read"], false);
    assert_eq!(value["presence"]["cache_write"], false);
    assert!((cached().price(cost) - 3.3).abs() < 1e-9);
}

#[test]
fn positive_programmatic_rates_remain_usable_without_presence() {
    let cost = Cost {
        input: 3.0,
        cache_read: 1.0,
        cache_write: 5.0,
        ..Cost::default()
    };
    assert_eq!(cached().price(cost), 1.5);
}

#[test]
fn an_unknown_presence_version_does_not_claim_free_cached_tokens() {
    let raw = json!({"input":3,"cache_read":0,"cache_write":0,
        "presence":{"version":2,"cache_read":true,"cache_write":true}});
    let cost: Cost = serde_json::from_value(raw).expect("future metadata");
    assert!((cached().price(cost) - 3.3).abs() < 1e-9);
}

#[test]
fn cached_price_metadata_is_additive_for_old_readers() {
    #[derive(Deserialize)]
    struct OldCost {
        input: f64,
        output: f64,
        cache_read: f64,
        cache_write: f64,
    }
    let cost: Cost = serde_json::from_value(json!({"input":3,"output":15,"cache_read":0}))
        .expect("source prices");
    let old: OldCost =
        serde_json::from_value(serde_json::to_value(cost).expect("prices")).expect("old reader");
    assert_eq!(
        (old.input, old.output, old.cache_read, old.cache_write),
        (3.0, 15.0, 0.0, 0.0)
    );
}

#[test]
fn maximum_prompt_occupancy_saturates_instead_of_overflowing() {
    let usage = Usage {
        input: u64::MAX,
        cache_read: 1,
        cache_write: 1,
        ..Usage::default()
    };
    assert_eq!(usage.prompt_tokens(), u64::MAX);
}

#[test]
fn every_counter_accepts_integral_numbers_and_refuses_invalid_ones() {
    for field in [
        "input",
        "output",
        "cache_read",
        "cache_write",
        "cost_micros",
    ] {
        for valid in [json!(0), json!(1234), json!(1234.0), json!(u64::MAX)] {
            let mut value = json!({"input":0,"output":0,"cache_read":0,"cache_write":0});
            value[field] = valid.clone();
            let usage: Usage = serde_json::from_value(value).expect("integral count");
            assert_eq!(
                serde_json::to_value(usage).expect("count")[field].as_u64(),
                valid.as_u64().or(Some(1234))
            );
        }
        for invalid in [json!(-1), json!(0.5), json!(1e100), json!(true), json!("1")] {
            let mut value = json!({"input":0,"output":0,"cache_read":0,"cache_write":0});
            value[field] = invalid.clone();
            assert!(
                serde_json::from_value::<Usage>(value).is_err(),
                "{field} {invalid}"
            );
        }
    }
}
