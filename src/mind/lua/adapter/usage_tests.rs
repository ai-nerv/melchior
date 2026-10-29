use super::support::*;
use crate::mind::model::Usage;
use crate::mind::provider::api::Delta;
use serde_json::{Value, json};

fn measured(body: Value) -> (Usage, Vec<Delta>) {
    let deltas = stream(&adapter("openai-completions"), &[("", &body.to_string())]);
    let usage = deltas
        .iter()
        .find_map(|delta| match delta {
            Delta::Usage(usage) => Some(*usage),
            _ => None,
        })
        .expect("valid independent usage fields survive");
    (usage, deltas)
}

#[test]
fn nested_and_alternate_cached_counts_are_duplicate_not_additive() {
    for (nested, alternate, expected) in [
        (None, None, 0),
        (Some(90), None, 90),
        (None, Some(90), 90),
        (Some(90), Some(90), 90),
        (Some(80), Some(90), 90),
        (Some(90), Some(80), 90),
        (Some(200), Some(90), 100),
    ] {
        let (usage, _) = measured(json!({"usage":{"prompt_tokens":100,"completion_tokens":7,
            "prompt_tokens_details":{"cached_tokens":nested},"prompt_cache_hit_tokens":alternate}}));
        assert_eq!(
            (usage.input, usage.cache_read, usage.output),
            (100 - expected, expected, 7)
        );
        assert_eq!(usage.prompt_tokens(), 100);
    }
}

#[test]
fn malformed_cache_metadata_cannot_drop_valid_content_or_other_counters() {
    for invalid in [
        json!(null),
        json!("90"),
        json!(false),
        json!(-1),
        json!(0.5),
        json!(1e100),
        json!({}),
        json!([]),
    ] {
        let (usage, deltas) = measured(json!({"usage":{"prompt_tokens":100,"completion_tokens":7,
            "prompt_tokens_details":{"cached_tokens":invalid}},
            "choices":[{"delta":{"content":"paid content"}}]}));
        assert_eq!(
            (usage.input, usage.output, usage.cache_read),
            (100, 7, 0),
            "{invalid}"
        );
        assert!(
            deltas.contains(&Delta::Text("paid content".into())),
            "{invalid}"
        );
        let (usage, _) = measured(json!({"usage":{"prompt_tokens":100,"completion_tokens":7,
            "prompt_tokens_details":{"cached_tokens":invalid},"prompt_cache_hit_tokens":90}}));
        assert_eq!((usage.input, usage.cache_read), (10, 90), "{invalid}");
    }
    for invalid in [json!("bad"), json!(true), json!(-1)] {
        let (usage, deltas) = measured(json!({"usage":{"prompt_tokens":100,"completion_tokens":7,
            "prompt_tokens_details":invalid},"choices":[{"delta":{"content":"paid content"}}]}));
        assert_eq!((usage.input, usage.output), (100, 7));
        assert!(deltas.contains(&Delta::Text("paid content".into())));
    }
}

#[test]
fn a_partial_measurement_does_not_invent_an_inclusive_total() {
    let (usage, _) =
        measured(json!({"usage":{"completion_tokens":7,"prompt_cache_hit_tokens":90}}));
    assert_eq!((usage.input, usage.cache_read, usage.output), (0, 90, 7));
    let (usage, _) = measured(json!({"usage":{"prompt_tokens":100,"prompt_cache_hit_tokens":90}}));
    assert_eq!((usage.input, usage.cache_read, usage.output), (10, 90, 0));
}

#[test]
fn integral_float_tokens_survive_the_actual_lua_conversion() {
    let (usage, _) = measured(
        json!({"usage":{"prompt_tokens":100.0,"completion_tokens":7.0,
        "prompt_tokens_details":{"cached_tokens":90.0}}}),
    );
    assert_eq!((usage.input, usage.cache_read, usage.output), (10, 90, 7));
}

