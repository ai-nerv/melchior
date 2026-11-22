use super::*;

#[test]
fn split_signatures_arguments_and_initial_text_are_replayed_once_in_original_order() {
    let api = adapter("anthropic-messages");
    let mut model = model();
    model.compat.as_mut().expect("compat").allow_empty_signature = Some(false);
    let mut state = begin(&api, &model);
    let mut deltas = started(&api, &mut state);
    for (name, data) in [
        (
            "content_block_start",
            json!({"index":0,"content_block":{"type":"thinking","thinking":"α","signature":"sig"}}),
        ),
        (
            "content_block_delta",
            json!({"index":0,"delta":{"type":"thinking_delta","thinking":"β"}}),
        ),
        (
            "content_block_delta",
            json!({"index":0,"delta":{"type":"signature_delta","signature":"-one"}}),
        ),
        (
            "content_block_delta",
            json!({"index":0,"delta":{"type":"signature_delta","signature":"-two"}}),
        ),
        ("content_block_stop", json!({"index":0})),
        (
            "content_block_start",
            json!({"index":1,"content_block":{"type":"tool_use","id":"t","name":"echo","input":{}}}),
        ),
        (
            "content_block_delta",
            json!({"index":1,"delta":{"type":"input_json_delta","partial_json":"{\"n\":184467440"}}),
        ),
        (
            "content_block_delta",
            json!({"index":1,"delta":{"type":"input_json_delta","partial_json":"73709551615,\"array\":[null,[],{}]}"}}),
        ),
        ("content_block_stop", json!({"index":1})),
        (
            "content_block_start",
            json!({"index":2,"content_block":{"type":"text","text":"initial "}}),
        ),
        (
            "content_block_delta",
            json!({"index":2,"delta":{"type":"text_delta","text":"tail"}}),
        ),
        ("content_block_stop", json!({"index":2})),
    ] {
        deltas.extend(event(&api, &mut state, name, data));
    }
    deltas.extend(finished(&api, &mut state));
    assert_eq!(deltas.last(), Some(&Delta::Stop(StopReason::ToolUse)));
    assert_eq!(
        deltas
            .iter()
            .filter(|d| matches!(d, Delta::ToolCallArgs(_)))
            .count(),
        1
    );
    assert_eq!(
        deltas
            .iter()
            .filter(|d| matches!(d, Delta::Signature(_)))
            .count(),
        1
    );
    let context = Context {
        messages: vec![neutral(&deltas)],
        ..Default::default()
    };
    let body = api.prepare(&model, &context, &options()).expect("replay").0;
    assert_eq!(
        body["messages"][0]["content"],
        json!([
            {"type":"thinking","thinking":"αβ","signature":"sig-one-two"},
            {"type":"tool_use","id":"t","name":"echo","input":{"n":18446744073709551615u64,"array":[null,[],{}]}},
            {"type":"text","text":"initial tail"}
        ])
    );
    for protocol in [
        "openai-responses",
        "openai-completions",
        "google-generative-ai",
        "pi-messages",
    ] {
        let body = adapter(protocol)
            .prepare(&plain_model(), &context, &options())
            .expect("foreign request")
            .0;
        assert!(
            !body.to_string().contains("melchior.anthropic.v1:"),
            "{protocol}: {body}"
        );
    }
}

