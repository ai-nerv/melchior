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

/// A provider that cannot be reached is named, with the reason the connection failed.
mod unreachable {
    use super::*;

    /// A port nothing on this machine listens on, bound and released so the refusal is certain.
    fn closed_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
        let port = listener.local_addr().expect("its address").port();
        drop(listener);
        port
    }

    #[tokio::test]
    async fn the_failure_names_the_provider_and_what_went_wrong() {
        // It used to say "error sending request" and nothing else: which provider, and whether
        // the host was down, refused or unroutable, were two error sources below what was shown.
        let client = Client::with_base_delay(Duration::from_millis(1));
        let url = format!("http://127.0.0.1:{}/v1", closed_port());
        let said = stream(&client, &url)
            .await
            .expect_err("nothing listens there");
        assert_eq!(said.class, RetryClass::Transport);
        assert!(
            said.message.starts_with("could not reach "),
            "{}",
            said.message
        );
        assert!(
            said.message.to_lowercase().contains("refused"),
            "the reason underneath is kept: {}",
            said.message
        );
        assert_ne!(said.message, "error sending request");
    }

    #[tokio::test]
    async fn the_url_is_never_in_the_message() {
        // Some providers carry the key in the query string, so the URL stays out of what is said.
        let client = Client::with_base_delay(Duration::from_millis(1));
        let url = format!("http://127.0.0.1:{}/v1?key=SENTINEL", closed_port());
        let said = stream(&client, &url)
            .await
            .expect_err("nothing listens there");
        assert!(!said.message.contains("SENTINEL"), "{}", said.message);
        assert!(!said.message.contains("127.0.0.1"), "{}", said.message);
    }
}

/// How long an unreachable provider is waited on before the failure is the answer.
mod giving_up {
    use super::*;

    /// A connection error as hyper nests it: a message, with the operating system's reason under.
    #[derive(Debug)]
    struct Connecting(std::io::Error);

    impl std::fmt::Display for Connecting {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("tcp connect error")
        }
    }

    impl std::error::Error for Connecting {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.0)
        }
    }

    /// The top of the chain, as reqwest shows it: nothing useful of its own.
    #[derive(Debug)]
    struct Sending(Connecting);

    impl std::fmt::Display for Sending {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("error sending request")
        }
    }

    impl std::error::Error for Sending {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.0)
        }
    }

    fn sending(kind: std::io::ErrorKind) -> Sending {
        Sending(Connecting(std::io::Error::from(kind)))
    }

    #[test]
    fn no_route_to_the_host_is_not_asked_again() {
        // Four attempts and fifty seconds of backoff did not put a machine back on the network.
        for kind in [
            std::io::ErrorKind::HostUnreachable,
            std::io::ErrorKind::NetworkUnreachable,
        ] {
            let (said, routeless) = underneath(&sending(kind));
            assert!(routeless, "{kind:?}");
            assert!(said.is_some_and(|said| !said.contains("error sending request")));
        }
    }

    #[test]
    fn a_refused_connection_still_is() {
        // That host is there, and a daemon restarting on it answers a few seconds later.
        let (_, routeless) = underneath(&sending(std::io::ErrorKind::ConnectionRefused));
        assert!(!routeless);
    }

    #[test]
    fn a_failure_that_is_not_worth_repeating_is_not_repeated() {
        let failed = ProviderError::new(RetryClass::Transport, "gone").finally();
        assert_eq!(
            failed.class,
            RetryClass::Transport,
            "still a transport failure"
        );
        assert!(!failed.again);
        assert!(ProviderError::new(RetryClass::Transport, "blip").again);
        assert!(!ProviderError::new(RetryClass::Auth, "denied").again);
    }

    #[tokio::test]
    async fn a_refused_connection_is_retried_through_the_client() {
        let client = Client::with_base_delay(Duration::from_millis(1));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
        let port = listener.local_addr().expect("its address").port();
        drop(listener);
        let (adapter, provider, model, context, options) = (
            adapter(),
            provider(&format!("http://127.0.0.1:{port}/v1")),
            model(),
            Context::default(),
            Options::default(),
        );
        let mut retried = 0;
        let outcome = client
            .stream_reporting(
                &Call {
                    adapter: &adapter,
                    provider: &provider,
                    model: &model,
                    context: &context,
                    options: &options,
                },
                |_| {},
                |_| retried += 1,
            )
            .await;
        assert!(outcome.is_err());
        assert_eq!(retried, MAX_ATTEMPTS - 1, "every attempt was made");
    }
}
