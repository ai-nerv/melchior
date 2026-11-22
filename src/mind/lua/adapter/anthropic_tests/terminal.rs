use super::*;

#[test]
fn only_successful_known_terminals_publish_complete_replay() {
    let api = adapter("anthropic-messages");
    for (name, delta, expected, replay) in [
        (
            "message_delta",
            json!({"stop_reason":"end_turn"}),
            StopReason::EndTurn,
            true,
        ),
        (
            "message_delta",
            json!({"stop_reason":"stop_sequence"}),
            StopReason::EndTurn,
            true,
        ),
        (
            "message_delta",
            json!({"stop_reason":"tool_use"}),
            StopReason::ToolUse,
            true,
        ),
        (
            "message_delta",
            json!({"stop_reason":"max_tokens"}),
            StopReason::Length,
            false,
        ),
        (
            "message_delta",
            json!({"stop_reason":"future_unknown"}),
            StopReason::Error,
            false,
        ),
        (
            "message_delta",
            json!({"stop_reason":42}),
            StopReason::Error,
            false,
        ),
        ("message_stop", json!({}), StopReason::Error, false),
        ("error", json!({}), StopReason::Error, false),
    ] {
        let mut state = begin(&api, &model());
        started(&api, &mut state);
        event(
            &api,
            &mut state,
            "content_block_start",
            json!({"index":0,
            "content_block":{"type":"tool_use","id":"a","name":"echo","input":{}}}),
        );
        event(&api, &mut state, "content_block_stop", json!({"index":0}));
        let deltas = event(
            &api,
            &mut state,
            name,
            json!({"delta":delta,"usage":{"output_tokens":12}}),
        );
        assert_eq!(
            deltas.last(),
            Some(&Delta::Stop(expected)),
            "{name} {delta}: {deltas:?}"
        );
        assert_eq!(
            deltas.iter().any(|d| matches!(d, Delta::Signature(_))),
            replay,
            "{name} {delta}"
        );
        assert!(event(&api, &mut state, "message_stop", json!({})).is_empty());
        assert!(
            event(
                &api,
                &mut state,
                "message_delta",
                json!({"delta":{"stop_reason":"tool_use"}})
            )
            .is_empty()
        );
        assert!(state.scratch.to_string().len() < 1024);
    }
}

#[test]
fn malformed_signature_delta_poisoning_survives_the_lua_boundary() {
    let api = adapter("anthropic-messages");
    for signature in [json!({}), json!([]), json!(1), json!(null), json!(false)] {
        let mut state = begin(&api, &model());
        started(&api, &mut state);
        event(
            &api,
            &mut state,
            "content_block_start",
            json!({"index":0,
            "content_block":{"type":"thinking","thinking":"reason","signature":signature}}),
        );
        event(
            &api,
            &mut state,
            "content_block_delta",
            json!({"index":0,
            "delta":{"type":"signature_delta","signature":"suffix"}}),
        );
        event(&api, &mut state, "content_block_stop", json!({"index":0}));
        let deltas = finished(&api, &mut state);
        assert_eq!(
            deltas.last(),
            Some(&Delta::Stop(StopReason::Error)),
            "{signature}: {deltas:?}"
        );
        assert!(!deltas.iter().any(|d| matches!(d, Delta::Signature(_))));
    }
}
