use super::support::adapter;
use crate::mind::model::StopReason;
use crate::mind::provider::api::{Adapter, Delta, StreamState};
use crate::mind::provider::sse::Event;
use serde_json::{Value, json};

fn chunk(calls: Value) -> Value {
    json!({"choices":[{"index":0,"delta":{"tool_calls":calls},"finish_reason":null}]})
}

fn start(index: usize, id: &str, args: &str) -> Value {
    json!({"index":index,"id":id,"type":"function","function":{"name":"echo","arguments":args}})
}

fn args(index: usize, value: &str) -> Value {
    json!({"index":index,"function":{"arguments":value}})
}

fn terminal(reason: &str) -> Value {
    json!({"choices":[{"index":0,"delta":{},"finish_reason":reason}]})
}

fn run(events: &[Value]) -> Vec<Delta> {
    raw(&events.iter().map(Value::to_string).collect::<Vec<_>>()).0
}

fn raw(events: &[String]) -> (Vec<Delta>, StreamState) {
    let wire = adapter("openai-completions");
    let mut state = StreamState::default();
    let deltas = events
        .iter()
        .flat_map(|value| {
            wire.on_event(
                &mut state,
                &Event {
                    name: String::new(),
                    data: value.clone(),
                },
            )
        })
        .collect();
    (deltas, state)
}

fn calls(events: &[Delta]) -> Vec<(String, String, String)> {
    let mut calls = Vec::new();
    for event in events {
        match event {
            Delta::ToolCallStart { id, name } => {
                calls.push((id.clone(), name.clone(), String::new()))
            }
            Delta::ToolCallArgs(args) => calls.last_mut().expect("active call").2.push_str(args),
            _ => {}
        }
    }
    calls
}

#[test]
fn interleaved_indexes_keep_arguments_attached_to_their_calls_and_stream_the_first_call() {
    let mut script = vec![
        chunk(json!([start(0, "a", "{\"a\":"), start(1, "b", "[2,")])),
        chunk(json!([args(1, "3]"), args(0, "1}")])),
    ];
    assert_eq!(
        calls(&run(&script)),
        vec![("a".into(), "echo".into(), "{\"a\":1}".into())]
    );
    script.push(terminal("tool_calls"));
    let events = run(&script);
    assert_eq!(
        calls(&events),
        vec![
            ("a".into(), "echo".into(), "{\"a\":1}".into()),
            ("b".into(), "echo".into(), "[2,3]".into())
        ]
    );
    assert_eq!(events.last(), Some(&Delta::Stop(StopReason::ToolUse)));
}

#[test]
fn repetition_truncation_never_reports_success_even_when_tool_json_is_balanced() {
    let events = run(&[
        chunk(json!([start(0, "a", "{}"), start(1, "b", "[]")])),
        terminal("repetition_truncation"),
    ]);
    assert_eq!(events.last(), Some(&Delta::Stop(StopReason::Length)));
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Delta::Stop(StopReason::ToolUse | StopReason::EndTurn)))
    );
}

#[test]
fn out_of_order_indexes_and_delayed_identity_fields_have_stable_source_order() {
    let script = [
        chunk(json!([start(1, "b", "[null,[],{}]")])),
        chunk(json!([{"index":0,"function":{"arguments":"18446744073709551615"}}])),
        chunk(json!([{"index":0,"id":"a"}])),
        chunk(json!([{"index":0,"function":{"name":"echo"}}])),
        terminal("tool_calls"),
    ];
    for prefix in 1..4 {
        assert!(calls(&run(&script[..prefix])).is_empty());
    }
    assert_eq!(
        calls(&run(&script[..4])),
        vec![("a".into(), "echo".into(), u64::MAX.to_string())]
    );
    assert_eq!(
        calls(&run(&script)),
        vec![
            ("a".into(), "echo".into(), u64::MAX.to_string()),
            ("b".into(), "echo".into(), "[null,[],{}]".into())
        ]
    );
}

#[test]
fn exact_arguments_and_non_ascii_text_never_pass_through_numeric_or_table_reencoding() {
    for value in [
        "null",
        "true",
        "0",
        "0.125",
        "-9223372036854775808",
        "18446744073709551615",
        "[]",
        "{}",
        "\"\"",
        "{\"city\":\"Zürich 🦀\",\"a\":[null,[],{}]}",
    ] {
        let midpoint = value
            .char_indices()
            .nth(value.chars().count() / 2)
            .map_or(0, |(at, _)| at);
        let events = run(&[
            chunk(json!([start(0, "a", &value[..midpoint])])),
            chunk(json!([args(0, &value[midpoint..])])),
            terminal("tool_calls"),
        ]);
        assert_eq!(
            calls(&events),
            vec![("a".into(), "echo".into(), value.into())]
        );
        assert_eq!(events.last(), Some(&Delta::Stop(StopReason::ToolUse)));
    }
}

