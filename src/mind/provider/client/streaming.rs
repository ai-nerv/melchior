use super::*;
use crate::mind::lua::adapter::{LuaAdapter, engine_with_builtins};
use crate::mind::provider::testing::{Server, Step, chunk, headers, reply};
use serde_json::json;
use std::time::Duration;

fn model() -> Model {
    serde_json::from_value(
        json!({"id":"test","name":"Test","context_window":200000,"max_tokens":1000}),
    )
    .expect("model")
}

fn provider(url: &str) -> Provider {
    Provider {
        id: "local".into(),
        name: "Local".into(),
        base_url: Some(url.into()),
        api: crate::mind::provider::model::Api::OpenAiCompletions,
        auth: Auth::None,
        compat: None,
        models: Vec::new(),
        discover: false,
        details: None,
        avoid: Vec::new(),
    }
}

fn adapter() -> LuaAdapter {
    LuaAdapter::new(
        engine_with_builtins().expect("protocols"),
        "openai-completions",
    )
    .expect("adapter")
}

fn source() -> String {
    [": keepalive\r\n".into(), format!("data: {}\r\n\r\n", json!({"choices":[{"delta":{"content":"café 🦀 漢字"}}]})),
        format!("data: {}\n\n", json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call1","function":{"name":"weather","arguments":"{\"city\":\"Zürich\"}"}}]}}]})),
        format!("data: {}\r\r", json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":7,"completion_tokens":3}})),
        "data: [DONE]".into()].concat()
}

async fn stream(client: &Client, url: &str) -> Result<Vec<Delta>, ProviderError> {
    let (adapter, provider, model, context, options) = (
        adapter(),
        provider(url),
        model(),
        Context::default(),
        Options::default(),
    );
    let mut events = Vec::new();
    tokio::time::timeout(
        Duration::from_secs(3),
        client.stream(
            &Call {
                adapter: &adapter,
                provider: &provider,
                model: &model,
                context: &context,
                options: &options,
            },
            |event| events.push(event),
        ),
    )
    .await
    .expect("local stream deadline")?;
    Ok(events)
}

#[tokio::test]
async fn every_http_byte_split_preserves_text_tool_arguments_usage_and_order() {
    let source = source();
    let bytes = source.as_bytes();
    let mut replies = vec![reply(200, bytes)];
    for split in 0..=bytes.len() {
        let mut parts = vec![headers(200)];
        if split > 0 {
            parts.push(chunk(&bytes[..split]));
        }
        parts.push(Step::Pause(Duration::from_millis(1)));
        if split < bytes.len() {
            parts.push(chunk(&bytes[split..]));
        }
        parts.push(Step::Bytes(b"0\r\n\r\n".to_vec()));
        replies.push(parts);
    }
    for width in [1, 2, 3, 7, 17] {
        let mut parts = vec![headers(200)];
        parts.extend(bytes.chunks(width).map(chunk));
        parts.push(Step::Bytes(b"0\r\n\r\n".to_vec()));
        replies.push(parts);
    }
    let server = Server::start(replies).await;
    let client = Client::with_base_delay(Duration::ZERO);
    let expected = stream(&client, &server.url).await.expect("unsplit");
    assert!(
        expected.contains(&Delta::Text("café 🦀 漢字".into())),
        "{expected:?}"
    );
    assert!(
        expected.contains(&Delta::ToolCallArgs("{\"city\":\"Zürich\"}".into())),
        "{expected:?}"
    );
    assert!(
        expected
            .iter()
            .any(|d| matches!(d, Delta::Usage(u) if u.input == 7 && u.output == 3)),
        "{expected:?}"
    );
    for split in 0..=bytes.len() {
        assert_eq!(
            stream(&client, &server.url).await.expect("split stream"),
            expected,
            "split {split}"
        );
    }
    for width in [1, 2, 3, 7, 17] {
        assert_eq!(
            stream(&client, &server.url)
                .await
                .expect("partitioned stream"),
            expected,
            "width {width}"
        );
    }
}

