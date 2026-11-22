use melchior::mind::lua::adapter::LuaAdapter;
use melchior::mind::lua::engine::Engine;
use melchior::mind::model::{Context, Message, StopReason};
use melchior::mind::provider::api::{Delta, Options};
use melchior::mind::provider::client::{Call, Client};
use melchior::mind::provider::endpoint::{Auth, Provider};
use melchior::mind::provider::model::{Api, Model};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub fn supports(api: Api) -> bool {
    matches!(
        api,
        Api::OpenAiCompletions
            | Api::OpenAiResponses
            | Api::AnthropicMessages
            | Api::GoogleGenerativeAi
    )
}

fn response(api: Api) -> (&'static str, &'static str) {
    match api {
        Api::OpenAiCompletions => (
            "/chat/completions",
            concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"parity-ok\"},\"finish_reason\":null}]}\n\n",
                "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":2}}\n\n",
                "data: [DONE]\n\n"
            ),
        ),
        Api::OpenAiResponses => (
            "/responses",
            concat!(
                "data: {\"type\":\"response.output_text.delta\",\"delta\":\"parity-ok\"}\n\n",
                "data: {\"type\":\"response.completed\",\"response\":{\"output\":[],\"usage\":{\"input_tokens\":7,\"output_tokens\":2}}}\n\n"
            ),
        ),
        Api::AnthropicMessages => (
            "/v1/messages",
            concat!(
                "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"role\":\"assistant\",\"content\":[],\"usage\":{\"input_tokens\":7,\"output_tokens\":0}}}\n\n",
                "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
                "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"parity-ok\"}}\n\n",
                "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
                "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":2}}\n\n",
                "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
            ),
        ),
        Api::GoogleGenerativeAi => (
            "/models/parity:streamGenerateContent?alt=sse",
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"parity-ok\"}]},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":7,\"candidatesTokenCount\":2}}\n\n",
        ),
        _ => panic!("no text fixture for {api:?}"),
    }
}

pub async fn text_and_usage(declared: &Provider, api: Api) {
    let (path, reply) = response(api);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("fixture listener");
    let address = listener.local_addr().expect("fixture address");
    let server = tokio::spawn(async move {
        tokio::time::timeout(Duration::from_secs(5), async move {
            let (mut socket, _) = listener.accept().await.expect("fixture connection");
            let mut headers = Vec::new();
            while !headers.ends_with(b"\r\n\r\n") {
                assert!(headers.len() < 65536);
                headers.push(socket.read_u8().await.expect("request header"));
            }
            let headers = String::from_utf8(headers).expect("header encoding");
            assert!(headers.starts_with(&format!("POST {path} HTTP/1.1\r\n")), "unexpected route");
            assert!(!headers.to_ascii_lowercase().contains("authorization:"));
            let length = headers.lines().find_map(|line| line.to_ascii_lowercase()
                .strip_prefix("content-length:").and_then(|value| value.trim().parse::<usize>().ok()))
                .expect("content length");
            assert!(length < 65536);
            let mut bytes = vec![0; length];
            socket.read_exact(&mut bytes).await.expect("request body");
            let body: Value = serde_json::from_slice(&bytes).expect("request JSON");
            assert!(String::from_utf8(bytes).expect("request text").contains("parity-question"));
            let wire = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len());
            socket.write_all(wire.as_bytes()).await.expect("fixture response");
            body
        }).await.expect("server deadline")
    });
    let mut provider = declared.clone();
    provider.base_url = Some(format!("http://{address}"));
    provider.auth = Auth::None;
    let model: Model = serde_json::from_value(json!({"id":"parity","name":"Parity",
        "provider":provider.id,"api":api,"context_window":8192,"max_tokens":128,
        "compat":provider.compat}))
    .expect("fixture model");
    let mut engine = Engine::new();
    engine
        .run(include_str!("../../config/apis.lua"), "apis.lua")
        .expect("shipped protocols");
    let adapter = LuaAdapter::new(engine, api.as_str()).expect("registered adapter");
    let context = Context {
        messages: vec![Message::user("parity-question")],
        ..Context::default()
    };
    let options = Options::default();
    let call = Call {
        adapter: &adapter,
        provider: &provider,
        model: &model,
        context: &context,
        options: &options,
    };
    let mut deltas = Vec::new();
    let outcome = tokio::time::timeout(
        Duration::from_secs(5),
        Client::with_base_delay(Duration::ZERO).stream(&call, |delta| deltas.push(delta)),
    )
    .await;
    let body = server.await.expect("server task");
    outcome
        .expect("client deadline")
        .expect("successful fixture inference");
    assert!(
        deltas
            .iter()
            .any(|delta| matches!(delta, Delta::Text(text) if text == "parity-ok"))
    );
    assert_eq!(
        deltas
            .iter()
            .filter(|delta| matches!(delta, Delta::Stop(_)))
            .count(),
        1
    );
    assert!(
        deltas
            .iter()
            .any(|delta| matches!(delta, Delta::Stop(StopReason::EndTurn)))
    );
    let usage = deltas
        .iter()
        .filter_map(|delta| match delta {
            Delta::Usage(usage) => Some(usage),
            _ => None,
        })
        .next_back()
        .expect("reported usage");
    assert_eq!((usage.input, usage.output), (7, 2));
    if api != Api::GoogleGenerativeAi {
        assert_eq!(body["model"], "parity");
    }
}
