use super::*;

fn read(
    api: &str,
    usage: Value,
    ending: &str,
) -> (
    crate::mind::model::Usage,
    crate::mind::model::MeasuredUsage,
    u64,
) {
    let wire = adapter(api);
    let mut state = StreamState::default();
    let event = Event {
        name: String::new(),
        data: json!({"type":ending,"response":{
        "output":[],"usage":usage,"incomplete_details":{"reason":"max_output_tokens"}}})
        .to_string(),
    };
    let values = wire.on_event_measured(&mut state, &event);
    let found = values
        .iter()
        .find_map(|(value, measured)| match value {
            Delta::Usage(usage) => Some((*usage, *measured, state.usage_anomalies)),
            _ => None,
        })
        .expect("usage");
    assert!(wire.on_event_measured(&mut state, &event).is_empty());
    found
}

#[test]
fn ticks_round_to_microdollars_without_losing_large_integer_precision() {
    for api in APIS {
        for ticks in [
            0,
            1,
            4999,
            5000,
            9999,
            10000,
            158500,
            37756000,
            9007199254744999,
            9007199254745000,
            u64::MAX,
        ] {
            let expected = ticks / 10000 + u64::from(ticks % 10000 >= 5000);
            let (usage, measured, anomalies) = read(
                api,
                json!({"cost_in_usd_ticks":ticks,
                "input_tokens":90,"output_tokens":12,"input_tokens_details":{"cached_tokens":30}}),
                "response.completed",
            );
            assert_eq!(usage.cost_micros, expected, "{api} {ticks}");
            assert_eq!(measured.cost_micros, Some(expected), "{api} {ticks}");
            assert_eq!((usage.input, usage.cache_read, usage.output), (60, 30, 12));
            assert_eq!(anomalies, 0);
        }
    }
}

#[test]
fn malformed_ticks_are_not_reported_as_zero_or_replaced_by_an_unrelated_dollar_field() {
    for value in [
        Value::Null,
        json!(-1),
        json!(0.5),
        json!(10000.0),
        json!(true),
        json!(false),
        json!("10000"),
        json!([]),
        json!({}),
        serde_json::from_str("18446744073709551616").expect("number"),
    ] {
        let (usage, measured, anomalies) = read(
            APIS[0],
            json!({"cost_in_usd_ticks":value,"cost":1,
            "input_tokens":2,"output_tokens":3}),
            "response.completed",
        );
        assert_eq!(usage.cost_micros, 0, "{value}");
        assert_eq!(measured.cost_micros, None, "{value}");
        assert_eq!(anomalies, 1, "{value}");
        assert_eq!((usage.input, usage.output), (2, 3));
    }
}

#[test]
fn ticks_have_explicit_precedence_and_missing_ticks_keep_existing_dollar_usage() {
    for (value, expected) in [
        (json!({}), None),
        (json!({"cost":0}), Some(0)),
        (json!({"cost":0.25}), Some(250000)),
        (json!({"cost_in_usd_ticks":0,"cost":99}), Some(0)),
        (json!({"cost_in_usd_ticks":158500,"cost":99}), Some(16)),
    ] {
        let (_, measured, anomalies) = read(APIS[0], value, "response.completed");
        assert_eq!(measured.cost_micros, expected);
        assert_eq!(anomalies, 0);
    }
}

#[test]
fn failed_and_incomplete_responses_retain_the_reported_charge_once() {
    for ending in ["response.failed", "response.incomplete"] {
        let (usage, measured, anomalies) =
            read(APIS[0], json!({"cost_in_usd_ticks":2500000000_u64}), ending);
        assert_eq!(usage.cost_micros, 250000);
        assert_eq!(measured.cost_micros, Some(250000));
        assert_eq!(anomalies, 0);
    }
}
