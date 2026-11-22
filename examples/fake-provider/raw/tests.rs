use super::*;
use melchior::mind::lua::adapter::{LuaAdapter, engine_with_builtins};
use melchior::mind::model::StopReason;
use melchior::mind::provider::api::{Adapter, Delta, StreamState};
use serde_json::{Value, json};

#[test]
fn literal_frames_escape_data_and_reject_names_limits_and_mixed_dialects() {
    let event: Event = serde_json::from_value(
        json!({"sse":{"name":"response.completed","data":{"text":"Σ\n\ndata: injected"}}}),
    )
    .expect("event");
    assert_eq!(
        frame(&event).expect("frame"),
        "event: response.completed\ndata: {\"text\":\"Σ\\n\\ndata: injected\"}\n\n"
    );
    for name in ["", "bad\nname", "bad:name", "Σ"] {
        let event: Event =
            serde_json::from_value(json!({"sse":{"name":name,"data":{}}})).expect("event");
        assert!(frame(&event).is_err());
    }
    let large: Event = serde_json::from_value(json!({"sse":{"data":"x".repeat(256 * 1024)}}))
        .expect("large frame");
    assert!(frame(&large).is_err());
    for value in [
        json!({"dialect":"raw-sse","events":[]}),
        json!({"dialect":"raw-sse","events":[{"text":"wrong kind"}]}),
        json!({"dialect":"raw-sse","usage":{"input":1},"events":[{"sse":{"data":{}}}]}),
        json!({"dialect":"anthropic-messages","events":[{"sse":{"data":{}}}]}),
        json!({"events":[{"sse":{"data":{}}}]}),
    ] {
        assert!(
            serde_json::from_value::<Turn>(value)
                .expect("turn")
                .validate()
                .is_err()
        );
    }
}

#[test]
fn image_corpus_rounds_decode_tools_text_usage_and_one_terminal_event() {
    let corpus: Value =
        serde_json::from_str(include_str!("../fixtures/image-rounds.json")).expect("corpus");
    for (api, values) in corpus.as_object().expect("protocols") {
        let turns: Vec<Turn> = serde_json::from_value(values.clone()).expect("turns");
        assert_eq!(turns.len(), 5);
        for (index, turn) in turns.iter().enumerate() {
            turn.validate().expect("valid fixture turn");
            if index == 0 {
                assert_eq!(turn.refuse.as_ref().expect("retry refusal").status, 503);
                continue;
            }
            let wire =
                LuaAdapter::new(engine_with_builtins().expect("builtins"), api).expect("adapter");
            let mut parser = melchior::mind::provider::sse::Parser::new();
            let mut state = StreamState::default();
            let mut deltas = Vec::new();
            for event in &turn.events {
                for byte in frame(event).expect("frame").bytes() {
                    parser
                        .feed(&[byte], |event| {
                            deltas.extend(wire.on_event(&mut state, &event))
                        })
                        .expect("split SSE");
                }
            }
            assert!(parser.finish().expect("finish").is_none());
            let stops: Vec<_> = deltas
                .iter()
                .filter(|delta| matches!(delta, Delta::Stop(_)))
                .collect();
            assert_eq!(
                stops,
                vec![&Delta::Stop(if index == 1 {
                    StopReason::ToolUse
                } else {
                    StopReason::EndTurn
                })],
                "{api} round {index} terminal events"
            );
            if index == 1 {
                let id = if api == "google-generative-ai" {
                    "melchior-google-call-1"
                } else {
                    "image-call"
                };
                assert!(deltas.contains(&Delta::ToolCallStart {
                    id: id.into(),
                    name: "paint".into()
                }));
                assert!(deltas.contains(&Delta::ToolCallArgs("{}".into())));
            } else {
                let expected = match index {
                    2 => "image stored",
                    3 => "image resumed",
                    _ => "image continued",
                };
                let text: String = deltas
                    .iter()
                    .filter_map(|delta| match delta {
                        Delta::Text(text) => Some(text.as_str()),
                        _ => None,
                    })
                    .collect();
                assert_eq!(text, expected, "{api} round {index} text");
            }
            assert!(deltas.iter().any(
                |delta| matches!(delta, Delta::Usage(usage) if usage.input==7 && usage.output==2)
            ));
            assert_eq!(state.usage_anomalies, 0);
        }
    }
}

#[tokio::test]
async fn literal_stream_uses_chunked_http_without_inventing_chat_termination() {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let address = listener.local_addr().expect("address");
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("connection");
        super::super::request(&mut socket).await.expect("request");
        let turn: Turn = serde_json::from_value(json!({"dialect":"raw-sse","events":[{"sse":{"name":"response.completed","data":{"type":"response.completed"}}}]})).expect("turn");
        stream(&mut socket, &turn).await.expect("stream");
    });
    let mut client = tokio::net::TcpStream::connect(address)
        .await
        .expect("connect");
    client
        .write_all(b"POST /responses HTTP/1.1\r\nContent-Length: 2\r\n\r\n{}")
        .await
        .expect("request");
    let mut bytes = Vec::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        client.read_to_end(&mut bytes),
    )
    .await
    .expect("deadline")
    .expect("response");
    server.await.expect("server");
    let text = String::from_utf8(bytes).expect("HTTP");
    assert!(text.starts_with("HTTP/1.1 200 Fake\r\nContent-Type: text/event-stream"));
    assert!(
        text.contains("event: response.completed\ndata: {\"type\":\"response.completed\"}\n\n")
    );
    assert!(text.ends_with("0\r\n\r\n"));
    assert!(!text.contains("[DONE]"));
}