#[test]
fn missing_index_supports_unambiguous_single_and_id_addressed_calls_only() {
    let single = [
        chunk(json!([{"function":{"arguments":"{"}}])),
        chunk(json!([{"id":"a","function":{"name":"echo","arguments":"}"}}])),
        terminal("tool_calls"),
    ];
    assert_eq!(
        calls(&run(&single)),
        vec![("a".into(), "echo".into(), "{}".into())]
    );
    let paired = [
        chunk(json!([
            {"id":"a","function":{"name":"echo","arguments":"["}},
            {"id":"b","function":{"name":"echo","arguments":"{"}}
        ])),
        chunk(json!([
            {"id":"b","function":{"arguments":"}"}}, {"id":"a","function":{"arguments":"]"}}
        ])),
        terminal("tool_calls"),
    ];
    assert_eq!(
        calls(&run(&paired)),
        vec![
            ("a".into(), "echo".into(), "[]".into()),
            ("b".into(), "echo".into(), "{}".into())
        ]
    );
    let mut ambiguous = paired.to_vec();
    ambiguous.insert(1, chunk(json!([{"function":{"arguments":"null"}}])));
    assert_eq!(
        run(&ambiguous).last(),
        Some(&Delta::Stop(StopReason::Error))
    );
}

#[test]
fn repeated_identity_is_idempotent_but_conflicting_or_shared_identity_refuses_success() {
    let original = start(0, "a", "");
    let events = run(&[
        chunk(json!([original.clone(), original])),
        chunk(json!([args(0, "{}")])),
        terminal("tool_calls"),
    ]);
    assert_eq!(
        calls(&events),
        vec![("a".into(), "echo".into(), "{}".into())]
    );
    for changed in [
        start(1, "a", "{}"),
        start(0, "b", "{}"),
        json!({"index":0,"function":{"name":"other"}}),
    ] {
        let events = run(&[
            chunk(json!([start(0, "a", "{}")])),
            chunk(json!([changed])),
            terminal("tool_calls"),
        ]);
        assert_eq!(events.last(), Some(&Delta::Stop(StopReason::Error)));
        assert_eq!(calls(&events).len(), 1);
    }
}

#[test]
fn malformed_shapes_indices_and_metadata_never_become_executable_completion() {
    let mut mutations = vec![
        json!(null),
        json!(false),
        json!([]),
        json!({"index":0,"id":"a","type":"custom","custom":{"input":"{}"}}),
        json!({"index":0,"id":"a","function":false}),
        json!({"index":0,"id":1,"function":{"name":"echo","arguments":"{}"}}),
        json!({"index":0,"id":"a","function":{"name":1,"arguments":"{}"}}),
        json!({"index":0,"id":"a","function":{"name":"echo","arguments":{}}}),
    ];
    for index in [
        json!(null),
        json!(-1),
        json!(0.5),
        json!("0"),
        json!(256),
        json!(u64::MAX),
    ] {
        let mut bad = start(0, "a", "{}");
        bad["index"] = index;
        mutations.push(bad);
    }
    for bad in mutations {
        let events = run(&[chunk(json!([bad.clone()])), terminal("tool_calls")]);
        assert_eq!(
            events.last(),
            Some(&Delta::Stop(StopReason::Error)),
            "{bad}: {events:?}"
        );
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, Delta::Stop(StopReason::ToolUse)))
        );
    }
    for bad in [
        json!({"choices":{}}),
        json!({"choices":[false]}),
        json!({"choices":[null]}),
        json!({"choices":[0]}),
        json!({"choices":[""]}),
        json!({"choices":[{},{}]}),
        json!({"choices":[{"index":1,"delta":{"content":"wrong choice"}}]}),
        json!({"choices":[{"delta":[]}]}),
        chunk(json!({})),
        json!({"choices":[{"delta":{"content":42}}]}),
        json!({"choices":[{"delta":{"reasoning_content":false}}]}),
        json!({"choices":[{"delta":{"function_call":{"name":"legacy"}}}]}),
        json!({"choices":[{"delta":{"role":"user"}}]}),
    ] {
        for prefix in [
            String::new(),
            chunk(json!([start(0, "a", "{}")])).to_string(),
        ] {
            let mut stream = Vec::new();
            if !prefix.is_empty() {
                stream.push(prefix);
            }
            stream.extend([bad.to_string(), "[DONE]".into()]);
            let (events, _) = raw(&stream);
            assert_eq!(
                events.last(),
                Some(&Delta::Stop(StopReason::Error)),
                "{bad}"
            );
        }
    }
}

#[test]
fn terminal_validation_rejects_gaps_missing_identity_and_incomplete_json() {
    for call in [
        start(1, "b", "{}"),
        start(0, "", "{}"),
        json!({"index":0,"id":"a","function":{"arguments":"{}"}}),
        start(0, "a", ""),
        start(0, "a", "{"),
        start(0, "a", "{}{}"),
        start(0, "a", "NaN"),
    ] {
        let events = run(&[chunk(json!([call.clone()])), terminal("tool_calls")]);
        assert_eq!(
            events.last(),
            Some(&Delta::Stop(StopReason::Error)),
            "{call}"
        );
    }
    assert_eq!(
        run(&[terminal("tool_calls")]),
        [Delta::Stop(StopReason::Error)]
    );
    assert_eq!(
        run(&[chunk(json!([start(0, "a", "{}")])), terminal("stop")]).last(),
        Some(&Delta::Stop(StopReason::Error))
    );
}