#[tokio::test]
async fn malformed_tail_preserves_completed_events_and_usage_at_every_http_split() {
    let valid = source();
    let bytes = [valid.as_bytes(), b"\n\ndata: \xff\n\n"].concat();
    let mut replies = vec![reply(200, valid.as_bytes()), reply(200, &bytes)];
    for split in 0..=bytes.len() {
        let mut parts = vec![headers(200)];
        if split > 0 {
            parts.push(chunk(&bytes[..split]));
        }
        parts.push(Step::Pause(Duration::from_millis(1)));
        if split < bytes.len() {
            parts.push(chunk(&bytes[split..]));
        }
        parts.push(Step::Bytes(b"0\r\n\r\n".to_vec()));
        replies.push(parts);
    }
    for width in [1, 2, 3, 7, 17] {
        let mut parts = vec![headers(200)];
        parts.extend(bytes.chunks(width).map(chunk));
        parts.push(Step::Bytes(b"0\r\n\r\n".to_vec()));
        replies.push(parts);
    }
    let count = replies.len() - 1;
    let server = Server::start(replies).await;
    let client = Client::with_base_delay(Duration::ZERO);
    let expected = stream(&client, &server.url).await.expect("valid baseline");
    assert!(
        expected
            .iter()
            .any(|d| matches!(d, Delta::Usage(u) if u.input == 7 && u.output == 3))
    );
    for partition in 0..count {
        let (adapter, provider, model, context, options) = (
            adapter(),
            provider(&server.url),
            model(),
            Context::default(),
            Options::default(),
        );
        let mut events = Vec::new();
        let error = tokio::time::timeout(
            Duration::from_secs(3),
            client.attempt(
                &Call {
                    adapter: &adapter,
                    provider: &provider,
                    model: &model,
                    context: &context,
                    options: &options,
                },
                |event| events.push(event),
            ),
        )
        .await
        .expect("malformed stream deadline")
        .expect_err("invalid trailer");
        assert_eq!(error.class, RetryClass::Transport);
        assert_eq!(events, expected, "partition {partition}");
    }
}

#[tokio::test]
async fn malformed_utf8_retries_with_a_fresh_parser_and_adapter_state() {
    let abandoned = format!("data: {}\n\n", json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"abandoned","function":{"name":"old","arguments":"{"}}]}}]})).into_bytes();
    let delivered = std::sync::Arc::new(tokio::sync::Notify::new());
    let server = Server::start(vec![
        vec![
            headers(200),
            chunk(&abandoned),
            Step::Wait(std::sync::Arc::clone(&delivered)),
            chunk(b"event: abandoned\r\ndata: \xff\n\n"),
            Step::Bytes(b"0\r\n\r\n".to_vec()),
        ],
        reply(200, source().as_bytes()),
    ])
    .await;
    let (adapter, provider, model, context, options) = (
        adapter(),
        provider(&server.url),
        model(),
        Context::default(),
        Options::default(),
    );
    let events = std::cell::RefCell::new(Vec::new());
    let retries = std::cell::Cell::new(0);
    tokio::time::timeout(
        Duration::from_secs(3),
        Client::with_base_delay(Duration::ZERO).stream_reporting(
            &Call {
                adapter: &adapter,
                provider: &provider,
                model: &model,
                context: &context,
                options: &options,
            },
            |event| {
                if matches!(&event, Delta::ToolCallStart {id,..} if id == "abandoned") {
                    delivered.notify_one();
                }
                events.borrow_mut().push(event);
            },
            |_| {
                assert!(
                    events
                        .borrow()
                        .iter()
                        .any(|d| matches!(d, Delta::ToolCallStart {id,..} if id == "abandoned"))
                );
                events.borrow_mut().clear();
                retries.set(retries.get() + 1);
            },
        ),
    )
    .await
    .expect("retry deadline")
    .expect("retried stream");
    let events = events.into_inner();
    assert!(events.contains(&Delta::Text("café 🦀 漢字".into())));
    assert!(
        events
            .iter()
            .any(|d| matches!(d, Delta::ToolCallStart {id,..} if id == "call1"))
    );
    assert!(
        !events
            .iter()
            .any(|d| matches!(d, Delta::ToolCallStart {id,..} if id == "abandoned"))
    );
    assert_eq!(retries.get(), 1);
    assert_eq!(server.requests.load(std::sync::atomic::Ordering::SeqCst), 2);
}

#[tokio::test]
async fn bounded_error_bodies_keep_status_retry_policy_and_hide_echoed_credentials() {
    for (status, class) in [
        (400, RetryClass::Overflow),
        (401, RetryClass::Auth),
        (429, RetryClass::Throttle),
        (503, RetryClass::Overload),
    ] {
        let server = Server::start(vec![reply(
            status,
            b"context length exceeded SECRET-bearer-123",
        )])
        .await;
        let client = Client::new();
        let (adapter, provider, model, context, options) = (
            adapter(),
            provider(&server.url),
            model(),
            Context::default(),
            Options::default(),
        );
        let why = client
            .attempt(
                &Call {
                    adapter: &adapter,
                    provider: &provider,
                    model: &model,
                    context: &context,
                    options: &options,
                },
                |_| {},
            )
            .await
            .expect_err("HTTP failure");
        assert_eq!(why.class, class);
        assert!(!why.to_string().contains("SECRET"));
    }
}

