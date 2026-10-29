use super::{LuaAdapter, engine_with_builtins, support::adapter};
use crate::mind::model::{MeasuredUsage, Usage};
use crate::mind::provider::{
    api::{Adapter, Delta, StreamState},
    sse,
};
use serde_json::{Value, json};

fn feed(
    adapter: &LuaAdapter,
    state: &mut StreamState,
    name: &str,
    value: Value,
) -> Vec<(Delta, MeasuredUsage)> {
    adapter.on_event_measured(
        state,
        &sse::Event {
            name: name.into(),
            data: value.to_string(),
        },
    )
}

fn usage(pairs: &[(Delta, MeasuredUsage)]) -> (Usage, MeasuredUsage) {
    pairs
        .iter()
        .find_map(|(delta, measured)| match delta {
            Delta::Usage(value) => Some((*value, *measured)),
            _ => None,
        })
        .expect("usage observation")
}

#[test]
fn all_builtin_dialects_distinguish_present_zero_from_missing_fields() {
    for (dialect, event, body) in [
        (
            "openai-completions",
            "",
            json!({"usage":{"prompt_tokens":0,"completion_tokens":0,"prompt_cache_hit_tokens":0}}),
        ),
        (
            "openai-responses",
            "",
            json!({"type":"response.completed","response":{"usage":{"input_tokens":0,"output_tokens":0,"input_tokens_details":{"cached_tokens":0}}}}),
        ),
        (
            "google-generative-ai",
            "",
            json!({"usageMetadata":{"promptTokenCount":0,"candidatesTokenCount":0,"cachedContentTokenCount":0}}),
        ),
        (
            "anthropic-messages",
            "message_start",
            json!({"message":{"usage":{"input_tokens":0,"output_tokens":0,"cache_read_input_tokens":0}}}),
        ),
        (
            "pi-messages",
            "",
            json!({"type":"done","usage":{"input":0,"output":0,"cacheRead":0}}),
        ),
        (
            "decisions",
            "body",
            json!({"answers":{},"usage":{"input_tokens":0,"output_tokens":0}}),
        ),
    ] {
        let adapter = adapter(dialect);
        let mut state = StreamState::default();
        let pairs = feed(&adapter, &mut state, event, body.clone());
        let (value, measured) = usage(&pairs);
        assert_eq!(value, Usage::default(), "{dialect}");
        assert_eq!(measured.input, Some(0), "{dialect}");
        assert_eq!(measured.output, Some(0), "{dialect}");
        assert_eq!(
            measured.cache_read,
            (dialect != "decisions").then_some(0),
            "{dialect}"
        );
        assert_eq!(measured.cache_write, None, "{dialect}");
        assert_eq!(measured.cost_micros, None, "{dialect}");
        let legacy = adapter.on_event(
            &mut StreamState::default(),
            &sse::Event {
                name: event.into(),
                data: body.to_string(),
            },
        );
        assert_eq!(
            pairs
                .into_iter()
                .map(|(delta, _)| delta)
                .collect::<Vec<_>>(),
            legacy
        );
    }
}

#[test]
fn inclusive_input_requires_a_measured_cache_partition() {
    let adapter = adapter("openai-completions");
    let mut state = StreamState::default();
    let (_, missing) = usage(&feed(
        &adapter,
        &mut state,
        "",
        json!({"usage":{"prompt_tokens":100,"completion_tokens":0}}),
    ));
    assert_eq!(
        missing,
        MeasuredUsage {
            output: Some(0),
            ..MeasuredUsage::default()
        }
    );
    let (value, partitioned) = usage(&feed(
        &adapter,
        &mut state,
        "",
        json!({"usage":{"prompt_cache_hit_tokens":30}}),
    ));
    assert_eq!((value.input, value.cache_read), (70, 30));
    assert_eq!(
        partitioned,
        MeasuredUsage {
            input: Some(70),
            output: Some(0),
            cache_read: Some(30),
            ..MeasuredUsage::default()
        }
    );
    let (_, raised) = usage(&feed(
        &adapter,
        &mut state,
        "",
        json!({"usage":{"prompt_tokens":150}}),
    ));
    assert_eq!(raised.input, Some(120));
    assert_eq!(raised.cache_read, Some(30));
}

