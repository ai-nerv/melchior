use super::*;

fn call(id: &str, arguments: &str) -> Value {
    json!({"type":"function_call","id":format!("fc_{id}"),"call_id":id,
        "name":format!("tool_{id}"),"arguments":arguments})
}

fn item(kind: &str, index: usize, value: Value) -> Value {
    json!({"type":format!("response.output_item.{kind}"),"output_index":index,"item":value})
}

fn args(index: usize, id: &str, delta: &str) -> Value {
    json!({"type":"response.function_call_arguments.delta","output_index":index,
        "item_id":format!("fc_{id}"),"delta":delta})
}

fn terminal(kind: &str, output: Option<Value>) -> Value {
    let mut value = json!({"type":format!("response.{kind}"),"response":{
        "usage":{"input_tokens":100,"output_tokens":20,"input_tokens_details":{"cached_tokens":40}},
        "incomplete_details":{"reason":"max_output_tokens"}}});
    if let Some(output) = output {
        value["response"]["output"] = output;
    }
    value
}

fn run(api: &str, events: &[Value]) -> Vec<Delta> {
    let wire = adapter(api);
    let request = wire.request(&model(api), &plain_context(), &bound_options());
    let mut state = StreamState {
        scratch: request["__scratch"].clone(),
        ..Default::default()
    };
    events
        .iter()
        .flat_map(|value| {
            wire.on_event(
                &mut state,
                &Event {
                    name: String::new(),
                    data: value.to_string(),
                },
            )
        })
        .collect()
}

fn calls(deltas: &[Delta]) -> Vec<(String, String, String)> {
    let mut calls = Vec::new();
    for delta in deltas {
        match delta {
            Delta::ToolCallStart { id, name } => {
                calls.push((id.clone(), name.clone(), String::new()))
            }
            Delta::ToolCallArgs(args) => calls.last_mut().expect("started call").2.push_str(args),
            _ => {}
        }
    }
    calls
}