#[tokio::test]
async fn provider_transport_errors_exclude_sensitive_endpoint_urls() {
    let (adapter, provider, model, context, options) = (
        adapter(),
        provider("unsupported://SECRET-token/path?key=SECRET-query"),
        model(),
        Context::default(),
        Options::default(),
    );
    let why = Client::new()
        .attempt(
            &Call {
                adapter: &adapter,
                provider: &provider,
                model: &model,
                context: &context,
                options: &options,
            },
            |_| {},
        )
        .await
        .expect_err("unsupported transport");
    assert_eq!(why.class, RetryClass::Transport);
    assert!(!why.to_string().contains("SECRET"), "{why}");
}

#[tokio::test]
async fn stalled_and_oversized_errors_are_bounded_and_retry_only_when_appropriate() {
    for stalled in [true, false] {
        for (status, attempts) in [(401, 1), (503, 4)] {
            let replies = (0..attempts)
                .map(|_| {
                    if stalled {
                        vec![headers(status), Step::Hold]
                    } else {
                        reply(status, &vec![b'x'; control::LIMITS.bytes + 1])
                    }
                })
                .collect();
            let server = Server::start(replies).await;
            let mut client = Client::with_base_delay(Duration::ZERO);
            client.control.timeout = Duration::from_millis(50);
            let start = std::time::Instant::now();
            let why = tokio::time::timeout(Duration::from_secs(3), stream(&client, &server.url))
                .await
                .expect("bounded request")
                .expect_err("invalid error response");
            assert!(start.elapsed() < Duration::from_secs(3));
            assert_eq!(why.class, RetryClass::of_status(status));
            assert!(
                why.to_string()
                    .contains(if stalled { "deadline" } else { "byte limit" })
            );
            assert_eq!(
                server.requests.load(std::sync::atomic::Ordering::SeqCst),
                attempts
            );
        }
    }
}

#[tokio::test]
async fn successful_streams_outlive_control_deadlines_and_cancellation_closes_them() {
    let large = format!(
        "data: {}\n\n",
        json!({"choices":[{"delta":{"content":"x".repeat(control::LIMITS.bytes + 1)}}]})
    );
    let server = Server::start(vec![
        vec![
            headers(200),
            Step::Pause(Duration::from_millis(150)),
            chunk(large.as_bytes()),
            chunk(source().as_bytes()),
            Step::Bytes(b"0\r\n\r\n".to_vec()),
        ],
        vec![headers(200), Step::Hold],
    ])
    .await;
    let mut client = Client::new();
    client.control.timeout = Duration::from_millis(25);
    assert!(stream(&client, &server.url).await.is_ok());
    {
        let waiting = stream(&client, &server.url);
        tokio::pin!(waiting);
        tokio::select! {
            _ = &mut waiting => panic!("held stream ended"),
            () = server.wait_for(&server.requests, 2) => {}
        }
    }
    server.wait_for(&server.closed, 1).await;
}

#[test]
fn what_a_dialect_keeps_for_itself_is_not_sent() {
    let built = json!({"model": "m", "state": "s", "__scratch": {"fields": {"safe": {}}}});
    let (sent, kept) = parted(built);
    assert_eq!(sent, json!({"model": "m", "state": "s"}));
    assert_eq!(kept, Some(json!({"fields": {"safe": {}}})));
    assert_eq!(parted(json!({"model": "m"})), (json!({"model": "m"}), None));
}

#[tokio::test]
async fn a_reply_that_is_one_document_reaches_the_dialect_whole_however_it_is_cut() {
    // Not a stream: no `data:` line ever comes, so no event does. Cut in the middle of a
    // multi-byte character to be sure it is put together before it is read.
    let document = json!({
        "answers": { "answer": { "type": "noul", "noul": 0.91 } },
        "usage": { "input_tokens": 12, "output_tokens": 3, "cost": 0.000_002 },
        "note": "café 🦀",
    })
    .to_string();
    let bytes = document.as_bytes();
    let at = bytes
        .iter()
        .position(|b| *b >= 0x80)
        .expect("a wide character")
        + 1;
    let replies = vec![
        reply(200, bytes),
        vec![
            headers(200),
            chunk(&bytes[..at]),
            Step::Pause(Duration::from_millis(1)),
            chunk(&bytes[at..]),
            Step::Bytes(b"0\r\n\r\n".to_vec()),
        ],
    ];
    let server = Server::start(replies).await;
    let deciding =
        LuaAdapter::new(engine_with_builtins().expect("protocols"), "decisions").expect("adapter");
    for _ in 0..2 {
        let (provider, model, context, options) = (
            provider(&server.url),
            model(),
            Context::default(),
            Options::default(),
        );
        let mut events = Vec::new();
        Client::new()
            .stream(
                &Call {
                    adapter: &deciding,
                    provider: &provider,
                    model: &model,
                    context: &context,
                    options: &options,
                },
                |event| events.push(event),
            )
            .await
            .expect("answered");
        assert!(
            events.contains(&Delta::Text("yes (0.91)".into())),
            "{events:?}"
        );
        assert!(events.contains(&Delta::Stop(crate::mind::model::StopReason::EndTurn)));
    }
}