#[test]
fn malformed_later_fields_do_not_erase_previous_paid_measurements() {
    let events = [
        (
            "",
            r#"{"usage":{"prompt_tokens":100,"completion_tokens":7,"prompt_cache_hit_tokens":90,"cost":0.25}}"#,
        ),
        (
            "",
            r#"{"usage":{"completion_tokens":"bad","prompt_tokens_details":{"cached_tokens":false}},"choices":[{"delta":{"content":"still here"}}]}"#,
        ),
    ];
    let deltas = stream(&adapter("openai-completions"), &events);
    let usages: Vec<_> = deltas
        .iter()
        .filter_map(|delta| match delta {
            Delta::Usage(usage) => Some(*usage),
            _ => None,
        })
        .collect();
    assert_eq!(usages.len(), 2);
    assert_eq!(usages[0], usages[1]);
    assert_eq!(usages[1].cost_micros, 250_000);
    assert!(deltas.contains(&Delta::Text("still here".into())));
}

#[test]
fn responses_and_gemini_bound_inclusive_cache_counts_but_anthropic_is_exclusive() {
    for (name, body, expected) in [
        (
            "openai-responses",
            json!({"type":"response.completed", "response":{"usage":{
            "input_tokens":100,"output_tokens":7,"input_tokens_details":{"cached_tokens":200}}}}),
            (0, 100, 7),
        ),
        (
            "google-generative-ai",
            json!({"usageMetadata":{"promptTokenCount":100,
            "candidatesTokenCount":7,"cachedContentTokenCount":200}}),
            (0, 100, 7),
        ),
        (
            "anthropic-messages",
            json!({"message":{"usage":{"input_tokens":100,
            "output_tokens":7,"cache_read_input_tokens":90}}}),
            (100, 90, 7),
        ),
    ] {
        let event = if name == "anthropic-messages" {
            "message_start"
        } else {
            ""
        };
        let deltas = stream(&adapter(name), &[(event, &body.to_string())]);
        let usage = deltas
            .iter()
            .find_map(|delta| match delta {
                Delta::Usage(usage) => Some(*usage),
                _ => None,
            })
            .expect("valid usage");
        assert_eq!(
            (usage.input, usage.cache_read, usage.output),
            expected,
            "{name}"
        );
    }
}

#[test]
fn invalid_measurements_have_an_observable_anomaly_without_losing_content() {
    use crate::mind::provider::{
        api::{Adapter, StreamState},
        sse::Event,
    };
    let mut state = StreamState::default();
    let deltas = adapter("openai-completions").on_event(&mut state, &Event {name:String::new(),
        data:json!({"usage":{"prompt_tokens":100,"prompt_tokens_details":{"cached_tokens":"bad"}},
            "choices":[{"delta":{"content":"paid content"}}]}).to_string()});
    assert!(state.usage_anomalies > 0);
    assert_eq!(state.usage.input, 100);
    assert!(deltas.contains(&Delta::Text("paid content".into())));
}

#[test]
fn late_cache_classification_and_stale_totals_preserve_prompt_occupancy() {
    let events = [
        (
            "",
            r#"{"usage":{"prompt_tokens":100,"completion_tokens":7,"cost":0.25}}"#,
        ),
        ("", r#"{"usage":{"prompt_cache_hit_tokens":90}}"#),
        (
            "",
            r#"{"usage":{"prompt_tokens":50,"prompt_cache_hit_tokens":5,"completion_tokens":3,"cost":0}}"#,
        ),
        (
            "",
            r#"{"usage":{"prompt_tokens":100,"prompt_cache_hit_tokens":70,"completion_tokens":2}}"#,
        ),
    ];
    let deltas = stream(&adapter("openai-completions"), &events);
    let measured: Vec<_> = deltas
        .iter()
        .filter_map(|delta| match delta {
            Delta::Usage(usage) => Some(*usage),
            _ => None,
        })
        .collect();
    assert_eq!(measured.len(), 4);
    assert_eq!(measured[0].prompt_tokens(), 100);
    for usage in &measured[1..] {
        assert_eq!((usage.input, usage.cache_read, usage.output), (10, 90, 7));
        assert_eq!(usage.prompt_tokens(), 100);
        assert_eq!(usage.cost_micros, 250_000);
    }
}