fn text(deltas: &[Delta]) -> String {
    deltas
        .iter()
        .filter_map(|delta| match delta {
            Delta::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn usage(deltas: &[Delta]) {
    let usage = deltas
        .iter()
        .find_map(|delta| match delta {
            Delta::Usage(usage) => Some(usage),
            _ => None,
        })
        .expect("usage");
    assert_eq!((usage.input, usage.cache_read, usage.output), (60, 40, 20));
}

#[test]
fn interleaved_arguments_keep_source_order_and_stream_the_active_call() {
    for api in APIS {
        let mut events = vec![
            item("added", 0, call("a", "")),
            args(0, "a", "{\"a\":"),
            item("added", 1, call("b", "")),
            args(1, "b", "[2]"),
            item("done", 1, call("b", "[2]")),
        ];
        assert_eq!(
            calls(&run(api, &events)),
            vec![("a".into(), "tool_a".into(), "{\"a\":".into())]
        );
        events.extend([
            args(0, "a", "1}"),
            item("done", 0, call("a", "{\"a\":1}")),
            terminal(
                "completed",
                Some(json!([call("a", "{\"a\":1}"), call("b", "[2]")])),
            ),
        ]);
        let deltas = run(api, &events);
        assert_eq!(
            calls(&deltas),
            vec![
                ("a".into(), "tool_a".into(), "{\"a\":1}".into()),
                ("b".into(), "tool_b".into(), "[2]".into())
            ]
        );
        assert_eq!(deltas.last(), Some(&Delta::Stop(StopReason::ToolUse)));
        usage(&deltas);
    }
}

#[test]
fn terminal_only_output_reconstructs_visible_content_and_multiple_calls() {
    for api in APIS {
        let deltas = run(api, &[terminal("completed", Some(items()))]);
        assert_eq!(text(&deltas), "Checking");
        assert_eq!(
            calls(&deltas),
            vec![(
                "call_a".into(),
                "lookup".into(),
                "{\"data\":[null,[],{},18446744073709551615]}".into()
            )]
        );
        assert!(
            deltas
                .iter()
                .any(|d| matches!(d,Delta::Thinking(value) if value=="private"))
        );
        assert!(deltas.iter().any(|d| matches!(d, Delta::Signature(_))));
        usage(&deltas);
    }
}

#[test]
fn done_items_supply_missing_terminal_output_and_preserve_exact_state() {
    for api in APIS {
        let output = items();
        let mut events: Vec<_> = output
            .as_array()
            .expect("items")
            .iter()
            .enumerate()
            .map(|(index, value)| item("done", index, value.clone()))
            .collect();
        events.push(terminal("completed", None));
        let deltas = run(api, &events);
        assert_eq!(text(&deltas), "Checking");
        assert_eq!(calls(&deltas).len(), 1);
        let signature = deltas
            .iter()
            .find_map(|d| match d {
                Delta::Signature(s) => Some(s),
                _ => None,
            })
            .expect("signature");
        let replay =
            adapter(api).request(&model(api), &context(signature.clone()), &bound_options());
        assert_eq!(replay["input"], output);
        assert_eq!(deltas.last(), Some(&Delta::Stop(StopReason::ToolUse)));
    }
}

#[test]
fn text_parts_are_ordered_and_done_events_only_supply_missing_suffixes() {
    let message = json!({"type":"message","id":"msg","role":"assistant","content":[
        {"type":"output_text","text":"Hello "},{"type":"output_text","text":"world"}]});
    for api in APIS {
        let deltas = run(
            api,
            &[
                item(
                    "added",
                    0,
                    json!({"type":"message","id":"msg","role":"assistant","content":[]}),
                ),
                json!({"type":"response.output_text.delta","output_index":0,"item_id":"msg","content_index":1,"delta":"world"}),
                json!({"type":"response.output_text.delta","output_index":0,"item_id":"msg","content_index":0,"delta":"Hel"}),
                json!({"type":"response.output_text.done","output_index":0,"item_id":"msg","content_index":0,"text":"Hello "}),
                item("done", 0, message.clone()),
                terminal("completed", Some(json!([message]))),
            ],
        );
        assert_eq!(text(&deltas), "Hello world");
        assert_eq!(deltas.last(), Some(&Delta::Stop(StopReason::EndTurn)));
    }
}

#[test]
fn conflicting_identity_arguments_and_orphaned_deltas_fail_with_usage_intact() {
    let original = call("a", "{}");
    for invalid in [
        item("done", 0, call("b", "{}")),
        item("done", 0, call("a", "[]")),
        args(0, "b", "x"),
        args(1, "a", "x"),
        item("added", 1, call("a", "")),
    ] {
        let deltas = run(
            APIS[0],
            &[
                item("added", 0, call("a", "")),
                args(0, "a", "{}"),
                invalid,
                terminal("completed", Some(json!([original]))),
            ],
        );
        assert_eq!(
            deltas.last(),
            Some(&Delta::Stop(StopReason::Error)),
            "{deltas:?}"
        );
        assert!(!deltas.iter().any(|d| matches!(d, Delta::Signature(_))));
        usage(&deltas);
    }
}

#[test]
fn incomplete_stream_keeps_every_partial_call_without_creating_a_continuation() {
    for api in APIS {
        let deltas = run(
            api,
            &[
                item("added", 0, call("a", "")),
                args(0, "a", "{\"a\":"),
                item("added", 1, call("b", "")),
                args(1, "b", "["),
                terminal("incomplete", None),
            ],
        );
        assert_eq!(
            calls(&deltas),
            vec![
                ("a".into(), "tool_a".into(), "{\"a\":".into()),
                ("b".into(), "tool_b".into(), "[".into())
            ]
        );
        assert_eq!(deltas.last(), Some(&Delta::Stop(StopReason::Length)));
        assert!(!deltas.iter().any(|d| matches!(d, Delta::Signature(_))));
        usage(&deltas);
    }
}

#[test]
fn pending_payload_and_indices_have_finite_limits_and_keep_usage() {
    for invalid in [
        args(0, "a", &"x".repeat(1048577)),
        item("added", 256, call("b", "")),
        json!({"type":"response.output_item.added","output_index":-1,"item":call("b","")}),
    ] {
        let deltas = run(
            APIS[0],
            &[
                item("added", 0, call("a", "")),
                invalid,
                terminal("completed", None),
            ],
        );
        assert_eq!(deltas.last(), Some(&Delta::Stop(StopReason::Error)));
        assert!(!deltas.iter().any(|d| matches!(d, Delta::Signature(_))));
        usage(&deltas);
    }
}

#[test]
fn late_parts_and_malformed_terminal_shapes_cannot_finish_cleanly() {
    let message = json!({"type":"message","id":"msg","role":"assistant","content":[
        {"type":"output_text","text":"ready"}]});
    for invalid in [
        json!({"type":"response.output_text.delta","output_index":0,"item_id":"msg","content_index":1,"delta":"late"}),
        terminal("completed", Some(json!({}))),
        json!({"type":"response.completed","response":null}),
    ] {
        let deltas = run(
            APIS[0],
            &[
                item("done", 0, message.clone()),
                invalid,
                terminal("completed", Some(json!([message]))),
            ],
        );
        assert_eq!(deltas.last(), Some(&Delta::Stop(StopReason::Error)));
        assert!(
            !deltas
                .iter()
                .any(|delta| matches!(delta, Delta::Signature(_)))
        );
    }
}

#[test]
fn reasoning_channels_and_refusal_parts_are_preserved_without_repeating_done_text() {
    let reasoning = json!({"type":"reasoning","id":"rs","encrypted_content":"opaque","summary":[
        {"type":"summary_text","text":"summary"}],"content":[{"type":"reasoning_text","text":"raw"}]});
    let refusal = json!({"type":"message","role":"assistant","id":"msg","content":[
        {"type":"refusal","refusal":"No thanks"}]});
    let deltas = run(
        APIS[0],
        &[
            item(
                "added",
                0,
                json!({"type":"reasoning","id":"rs","summary":[]}),
            ),
            json!({"type":"response.reasoning_text.delta","output_index":0,"item_id":"rs","content_index":0,"delta":"raw"}),
            json!({"type":"response.reasoning_summary_text.delta","output_index":0,"item_id":"rs","summary_index":0,"delta":"sum"}),
            item("done", 0, reasoning.clone()),
            item(
                "added",
                1,
                json!({"type":"message","id":"msg","role":"assistant","content":[]}),
            ),
            json!({"type":"response.refusal.delta","output_index":1,"item_id":"msg","content_index":0,"delta":"No"}),
            json!({"type":"response.refusal.done","output_index":1,"item_id":"msg","content_index":0,"refusal":"No thanks"}),
            item("done", 1, refusal.clone()),
            terminal("completed", Some(json!([reasoning, refusal]))),
        ],
    );
    let thinking: String = deltas
        .iter()
        .filter_map(|d| match d {
            Delta::Thinking(value) => Some(value.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(thinking, "rawsummary");
    assert_eq!(text(&deltas), "No thanks");
    assert_eq!(deltas.last(), Some(&Delta::Stop(StopReason::EndTurn)));
}

#[test]
fn an_unrecognized_incomplete_reason_is_not_a_successful_turn() {
    for why in ["content_filter", "unknown"] {
        let mut ended = terminal("incomplete", None);
        ended["response"]["incomplete_details"]["reason"] = why.into();
        let deltas = run(APIS[0], &[ended]);
        assert_eq!(deltas.last(), Some(&Delta::Stop(StopReason::Error)));
        usage(&deltas);
    }
}

#[test]
fn unfinished_or_namespaced_calls_cannot_be_misrepresented_as_plain_completed_tools() {
    for (key, value) in [("status", "incomplete"), ("namespace", "other_tools")] {
        let mut output = call("a", "{}");
        output[key] = value.into();
        let deltas = run(APIS[0], &[terminal("completed", Some(json!([output])))]);
        assert_eq!(deltas.last(), Some(&Delta::Stop(StopReason::Error)));
        assert!(!deltas.iter().any(|d| matches!(d, Delta::Signature(_))));
        usage(&deltas);
    }
}
