use super::{LuaAdapter, support::adapter};
use crate::mind::model::{MeasuredUsage, Usage};
use crate::mind::provider::{
    api::{Adapter, Delta, StreamState},
    sse,
};
use serde_json::{Value, json};

const APIS: [&str; 2] = ["google-generative-ai", "google-vertex"];

fn feed(adapter: &LuaAdapter, state: &mut StreamState, raw: Value) -> Vec<Delta> {
    adapter.on_event(
        state,
        &sse::Event {
            name: String::new(),
            data: raw.to_string(),
        },
    )
}

fn usage(adapter: &LuaAdapter, state: &mut StreamState, raw: Value) -> (Usage, MeasuredUsage) {
    let deltas = feed(adapter, state, json!({"usageMetadata":raw}));
    assert!(deltas.iter().any(|delta| matches!(delta, Delta::Usage(_))));
    (state.usage, state.measured_usage)
}

#[test]
fn candidate_and_thought_snapshots_are_added_once_with_late_partial_updates() {
    for api in APIS {
        let adapter = adapter(api);
        let mut state = StreamState::default();
        for (raw, expected) in [
            (
                json!({"promptTokenCount":100,"cachedContentTokenCount":40,"candidatesTokenCount":7,"thoughtsTokenCount":5}),
                12,
            ),
            (json!({"candidatesTokenCount":7,"thoughtsTokenCount":5}), 12),
            (json!({"thoughtsTokenCount":9}), 16),
            (json!({"candidatesTokenCount":10}), 19),
            (json!({}), 19),
        ] {
            let (value, measured) = usage(&adapter, &mut state, raw);
            assert_eq!(
                (value.input, value.cache_read, value.output),
                (60, 40, expected),
                "{api}"
            );
            assert_eq!(measured.output, Some(expected), "{api}");
            assert_eq!((measured.input, measured.cache_read), (Some(60), Some(40)));
        }
        assert_eq!(state.usage_anomalies, 0);
    }
}

#[test]
fn stale_components_cannot_cancel_growth_in_the_other_component() {
    for api in APIS {
        let adapter = adapter(api);
        let mut state = StreamState::default();
        for (raw, expected) in [
            (
                json!({"candidatesTokenCount":7.0,"thoughtsTokenCount":40.0}),
                47,
            ),
            (
                json!({"candidatesTokenCount":90,"thoughtsTokenCount":3}),
                130,
            ),
            (
                json!({"candidatesTokenCount":50,"thoughtsTokenCount":30}),
                130,
            ),
            (
                json!({"candidatesTokenCount":0,"thoughtsTokenCount":0}),
                130,
            ),
        ] {
            let (value, measured) = usage(&adapter, &mut state, raw);
            assert_eq!(value.output, expected, "{api}");
            assert_eq!(measured.output, Some(expected));
        }
        assert!(state.usage_anomalies >= 3);
    }
}

#[test]
fn absent_zero_and_invalid_unknown_components_keep_distinct_evidence() {
    for api in APIS {
        let adapter = adapter(api);
        let mut state = StreamState::default();
        assert_eq!(usage(&adapter, &mut state, json!({})).1.output, None);
        assert_eq!(
            usage(&adapter, &mut state, json!({"thoughtsTokenCount":5}))
                .0
                .output,
            5
        );
        assert_eq!(state.measured_usage.output, None);
        assert_eq!(
            usage(&adapter, &mut state, json!({"candidatesTokenCount":0}))
                .1
                .output,
            Some(5)
        );

        let mut state = StreamState::default();
        let (value, measured) = usage(
            &adapter,
            &mut state,
            json!({"candidatesTokenCount":7,"thoughtsTokenCount":"bad"}),
        );
        assert_eq!(value.output, 7);
        assert_eq!(measured.output, None);
        assert_eq!(
            usage(&adapter, &mut state, json!({"candidatesTokenCount":8}))
                .1
                .output,
            None
        );
        assert_eq!(
            usage(&adapter, &mut state, json!({"thoughtsTokenCount":0}))
                .1
                .output,
            Some(8)
        );

        let mut state = StreamState::default();
        assert_eq!(
            usage(&adapter, &mut state, json!({"candidatesTokenCount":0}))
                .1
                .output,
            Some(0)
        );
        assert_eq!(
            usage(
                &adapter,
                &mut state,
                json!({"candidatesTokenCount":0,"thoughtsTokenCount":0})
            )
            .1
            .output,
            Some(0)
        );
    }
}

