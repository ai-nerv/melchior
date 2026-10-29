use super::*;

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Legacy {
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
    cost_micros: u64,
}

#[test]
fn optional_estimates_preserve_both_json_and_cbor_reader_directions() {
    let old = Legacy {
        input: 10,
        output: 7,
        cache_read: 90,
        cache_write: 0,
        cost_micros: 4,
    };
    let legacy_json = serde_json::to_value(&old).expect("legacy json");
    let mut new: Usage = serde_json::from_value(legacy_json.clone()).expect("new reader");
    assert_eq!(new.estimated_micros, None);
    assert_eq!(
        serde_json::to_value(new).expect("unchanged legacy shape"),
        legacy_json
    );
    new.estimated_micros = Some(3);
    assert_eq!(new.charged_micros(), 7);
    assert_eq!(
        serde_json::from_value::<Legacy>(serde_json::to_value(new).expect("new json"))
            .expect("old reader"),
        old
    );
    let mut encoded = Vec::new();
    ciborium::ser::into_writer(&old, &mut encoded).expect("old cbor");
    let restored: Usage = ciborium::de::from_reader(encoded.as_slice()).expect("new cbor reader");
    assert_eq!(restored.estimated_micros, None);
    encoded.clear();
    ciborium::ser::into_writer(&new, &mut encoded).expect("new cbor");
    assert_eq!(
        ciborium::de::from_reader::<Legacy, _>(encoded.as_slice()).expect("old cbor reader"),
        old
    );
    assert_eq!(
        ciborium::de::from_reader::<Usage, _>(encoded.as_slice()).expect("new cbor reader"),
        new
    );
}

#[test]
fn estimate_totals_saturate_and_do_not_reprice_legacy_usage() {
    let mut total = Usage {
        cost_micros: 4,
        ..Default::default()
    };
    total.add(Usage {
        cost_micros: 7,
        estimated_micros: Some(3),
        ..Default::default()
    });
    assert_eq!(total.cost_micros, 11);
    assert_eq!(total.estimated_micros, Some(3));
    assert_eq!(total.charged_micros(), 14);
    total.add(Usage {
        cost_micros: u64::MAX,
        estimated_micros: Some(u64::MAX),
        ..Default::default()
    });
    assert_eq!(total.charged_micros(), u64::MAX);
    assert_eq!(total.estimated_micros, Some(u64::MAX));
}

#[test]
fn optional_estimates_are_strict_nonnegative_integral_counters() {
    for value in [
        serde_json::json!(-1),
        serde_json::json!(0.5),
        serde_json::json!(1e100),
        serde_json::json!("3"),
    ] {
        let mut raw = serde_json::to_value(Usage::default()).expect("usage");
        raw["estimated_micros"] = value;
        assert!(serde_json::from_value::<Usage>(raw).is_err());
    }
}
