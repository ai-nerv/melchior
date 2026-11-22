use super::*;
use crate::mind::model::Usage;
use serde_json::json;

fn schedule() -> Cost {
    serde_json::from_value(json!({"input":1,"output":2,"cache_read":0.1,"cache_write":1.25,
        "tiers":[{"input_tokens_above":200,"input":4,"output":8,"cache_read":0.4,"cache_write":5},
                 {"input_tokens_above":100,"input":2,"output":4,"cache_read":0.2,"cache_write":2.5}]})).expect("schedule")
}

#[test]
fn highest_strict_threshold_prices_the_complete_prompt_including_cache() {
    let cost = schedule();
    for (prompt, input, output) in [
        (0, 1.0, 2.0),
        (100, 1.0, 2.0),
        (101, 2.0, 4.0),
        (200, 2.0, 4.0),
        (201, 4.0, 8.0),
    ] {
        let rates = cost.at(prompt);
        assert_eq!((rates.input, rates.output), (input, output));
    }
    for usage in [
        Usage {
            input: 101,
            output: 10,
            ..Default::default()
        },
        Usage {
            cache_read: 101,
            output: 10,
            ..Default::default()
        },
        Usage {
            cache_write: 101,
            output: 10,
            ..Default::default()
        },
        Usage {
            input: 1,
            cache_read: 50,
            cache_write: 50,
            output: 10,
            ..Default::default()
        },
    ] {
        let expected = (usage.input as f64 * 2.0
            + usage.cache_read as f64 * 0.2
            + usage.cache_write as f64 * 2.5
            + 40.0)
            / 1_000_000.0;
        assert!((usage.price(&cost) - expected).abs() < 1e-12);
    }
    let overflow = Usage {
        input: u64::MAX,
        cache_read: 1,
        ..Default::default()
    };
    assert_eq!(cost.at(overflow.prompt_tokens()).input, 4.0);
}

#[test]
fn partial_tiers_inherit_base_rates_and_preserve_explicit_free_cache_prices() {
    let cost: Cost = serde_json::from_value(json!({"input":3,"output":5,
        "tiers":[{"input_tokens_above":100,"input":6,"cache_read":0},
                 {"input_tokens_above":200,"output":7}]}))
    .expect("schedule");
    let first = cost.at(101);
    assert_eq!(
        (
            first.input,
            first.output,
            first.cache_read_rate(),
            first.cache_write_rate()
        ),
        (6.0, 5.0, 0.0, 6.0)
    );
    let second = cost.at(201);
    assert_eq!(
        (
            second.input,
            second.output,
            second.cache_read_rate(),
            second.cache_write_rate()
        ),
        (3.0, 7.0, 3.0, 3.0)
    );
    let reloaded: Cost =
        serde_json::from_value(serde_json::to_value(&cost).expect("JSON")).expect("reload");
    assert_eq!(reloaded, cost);
    assert!(reloaded.at(101).known(Usage {
        cache_read: 1,
        ..Default::default()
    }));
}

#[test]
fn declared_free_base_rates_are_known_but_generated_and_legacy_zeros_are_not() {
    let usage = Usage {
        input: 1,
        output: 1,
        ..Default::default()
    };
    for (raw, known) in [
        (json!({}), false),
        (json!({"input":0,"output":0}), true),
        (
            json!({"input":0,"output":0,"presence":{"version":1}}),
            false,
        ),
        (
            json!({"input":0,"output":0,"presence":{"version":2,"input":true,"output":true}}),
            false,
        ),
    ] {
        let cost: Cost = serde_json::from_value(raw).expect("prices");
        assert_eq!(cost.at(usage.prompt_tokens()).known(usage), known);
    }
    let generated = serde_json::to_value(Cost::default()).expect("generated");
    assert!(
        !serde_json::from_value::<Cost>(generated)
            .expect("reload")
            .at(2)
            .known(usage)
    );
}

#[test]
fn malformed_rates_and_unbounded_or_duplicate_tiers_are_refused() {
    for raw in [
        json!({"input":-1}),
        json!({"cache_read":-1}),
        json!({"tiers":[{"input_tokens_above":10,"output":-1}]}),
        json!({"tiers":[{"input_tokens_above":10},{"input_tokens_above":10}]}),
        json!({"tiers":[{"input_tokens_above":-1}]}),
        json!({"tiers":[{"input_tokens_above":0.5}]}),
        json!({"tiers":[{"input_tokens_above":10,"typo":1}]}),
        json!({"tiers":(0..65).map(|threshold|json!({"input_tokens_above":threshold})).collect::<Vec<_>>()}),
    ] {
        assert!(
            serde_json::from_value::<Cost>(raw.clone()).is_err(),
            "{raw}"
        );
    }
    for rate in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
        assert!(
            Cost {
                input: rate,
                ..Cost::default()
            }
            .validate()
            .is_err()
        );
    }
}
