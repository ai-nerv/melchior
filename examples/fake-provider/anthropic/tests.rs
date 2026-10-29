use super::*;
use melchior::mind::lua::adapter::{LuaAdapter, engine_with_builtins};
use melchior::mind::model::{Context, Message, StopReason, Usage};
use melchior::mind::provider::api::{Adapter, Delta, Options, StreamState};
use melchior::mind::provider::client::{Call, Client};

fn turn(value: Value) -> Turn {
    serde_json::from_value(value).expect("turn")
}

#[test]
fn default_chat_payloads_are_unchanged_and_new_unsupported_fields_refuse() {
    let chat =
        turn(json!({"events":[{"text":"hi"},{"usage":{"input":4,"output":2}},{"finish":"stop"}]}));
    chat.validate().expect("legacy chat");
    assert_eq!(chat.dialect, super::super::Dialect::Chat);
    let values: Vec<_> = chat
        .events
        .iter()
        .map(|event| event.payload().expect("payload"))
        .collect();
    assert_eq!(
        values,
        json!([
            {"choices":[{"index":0,"delta":{"content":"hi"},"finish_reason":null}]},
            {"choices":[],"usage":{"prompt_tokens":4,"completion_tokens":2}},
            {"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}
        ])
        .as_array()
        .expect("array")
        .clone()
    );
    for invalid in [
        json!({"events":[{"signature":"opaque"}]}),
        json!({"usage":{"input":1},"events":[]}),
    ] {
        assert!(turn(invalid).validate().is_err());
    }
    assert!(serde_json::from_value::<Turn>(json!({"dialect":"unknown","events":[]})).is_err());
}

#[test]
fn named_lifecycle_preserves_raw_tools_signatures_and_supplied_usage_in_actual_decoder() {
    let args = "{\"cache_control\":\"literal\",\"nested\":[null,[],{}]}";
    let turn = turn(
        json!({"dialect":"anthropic-messages","usage":{"input":9,"output":7,"cache_read":20,"cache_write":30},
        "events":[{"thinking":"reasoning Σ"},{"signature":"piece-one"},{"signature":"piece-two"},
            {"tool_call":{"id":"one","name":"read","arguments":args}},
            {"tool_call":{"id":"two","name":"read","arguments":"false"}},{"finish":"tool_use"}]}),
    );
    turn.validate().expect("valid named lifecycle");
    let mut encoder = Encoder::new(turn.usage.expect("counts"));
    let mut frames = vec![encoder.start()];
    for event in &turn.events {
        frames.extend(encoder.event(event).expect("event"));
    }
    let starts: Vec<_> = frames
        .iter()
        .filter(|(name, _)| *name == "content_block_start")
        .map(|(_, value)| value["index"].clone())
        .collect();
    let stops: Vec<_> = frames
        .iter()
        .filter(|(name, _)| *name == "content_block_stop")
        .map(|(_, value)| value["index"].clone())
        .collect();
    assert_eq!(starts, vec![json!(0), json!(1), json!(2)]);
    assert_eq!(starts, stops);
    assert_eq!(frames.last().expect("last frame").0, "message_stop");
    let bytes: String = frames.iter().map(|frame| render(frame, "\r\n")).collect();
    let wire = LuaAdapter::new(
        engine_with_builtins().expect("builtins"),
        "anthropic-messages",
    )
    .expect("adapter");
    let mut parser = melchior::mind::provider::sse::Parser::new();
    let mut state = StreamState::default();
    let mut deltas = Vec::new();
    for byte in bytes.as_bytes() {
        parser
            .feed(&[*byte], |event| {
                deltas.extend(wire.on_event(&mut state, &event))
            })
            .expect("one-byte SSE split");
    }
    assert!(parser.finish().expect("finish").is_none());
    assert_eq!(
        deltas,
        vec![
            Delta::Usage(Usage {
                input: 9,
                output: 0,
                cache_read: 20,
                cache_write: 30,
                ..Usage::default()
            }),
            Delta::Thinking("reasoning Σ".into()),
            Delta::Signature("piece-one".into()),
            Delta::Signature("piece-two".into()),
            Delta::ToolCallStart {
                id: "one".into(),
                name: "read".into()
            },
            Delta::ToolCallArgs(args.into()),
            Delta::ToolCallStart {
                id: "two".into(),
                name: "read".into()
            },
            Delta::ToolCallArgs("false".into()),
            Delta::Usage(Usage {
                input: 9,
                output: 7,
                cache_read: 20,
                cache_write: 30,
                ..Usage::default()
            }),
            Delta::Stop(StopReason::ToolUse)
        ]
    );
    assert_eq!(state.usage_anomalies, 0);
}

#[test]
fn malformed_named_sequences_do_not_silently_become_chat_or_unsigned_thinking() {
    for events in [
        json!([{"signature":"no-thinking"},{"finish":"end_turn"}]),
        json!([{"text":"not-thinking"},{"signature":"wrong"},{"finish":"end_turn"}]),
        json!([{"thinking":"first"},{"signature":"signed"},{"thinking":"too-late"},{"finish":"end_turn"}]),
        json!([{"finish":"end_turn"},{"text":"after-stop"}]),
        json!([{"metrics":{"input_tokens":1}},{"finish":"end_turn"}]),
        json!([{"usage":{"input":1,"output":2}},{"finish":"end_turn"}]),
        json!([{"finish":"made-up"}]),
        json!([{"text":"no-finish"}]),
    ] {
        assert!(
            turn(json!({"dialect":"anthropic-messages","events":events}))
                .validate()
                .is_err()
        );
    }
    turn(json!({"dialect":"anthropic-messages","events":[{"thinking":"partial"},{"drop":null}]}))
        .validate()
        .expect("intentional drop");
    let mut many = turn(json!({"events":[]}));
    many.events = (0..4097).map(|_| Event::Pause(0)).collect();
    assert!(many.validate().is_err());
}

#[test]
fn an_anthropic_refusal_needs_no_successful_stream_lifecycle() {
    turn(json!({"dialect":"anthropic-messages","refuse":{"status":400,"message":"unsupported cache_control"}}))
        .validate().expect("refusal instead of stream");
}

#[tokio::test]
async fn split_named_stream_runs_through_actual_http_transport_and_decoder() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let address = listener.local_addr().expect("address");
    let scripted = turn(
        json!({"dialect":"anthropic-messages","usage":{"input":3,"output":2,"cache_read":4,"cache_write":5},
        "events":[{"split":"text Σ"},{"finish":"end_turn"}]}),
    );
    scripted.validate().expect("script");
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        super::super::request(&mut socket)
            .await
            .expect("bounded request")
            .expect("body");
        stream(&mut socket, &scripted)
            .await
            .expect("actual fixture stream");
    });
    let provider: melchior::mind::provider::endpoint::Provider = serde_json::from_value(json!({"id":"fixture","name":"Fixture",
        "api":"anthropic-messages","auth":{"kind":"none"},"base_url":format!("http://{address}"),
        "compat":{"prompt_cache":"anthropic_ephemeral"},"models":[{"id":"test","name":"Test","context_window":8000,"max_tokens":100}]})).expect("provider");
    let wire = LuaAdapter::new(
        engine_with_builtins().expect("builtins"),
        "anthropic-messages",
    )
    .expect("adapter");
    let context = Context {
        messages: vec![Message::user("hello")],
        ..Context::default()
    };
    let options = Options::default();
    let mut deltas = Vec::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        Client::new().stream(
            &Call {
                adapter: &wire,
                provider: &provider,
                model: provider.models.first().expect("model"),
                context: &context,
                options: &options,
            },
            |delta| deltas.push(delta),
        ),
    )
    .await
    .expect("deadline")
    .expect("stream success");
    server.await.expect("server task");
    assert!(deltas.contains(&Delta::Text("text Σ".into())));
    assert!(deltas.contains(&Delta::Stop(StopReason::EndTurn)));
    assert!(deltas.contains(&Delta::Usage(Usage {
        input: 3,
        output: 2,
        cache_read: 4,
        cache_write: 5,
        ..Usage::default()
    })));
}

#[tokio::test]
async fn oversized_declared_request_is_refused_before_body_allocation() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let mut client = tokio::net::TcpStream::connect(listener.local_addr().expect("address"))
        .await
        .expect("connect");
    let (mut server, _) = listener.accept().await.expect("accept");
    client
        .write_all(
            format!(
                "POST / HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
                melchior::framing::MOST + 1
            )
            .as_bytes(),
        )
        .await
        .expect("header");
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        super::super::request(&mut server),
    )
    .await
    .expect("before any body")
    .expect_err("over cap");
    assert!(error.to_string().contains("frame cap"));
}
