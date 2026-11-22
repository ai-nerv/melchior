use super::support::{adapter, plain_model};
use crate::mind::{
    model::{Context, StopReason, ThinkingLevel},
    provider::{
        api::{Adapter, Delta, Options, StreamState},
        compat::Compat,
        model::{Api, Modality, Model},
        sse::Event,
    },
};
use serde_json::{Value, json};

fn model() -> Model {
    let mut model = plain_model();
    model.api = Api::MistralConversations;
    model.input.push(Modality::Image);
    model
}

fn run(chunks: &[Value]) -> (Vec<Delta>, StreamState) {
    let wire = adapter("mistral-conversations");
    let mut state = StreamState::default();
    let deltas = chunks
        .iter()
        .flat_map(|chunk| {
            wire.on_event(
                &mut state,
                &Event {
                    name: String::new(),
                    data: chunk.to_string(),
                },
            )
        })
        .collect();
    (deltas, state)
}

fn chunk(delta: Value, finish: Value) -> Value {
    json!({"choices":[{"index":0,"delta":delta,"finish_reason":finish}]})
}

#[test]
fn native_request_pairs_remapped_calls_keeps_chunks_and_does_not_mutate_history() {
    let wire = adapter("mistral-conversations");
    let context: Context = serde_json::from_value(json!({"system":"rules","tools":[{
        "name":"echo","description":"Echo","parameters":{"type":"object","examples":[{},[],null]}}],
        "messages":[
            {"role":"user","content":[{"type":"text","text":"inspect"},{"type":"image","media_type":"image/png","data":"AAAA"}]},
            {"role":"assistant","content":[{"type":"thinking","thinking":"why"},{"type":"text","text":""},
                {"type":"thinking","thinking":" more"},
                {"type":"tool_call","id":"x","name":"echo","arguments":{"n":18446744073709551615u64,"values":[{},[],null]}},
                {"type":"tool_call","id":"m00000078","name":"echo","arguments":{}}]},
            {"role":"tool","content":[{"type":"tool_result","id":"x","name":"echo","content":"failed","is_error":true,
                "images":[{"media_type":"image/png","data":"BBBB"}]}]},
            {"role":"tool","content":[{"type":"tool_result","id":"m00000078","name":"echo","content":"","is_error":false}]}
        ]})).expect("context");
    let before = context.clone();
    let body = wire.request(&model(), &context, &Options::default());
    for base in [
        "https://api.mistral.ai",
        "https://api.mistral.ai/",
        "https://api.mistral.ai/v1",
    ] {
        assert_eq!(
            wire.endpoint(base, &model()),
            "https://api.mistral.ai/v1/chat/completions"
        );
    }
    assert!(
        wire.headers(Some("key"))
            .contains(&("authorization".into(), "Bearer key".into()))
    );
    assert_eq!(
        body["messages"][1]["content"][1]["image_url"],
        "data:image/png;base64,AAAA"
    );
    let assistant = &body["messages"][2];
    assert_eq!(
        assistant["content"],
        json!([{"type":"thinking","thinking":[{"type":"text","text":"why more"}]}])
    );
    assert_eq!(assistant["prefix"], false);
    let first = assistant["tool_calls"][0]["id"].as_str().expect("id");
    assert_eq!(first.len(), 9);
    assert_ne!(first, "m00000078");
    assert!(first.bytes().all(|byte| byte.is_ascii_alphanumeric()));
    assert_eq!(body["messages"][3]["tool_call_id"], first);
    assert_eq!(body["messages"][4]["tool_call_id"], "m00000078");
    assert_eq!(
        body["messages"][3]["content"][0]["text"],
        "[tool error] failed"
    );
    assert_eq!(
        body["messages"][3]["content"][1]["image_url"],
        "data:image/png;base64,BBBB"
    );
    assert_eq!(
        body["messages"][4]["content"][0]["text"],
        "(no tool output)"
    );
    let arguments: Value = serde_json::from_str(
        assistant["tool_calls"][0]["function"]["arguments"]
            .as_str()
            .expect("arguments"),
    )
    .expect("JSON");
    assert_eq!(
        arguments,
        json!({"n":18446744073709551615u64,"values":[{},[],null]})
    );
    assert!(body["tools"][0]["function"].get("strict").is_none());
    for field in [
        "store",
        "stream_options",
        "max_completion_tokens",
        "reasoning_content",
        "prompt_mode",
    ] {
        assert!(body.get(field).is_none(), "{field}");
    }
    assert_eq!(context, before);
    assert_eq!(wire.request(&model(), &context, &Options::default()), body);
}