#[test]
fn failure_terminals_preserve_attempted_calls_and_late_usage_but_never_execute_them() {
    for (reason, expected) in [
        ("length", StopReason::Length),
        ("repetition_truncation", StopReason::Length),
        ("content_filter", StopReason::Error),
        ("unknown", StopReason::Error),
        ("error", StopReason::Error),
    ] {
        let (events,state) = raw(&[
            chunk(json!([start(0,"a","{}"),start(1,"b","{\"incomplete\":")])).to_string(),
            terminal(reason).to_string(),
            json!({"choices":[],"usage":{"prompt_tokens":90,"completion_tokens":12,"prompt_tokens_details":{"cached_tokens":30}}}).to_string(),
            "[DONE]".into(),"[DONE]".into(),terminal("tool_calls").to_string(),
        ]);
        assert_eq!(calls(&events).len(), 2);
        assert_eq!(
            events
                .iter()
                .filter_map(|e| if let Delta::Stop(reason) = e {
                    Some(*reason)
                } else {
                    None
                })
                .collect::<Vec<_>>(),
            [expected]
        );
        assert_eq!(
            (
                state.usage.input,
                state.usage.cache_read,
                state.usage.output
            ),
            (60, 30, 12)
        );
    }
}

#[test]
fn sentinels_complete_valid_legacy_streams_once_and_malformed_or_unfinished_streams_never_succeed()
{
    let (events, _) = raw(&[
        chunk(json!([start(0, "a", "{}")])).to_string(),
        "[DONE]".into(),
        "[DONE]".into(),
    ]);
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Delta::Stop(_)))
            .count(),
        1
    );
    assert_eq!(events.last(), Some(&Delta::Stop(StopReason::ToolUse)));
    let script = [chunk(json!([start(0, "a", "{}")])).to_string()];
    assert!(!raw(&script).0.iter().any(|e| matches!(e, Delta::Stop(_))));
    for bad in [
        "not JSON",
        "[]",
        "42",
        "null",
        "{\"error\":{\"message\":\"failed\"}}",
    ] {
        let (events, _) = raw(&[script[0].clone(), bad.into(), "[DONE]".into()]);
        assert_eq!(
            events.last(),
            Some(&Delta::Stop(StopReason::Error)),
            "{bad}"
        );
    }
}

#[test]
fn terminal_freezes_content_but_preserves_late_measured_zero_usage() {
    let (events, state) = raw(&[
        terminal("stop").to_string(),
        json!({"choices":[{"delta":{"content":"late","reasoning_content":"late"}}]}).to_string(),
        chunk(json!([start(0, "late", "{}")])).to_string(),
        json!({"choices":[],"usage":{"prompt_tokens":0,"completion_tokens":0,"cost":0}})
            .to_string(),
        terminal("stop").to_string(),
        "[DONE]".into(),
    ]);
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Delta::Stop(_)))
            .count(),
        1
    );
    assert!(calls(&events).is_empty());
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Delta::Text(_) | Delta::Thinking(_)))
    );
    assert_eq!(state.measured_usage.cost_micros, Some(0));
}

#[test]
fn call_count_identity_and_aggregate_byte_limits_have_exact_boundaries() {
    for (count, expected) in [(256, StopReason::ToolUse), (257, StopReason::Error)] {
        let values: Vec<_> = (0..count)
            .map(|i| start(i, &format!("id-{i}"), "{}"))
            .collect();
        let events = run(&[chunk(json!(values)), terminal("tool_calls")]);
        assert_eq!(events.last(), Some(&Delta::Stop(expected)));
        if count == 256 {
            assert_eq!(calls(&events).len(), 256);
        }
    }
    for (size, expected) in [(4096, StopReason::ToolUse), (4097, StopReason::Error)] {
        assert_eq!(
            run(&[
                chunk(json!([start(0, &"i".repeat(size), "{}")])),
                terminal("tool_calls")
            ])
            .last(),
            Some(&Delta::Stop(expected))
        );
    }
    for (extra, expected) in [(0, StopReason::ToolUse), (1, StopReason::Error)] {
        let args = format!("\"{}\"", "x".repeat(1024 * 1024 - 7 + extra));
        let (events, state) = raw(&[
            chunk(json!([start(0, "a", &args)])).to_string(),
            terminal("tool_calls").to_string(),
        ]);
        assert_eq!(events.last(), Some(&Delta::Stop(expected)));
        assert!(
            state.scratch.to_string().len() < 1024,
            "closed state is released"
        );
    }
}
