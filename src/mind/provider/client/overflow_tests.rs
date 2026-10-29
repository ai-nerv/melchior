use super::{Call, Client};
use crate::mind::lua::adapter::{LuaAdapter, engine_with_builtins};
use crate::mind::model::{Context, Message};
use crate::mind::provider::api::{Adapter, Options};
use crate::mind::provider::endpoint::Provider;
use crate::mind::provider::retry::RetryClass;
use crate::mind::provider::testing::{Server, reply};
use serde_json::json;
use std::sync::atomic::Ordering;
use std::time::Duration;

mod recovery;

#[tokio::test]
async fn input_and_untyped_length_refusals_send_once_without_changing_output_or_history() {
    use crate::mind::model::overflow::{Detail, Kind};
    for (message, kind) in [
        (
            "prompt is too long: 250000 tokens > 200000 maximum",
            Kind::Input,
        ),
        ("context length exceeded", Kind::Unknown),
    ] {
        let error =
            json!({"type":"error","error":{"type":"invalid_request_error","message":message}});
        let server = Server::start(vec![reply(400, error.to_string().as_bytes())]).await;
        let provider: Provider = serde_json::from_value(json!({"id":"fixture","name":"Fixture",
            "base_url":server.url,"api":"anthropic-messages","auth":{"kind":"none"},
            "models":[{"id":"fixture","name":"Fixture","context_window":8000,"max_tokens":900}]}))
        .expect("provider");
        let model = &provider.models[0];
        let adapter = LuaAdapter::new(
            engine_with_builtins().expect("builtins"),
            "anthropic-messages",
        )
        .expect("adapter");
        let context = Context {
            system: Some("exact rules\r\n".into()),
            messages: vec![Message::user("exact input")],
            ..Default::default()
        };
        let options = Options {
            max_tokens: Some(77),
            ..Default::default()
        };
        let expected = adapter.request(model, &context, &options);
        let original = context.clone();
        let mut retries = Vec::new();
        let mut deltas = Vec::new();
        let error = tokio::time::timeout(
            Duration::from_secs(3),
            Client::with_base_delay(Duration::ZERO).stream_reporting(
                &Call {
                    adapter: &adapter,
                    provider: &provider,
                    model,
                    context: &context,
                    options: &options,
                },
                |delta| deltas.push(delta),
                |retry| retries.push(retry),
            ),
        )
        .await
        .expect("bounded refusal")
        .expect_err("overflow");
        assert_eq!(error.class, RetryClass::Overflow);
        assert_eq!(
            error.overflow,
            Some(Detail {
                kind,
                prompt_tokens: None,
                limit_tokens: None,
                requested_output_tokens: None
            })
        );
        assert!(!error.again);
        assert!(deltas.is_empty());
        assert!(retries.is_empty());
        assert_eq!(server.requests.load(Ordering::SeqCst), 1);
        let bodies = server.bodies.lock().expect("bodies");
        assert_eq!(bodies.len(), 1);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&bodies[0]).expect("body"),
            expected
        );
        assert_eq!(expected["max_tokens"], 77);
        assert_eq!(context, original);
    }
}