#[test]
fn mapped_effort_clamps_but_default_and_unmapped_prompt_mode_are_distinct() {
    let wire = adapter("mistral-conversations");
    let mut model = model();
    model.thinking = serde_json::from_value(json!({"off":"none","minimal":null,"low":null,"medium":null,"high":"high","xhigh":null,"max":null})).expect("map");
    for (selected, expected) in [
        (None, "none"),
        (Some(ThinkingLevel::Off), "none"),
        (Some(ThinkingLevel::Low), "high"),
        (Some(ThinkingLevel::Max), "high"),
    ] {
        let body = wire.request(
            &model,
            &Context::default(),
            &Options {
                thinking: selected,
                ..Default::default()
            },
        );
        assert_eq!(body["reasoning_effort"], expected);
        assert!(body.get("prompt_mode").is_none());
    }
    let (off, _) = wire
        .prepare(
            &model,
            &Context::default(),
            &Options {
                output_limit: Some(64),
                ..Default::default()
            },
        )
        .expect("off budget");
    assert_eq!(off["reasoning_effort"], "none");
    assert_eq!(off["__budget"]["minimum"], 1);
    assert_eq!(off["max_tokens"], 64);
    model.thinking.insert(ThinkingLevel::Off, None);
    assert!(
        wire.request(&model, &Context::default(), &Options::default())
            .get("reasoning_effort")
            .is_none()
    );
    model.thinking.clear();
    for level in [None, Some(ThinkingLevel::Off), Some(ThinkingLevel::High)] {
        let body = wire.request(
            &model,
            &Context::default(),
            &Options {
                thinking: level,
                ..Default::default()
            },
        );
        assert_eq!(
            body.get("prompt_mode"),
            level
                .filter(|level| *level != ThinkingLevel::Off)
                .map(|_| json!("reasoning"))
                .as_ref()
        );
        assert!(body.get("reasoning_effort").is_none());
    }
    model.reasoning = false;
    assert!(
        wire.request(
            &model,
            &Context::default(),
            &Options {
                thinking: Some(ThinkingLevel::High),
                ..Default::default()
            }
        )
        .get("prompt_mode")
        .is_none()
    );
}

#[test]
fn streamed_thoughts_empty_content_and_interleaved_calls_keep_order_and_exact_arguments() {
    let (events, _) = run(&[
        chunk(
            json!({"content":[{"type":"thinking","thinking":[{"type":"text","text":"why"}]}]}),
            Value::Null,
        ),
        chunk(json!({"content":""}), Value::Null),
        chunk(
            json!({"content":[{"type":"thinking","thinking":[{"type":"text","text":" more"}]},{"type":"text","text":"answer"}]}),
            Value::Null,
        ),
        chunk(
            json!({"tool_calls":[{"index":1,"id":"abcdefghi","function":{"name":"echo","arguments":{"v":[{},[],null]}}},
            {"index":0,"id":"null","function":{"name":"echo","arguments":"{\"n\":"}}]}),
            Value::Null,
        ),
        chunk(
            json!({"tool_calls":[{"index":0,"function":{"arguments":"18446744073709551615}"}}]}),
            json!("tool_calls"),
        ),
    ]);
    assert_eq!(
        events,
        vec![
            Delta::Thinking("why".into()),
            Delta::Thinking(" more".into()),
            Delta::Text("answer".into()),
            Delta::ToolCallStart {
                id: "m00000000".into(),
                name: "echo".into()
            },
            Delta::ToolCallArgs("{\"n\":18446744073709551615}".into()),
            Delta::ToolCallStart {
                id: "abcdefghi".into(),
                name: "echo".into()
            },
            Delta::ToolCallArgs("{\"v\":[{},[],null]}".into()),
            Delta::Stop(StopReason::ToolUse)
        ]
    );
}