#[test]
fn malformed_components_preserve_paid_usage_and_independent_growth() {
    for api in APIS {
        let adapter = adapter(api);
        for invalid in [
            json!(null),
            json!(false),
            json!("7"),
            json!(-1),
            json!(0.5),
            json!(2147483648_u64),
            json!(u64::MAX),
            json!(1e100),
            json!([]),
            json!({}),
        ] {
            let mut state = StreamState::default();
            usage(
                &adapter,
                &mut state,
                json!({"candidatesTokenCount":8,"thoughtsTokenCount":5}),
            );
            let (value, measured) = usage(
                &adapter,
                &mut state,
                json!({"candidatesTokenCount":invalid,"thoughtsTokenCount":7}),
            );
            assert_eq!(value.output, 15, "{api} {invalid}");
            assert_eq!(measured.output, Some(15));
            let (value, measured) = usage(
                &adapter,
                &mut state,
                json!({"candidatesTokenCount":10,"thoughtsTokenCount":invalid}),
            );
            assert_eq!(value.output, 17, "{api} {invalid}");
            assert_eq!(measured.output, Some(17));
            assert!(state.usage_anomalies >= 2, "{api} {invalid}");
        }
    }
}

#[test]
fn wire_integer_bounds_are_exact_without_overflow_or_false_measurements() {
    for api in APIS {
        let adapter = adapter(api);
        let mut state = StreamState::default();
        let (value, measured) = usage(
            &adapter,
            &mut state,
            json!({"candidatesTokenCount":i32::MAX,"thoughtsTokenCount":i32::MAX}),
        );
        assert_eq!(value.output, 4_294_967_294);
        assert_eq!(measured.output, Some(4_294_967_294));
        let mut state = StreamState::default();
        let (value, measured) = usage(
            &adapter,
            &mut state,
            json!({"candidatesTokenCount":u64::MAX,"thoughtsTokenCount":u64::MAX}),
        );
        assert_eq!(value.output, 0);
        assert_eq!(measured.output, None);
        assert!(state.usage_anomalies >= 2);
        assert_eq!(
            usage(
                &adapter,
                &mut state,
                json!({"candidatesTokenCount":1,"thoughtsTokenCount":2})
            )
            .1
            .output,
            Some(3)
        );
    }
}

#[test]
fn late_accounting_after_any_terminal_preserves_thoughts_without_repeating_stop() {
    for api in APIS {
        let adapter = adapter(api);
        for finish in ["STOP", "MAX_TOKENS", "SAFETY"] {
            let mut state = StreamState::default();
            let deltas = feed(
                &adapter,
                &mut state,
                json!({
                    "candidates":[{"content":{"parts":[{"text":"answer"}]},"finishReason":finish}],
                    "usageMetadata":{"candidatesTokenCount":5,"thoughtsTokenCount":7}
                }),
            );
            assert_eq!(
                deltas
                    .iter()
                    .filter(|d| matches!(d, Delta::Stop(_)))
                    .count(),
                1
            );
            assert_eq!(state.usage.output, 12);
            let deltas = feed(
                &adapter,
                &mut state,
                json!({"usageMetadata":{"candidatesTokenCount":8,"thoughtsTokenCount":11}}),
            );
            assert_eq!(state.usage.output, 19);
            assert_eq!(state.measured_usage.output, Some(19));
            assert!(deltas.iter().all(|delta| matches!(delta, Delta::Usage(_))));
            let mut next = StreamState::default();
            assert_eq!(
                usage(
                    &adapter,
                    &mut next,
                    json!({"candidatesTokenCount":2,"thoughtsTokenCount":1})
                )
                .0
                .output,
                3
            );
        }
    }
}