#[test]
fn invalid_and_partial_updates_do_not_fabricate_or_erase_measurements() {
    let adapter = adapter("openai-completions");
    let mut state = StreamState::default();
    let (_, missing) = usage(&feed(
        &adapter,
        &mut state,
        "",
        json!({"usage":{"prompt_tokens":100,"completion_tokens":false,"cost":"bad","prompt_cache_hit_tokens":0.5}}),
    ));
    assert_eq!(missing, MeasuredUsage::default());
    let (_, known) = usage(&feed(
        &adapter,
        &mut state,
        "",
        json!({"usage":{"prompt_tokens":100.0,"completion_tokens":7.0,"prompt_cache_hit_tokens":30.0,"cost":0.25}}),
    ));
    assert_eq!(
        known,
        MeasuredUsage {
            input: Some(70),
            output: Some(7),
            cache_read: Some(30),
            cost_micros: Some(250000),
            ..MeasuredUsage::default()
        }
    );
    let (_, retained) = usage(&feed(
        &adapter,
        &mut state,
        "",
        json!({"usage":{"prompt_tokens":false,"completion_tokens":-2,"prompt_cache_hit_tokens":"bad","cost":1e100}}),
    ));
    assert_eq!(retained, known);
    let (_, lower) = usage(&feed(
        &adapter,
        &mut state,
        "",
        json!({"usage":{"completion_tokens":0,"cost":0}}),
    ));
    assert_eq!(lower, known);
    assert!(state.usage_anomalies > 0);
}

#[test]
fn conflicting_cache_counts_are_not_claimed_as_exact_measurements() {
    let adapter = adapter("openai-completions");
    let mut state = StreamState::default();
    let (held, measured) = usage(&feed(
        &adapter,
        &mut state,
        "",
        json!({"usage":{"prompt_tokens":100,"completion_tokens":7,"prompt_cache_hit_tokens":200}}),
    ));
    assert_eq!((held.input, held.cache_read), (0, 100));
    assert_eq!(measured.input, None);
    assert_eq!(measured.cache_read, None);
    assert_eq!(measured.output, Some(7));
}

#[test]
fn exclusive_cache_write_and_incremental_output_keep_their_own_presence() {
    let adapter = adapter("anthropic-messages");
    let mut state = StreamState::default();
    let (_, first) = usage(&feed(
        &adapter,
        &mut state,
        "message_start",
        json!({"message":{"usage":{"input_tokens":12,"cache_creation_input_tokens":0}}}),
    ));
    assert_eq!(
        first,
        MeasuredUsage {
            input: Some(12),
            cache_write: Some(0),
            ..MeasuredUsage::default()
        }
    );
    let (_, second) = usage(&feed(
        &adapter,
        &mut state,
        "message_delta",
        json!({"usage":{"output_tokens":0}}),
    ));
    assert_eq!(
        second,
        MeasuredUsage {
            output: Some(0),
            ..first
        }
    );
}

#[test]
fn reported_zero_cost_is_not_the_same_as_unreported_cost() {
    for cost in [0.0, 0.25] {
        let (_, measured) = usage(&feed(
            &adapter("openai-completions"),
            &mut StreamState::default(),
            "",
            json!({"usage":{"cost":cost}}),
        ));
        assert_eq!(measured.cost_micros, Some((cost * 1_000_000.0) as u64));
        assert_eq!(measured.input, None);
        assert_eq!(measured.output, None);
    }
}

#[test]
fn custom_protocol_metadata_cannot_override_native_counts_or_invent_legacy_presence() {
    let mut engine = engine_with_builtins().expect("builtins");
    engine.run(r#"melchior.api("evidence-fixture", {
        on_event = function(state, event)
            return { deltas = {
                { kind = "usage", usage = {input = 1}, measured = {input = 1} },
                { kind = "usage", usage = {output = 2} },
                { kind = "usage", usage = {input = 3}, measured = {input = 999, output = "bad", cache_read = false} },
            } }
        end,
    })"#, "owned evidence fixture").expect("fixture");
    let adapter = LuaAdapter::new(engine, "evidence-fixture").expect("adapter");
    let pairs = feed(&adapter, &mut StreamState::default(), "", json!({}));
    assert_eq!(pairs.len(), 3);
    assert_eq!(pairs[0].1.input, Some(1));
    assert_eq!(pairs[1].1.input, Some(1));
    assert_eq!(pairs[1].1.output, None);
    assert_eq!(pairs[2].1, MeasuredUsage::default());
    let Delta::Usage(native) = pairs[2].0 else {
        panic!("usage");
    };
    assert_eq!((native.input, native.output), (3, 2));
}