#[test]
fn malformed_unfinished_and_oversized_blocks_never_publish_replay_state() {
    let api = adapter("anthropic-messages");
    for case in 0..13 {
        let mut model = model();
        if case == 12 {
            model.compat.as_mut().expect("compat").allow_empty_signature = Some(false);
        }
        let mut state = begin(&api, &model);
        let mut deltas = started(&api, &mut state);
        let text = json!({"type":"text","text":"hi"});
        let events = match case {
            0 => vec![(
                "content_block_start",
                json!({"index":0,"content_block":text}),
            )],
            1 => vec![(
                "content_block_start",
                json!({"index":1,"content_block":text}),
            )],
            2 => vec![(
                "content_block_start",
                json!({"index":-1,"content_block":text}),
            )],
            3 => vec![(
                "content_block_start",
                json!({"index":0.5,"content_block":text}),
            )],
            4 => vec![("content_block_start", json!({"index":0}))],
            5 => vec![(
                "content_block_start",
                json!({"index":0,"content_block":{"type":"unknown","text":"hidden"}}),
            )],
            6 => vec![
                (
                    "content_block_start",
                    json!({"index":0,"content_block":text}),
                ),
                (
                    "content_block_delta",
                    json!({"index":1,"delta":{"type":"text_delta","text":"wrong block"}}),
                ),
            ],
            7 => vec![
                (
                    "content_block_start",
                    json!({"index":0,"content_block":text}),
                ),
                ("content_block_delta", json!({"index":0,"delta":42})),
            ],
            8 => vec![
                (
                    "content_block_start",
                    json!({"index":0,"content_block":{"type":"tool_use","id":"t","name":"echo","input":{}}}),
                ),
                (
                    "content_block_delta",
                    json!({"index":0,"delta":{"type":"input_json_delta","partial_json":"{"}}),
                ),
                ("content_block_stop", json!({"index":0})),
            ],
            9 => vec![(
                "content_block_start",
                json!({"index":0,"content_block":{"type":"thinking","thinking":"x".repeat(1048576)}}),
            )],
            10 => (0..257)
                .flat_map(|index| {
                    [
                        (
                            "content_block_start",
                            json!({"index":index,"content_block":text}),
                        ),
                        ("content_block_stop", json!({"index":index})),
                    ]
                })
                .collect(),
            11 => vec![
                (
                    "content_block_start",
                    json!({"index":0,"content_block":{"type":"tool_use","id":"t","name":"echo","input":{"initial":true}}}),
                ),
                (
                    "content_block_delta",
                    json!({"index":0,"delta":{"type":"input_json_delta","partial_json":"{}"}}),
                ),
                ("content_block_stop", json!({"index":0})),
            ],
            _ => vec![
                (
                    "content_block_start",
                    json!({"index":0,"content_block":{"type":"thinking","thinking":"unsigned"}}),
                ),
                ("content_block_stop", json!({"index":0})),
            ],
        };
        for (name, data) in events {
            deltas.extend(event(&api, &mut state, name, data));
        }
        deltas.extend(finished(&api, &mut state));
        assert_eq!(
            deltas.last(),
            Some(&Delta::Stop(StopReason::Error)),
            "case {case}: {deltas:?}"
        );
        assert!(
            !deltas.iter().any(|d| matches!(d, Delta::Signature(_))),
            "case {case}"
        );
        assert!(state.scratch.to_string().len() < 1024, "case {case}");
        assert_eq!(state.usage.output, 12, "billed failures retain usage");
    }
}

#[test]
fn compatibility_overrides_round_trip_and_explicit_false_disable_provider_defaults() {
    let base = model().compat.expect("compat");
    let local = Compat {
        anthropic_thinking: Some(AnthropicThinking::Budget),
        anthropic_replay: Some(false),
        allow_empty_signature: Some(false),
        supports_forced_tool_choice: Some(true),
        ..Default::default()
    };
    let layered = local.over(Some(base));
    assert_eq!(layered, local);
    assert_eq!(
        serde_json::from_value::<Compat>(serde_json::to_value(layered).expect("encode"))
            .expect("decode"),
        local
    );
    assert_eq!(Compat::default().over(Some(base)), base);
    for invalid in [
        json!({"anthropic_thinking":"automatic"}),
        json!({"anthropic_replay":"yes"}),
        json!({"allow_empty_signature":1}),
        json!({"supports_forced_tool_choice":[]}),
    ] {
        assert!(serde_json::from_value::<Compat>(invalid).is_err());
    }
}
