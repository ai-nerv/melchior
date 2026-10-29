use super::{Call, Client, anthropic_answer};
use crate::mind::lua::adapter::{LuaAdapter, engine_with_builtins};
use crate::mind::model::{Content, Context, Message, Role, Tool};
use crate::mind::provider::api::{Adapter, Delta, Options};
use crate::mind::provider::endpoint::Provider;
use crate::mind::provider::testing::{Server, Step, chunk, headers};
use serde_json::json;
use std::cell::RefCell;
use std::sync::atomic::Ordering;
use std::time::Duration;

fn partial() -> Vec<Step> {
    let events = [
        json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"abandoned partial"}}),
    ];
    vec![
        headers(200),
        chunk(
            events
                .iter()
                .map(|event| {
                    format!(
                        "event: {}\ndata: {event}\n\n",
                        event["type"].as_str().expect("type")
                    )
                })
                .collect::<String>()
                .as_bytes(),
        ),
    ]
}

fn provider(server: &Server) -> Provider {
    serde_json::from_value(json!({"id":"fixture","name":"Fixture",
        "base_url":server.url,"api":"anthropic-messages","auth":{"kind":"none"},
        "compat":{"prompt_cache":"anthropic_ephemeral"},"models":[{"id":"fixture","name":"Fixture",
            "context_window":8000,"max_tokens":900}]}))
    .expect("provider")
}

fn context() -> Context {
    let mut signed = Message::assistant("");
    signed.content = vec![
        Content::Thinking {
            thinking: "exact reasoning".into(),
            signature: Some("opaque-signature".into()),
        },
        Content::ToolCall {
            id: "original-call".into(),
            name: "lookup".into(),
            arguments: json!({"values":[null,[],{}],"cache_control":"literal"}),
            thought_signature: None,
        },
    ];
    Context {
        system: Some("unchanged rules\r\n".into()),
        tools: vec![Tool {
            name: "lookup".into(),
            description: "Lookup".into(),
            parameters: json!({"type":"object","required":[],"properties":{}}),
        }],
        messages: vec![
            Message::user("inspect"),
            signed,
            Message {
                role: Role::Tool,
                content: vec![Content::ToolResult {
                    id: "original-call".into(),
                    name: "lookup".into(),
                    content: "exact result\r\n".into(),
                    is_error: false,
                    images: vec![],
                }],
                stop_reason: None,
                usage: None,
                error: None,
            },
        ],
    }
}

#[tokio::test]
async fn cancellation_and_partial_retry_preserve_exact_marked_requests_and_fresh_stream_state() {
    for cancel in [true, false] {
        let mut first = partial();
        first.push(if cancel {
            Step::Hold
        } else {
            Step::Bytes(b"0\r\n\r\n".to_vec())
        });
        let server = Server::start(vec![first, anthropic_answer(4, 2, 30, 0)]).await;
        let provider = provider(&server);
        let model = &provider.models[0];
        let adapter = LuaAdapter::new(
            engine_with_builtins().expect("builtins"),
            "anthropic-messages",
        )
        .expect("adapter");
        let context = context();
        let original = serde_json::to_value(&context).expect("context");
        let options = Options::default();
        let expected = adapter.request(model, &context, &options);
        assert_eq!(
            expected["messages"][1]["content"][0]["signature"],
            "opaque-signature"
        );
        assert_eq!(
            expected["messages"][1]["content"][1]["cache_control"],
            json!({"type":"ephemeral"})
        );
        let client = Client::with_base_delay(Duration::ZERO);
        let call = Call {
            adapter: &adapter,
            provider: &provider,
            model,
            context: &context,
            options: &options,
        };
        let frames = RefCell::new(Vec::new());
        let retries = RefCell::new(Vec::new());
        let deltas = RefCell::new(Vec::new());
        if cancel {
            let seen = tokio::sync::Notify::new();
            {
                let waiting = client.stream_observing(
                    &call,
                    |delta| {
                        if delta == Delta::Text("abandoned partial".into()) {
                            seen.notify_one();
                        }
                        deltas.borrow_mut().push(delta);
                    },
                    |retry| retries.borrow_mut().push(retry),
                    |frame| frames.borrow_mut().push(frame),
                );
                tokio::pin!(waiting);
                tokio::select! {
                    result = &mut waiting => panic!("held stream ended: {result:?}"),
                    () = seen.notified() => {},
                    () = tokio::time::sleep(Duration::from_secs(3)) => panic!("partial text barrier timed out"),
                }
            }
            server.wait_for(&server.closed, 1).await;
            assert_eq!(server.requests.load(Ordering::SeqCst), 1);
            assert!(retries.borrow().is_empty());
            assert_eq!(*deltas.borrow(), [Delta::Text("abandoned partial".into())]);
            deltas.borrow_mut().clear();
        }
        tokio::time::timeout(
            Duration::from_secs(3),
            client.stream_observing(
                &call,
                |delta| deltas.borrow_mut().push(delta),
                |retry| {
                    assert_eq!(*deltas.borrow(), [Delta::Text("abandoned partial".into())]);
                    deltas.borrow_mut().clear();
                    retries.borrow_mut().push(retry);
                },
                |frame| frames.borrow_mut().push(frame),
            ),
        )
        .await
        .expect("bounded stream")
        .expect("success");
        assert_eq!(retries.borrow().len(), usize::from(!cancel));
        assert_eq!(server.requests.load(Ordering::SeqCst), 2);
        assert_eq!(frames.borrow().len(), 2);
        assert_eq!(frames.borrow()[0], frames.borrow()[1]);
        let captured = server.bodies.lock().expect("bodies");
        assert_eq!(captured.len(), 2);
        assert_eq!(captured[0], captured[1]);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&captured[0]).expect("body"),
            expected
        );
        assert_eq!(serde_json::to_value(&context).expect("context"), original);
        assert_eq!(
            deltas
                .borrow()
                .iter()
                .filter_map(|delta| match delta {
                    Delta::Text(text) => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            ["synthetic answer"]
        );
        let final_usage = deltas
            .borrow()
            .iter()
            .rev()
            .find_map(|delta| match delta {
                Delta::Usage(usage) => Some(*usage),
                _ => None,
            })
            .expect("usage");
        assert_eq!(
            (
                final_usage.input,
                final_usage.output,
                final_usage.cache_read,
                final_usage.cache_write
            ),
            (4, 2, 30, 0)
        );
    }
}