#[test]
fn every_cache_alias_and_late_cumulative_usage_preserves_measured_paid_tokens() {
    for alias in [
        json!({"promptTokensDetails":{"cachedTokens":40}}),
        json!({"prompt_tokens_details":{"cached_tokens":40}}),
        json!({"promptTokenDetails":{"cachedTokens":40}}),
        json!({"prompt_token_details":{"cached_tokens":40}}),
        json!({"numCachedTokens":40}),
        json!({"num_cached_tokens":40}),
    ] {
        let mut usage = alias;
        usage["prompt_tokens"] = json!(100);
        usage["completion_tokens"] = json!(7);
        let (events, state) = run(&[
            chunk(json!({"content":"done"}), json!("stop")),
            json!({"choices":[],"usage":usage}),
        ]);
        assert_eq!(
            (
                state.usage.input,
                state.usage.output,
                state.usage.cache_read
            ),
            (60, 7, 40)
        );
        assert_eq!(state.measured_usage.input, Some(60));
        assert_eq!(state.usage_anomalies, 0);
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, Delta::Stop(_)))
                .count(),
            1
        );
    }
    let (_, state) =
        run(&[json!({"choices":[],"usage":{"prompt_tokens":10,"num_cached_tokens":20}})]);
    assert_eq!((state.usage.input, state.usage.cache_read), (0, 10));
    assert!(state.usage_anomalies > 0);
}

#[test]
fn malformed_chunks_bad_calls_and_unknown_finishes_never_report_success() {
    for bad in [
        json!({}),
        chunk(json!({"content":{"type":"text","text":"bad"}}), Value::Null),
        chunk(
            json!({"content":[{"type":"thinking","thinking":"bad"}]}),
            Value::Null,
        ),
        chunk(
            json!({"tool_calls":[{"index":0,"id":"a","function":{"name":"echo","arguments":"{"}}]}),
            json!("tool_calls"),
        ),
        chunk(json!({}), json!("future_reason")),
    ] {
        let (events, _) = run(&[bad, chunk(json!({}), json!("stop"))]);
        assert_eq!(events.last(), Some(&Delta::Stop(StopReason::Error)));
    }
    for reason in ["length", "model_length"] {
        assert_eq!(
            run(&[chunk(json!({}), json!(reason))]).0.last(),
            Some(&Delta::Stop(StopReason::Length))
        );
    }
}

#[test]
fn strict_tools_are_opt_in_and_raw_session_identity_never_reaches_the_request() {
    let wire = adapter("mistral-conversations");
    let mut model = model();
    let context: Context = serde_json::from_value(json!({"messages":[],"tools":[{"name":"echo","description":"Echo","strict":"require",
        "parameters":{"type":"object","properties":{"v":{"type":"string"}},"required":["v"],"additionalProperties":false}}]})).expect("context");
    assert!(wire.prepare(&model, &context, &Options::default()).is_err());
    model.compat = Some(Compat {
        supports_strict_mode: Some(true),
        supports_prompt_cache_key: Some(true),
        ..Default::default()
    });
    let mut options = Options {
        session_id: Some("private-session".into()),
        ..Default::default()
    };
    let (body, _) = wire.prepare(&model, &context, &options).expect("strict");
    assert_eq!(body["tools"][0]["function"]["strict"], true);
    assert!(!body.to_string().contains("private-session"));
    assert!(body.get("prompt_cache_key").is_none());
    options.provider_session = Some("opaque".into());
    assert_eq!(
        wire.request(&model, &context, &options)["prompt_cache_key"],
        "opaque"
    );
}

#[test]
fn ordered_system_updates_and_tool_declaration_changes_use_the_existing_neutral_projection() {
    let mut model = model();
    model.compat = Some(Compat {
        supports_mid_convo_system_messages: Some(true),
        ..Default::default()
    });
    let context: Context = serde_json::from_value(json!({"system":"base","tools":[{"name":"echo","description":"Echo","parameters":{}}],
        "messages":[{"role":"user","content":[{"type":"text","text":"first"}]},
            {"role":"system","content":[{"type":"text","text":"later"},{"type":"tool_changes","added":[],"removed":["echo"]}]},
            {"role":"user","content":[{"type":"text","text":"second"}]}]})).expect("context");
    let (body, _) = adapter("mistral-conversations")
        .prepare(&model, &context, &Options::default())
        .expect("prepared");
    assert_eq!(body["messages"][0]["content"], "base");
    assert_eq!(body["messages"][1]["role"], "user");
    assert_eq!(
        body["messages"][2],
        json!({"role":"system","content":[{"type":"text","text":"later"}]})
    );
    assert_eq!(body["messages"][3]["role"], "user");
    assert!(body.get("tools").is_none());
}
