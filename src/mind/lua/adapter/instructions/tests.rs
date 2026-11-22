use super::*;
use crate::mind::lua::adapter::support::{adapter, plain_model};
use crate::mind::model::{Message, StopReason, Usage};
use crate::mind::provider::{
    api::{Adapter, Options},
    compat::Compat,
};
use serde_json::json;

fn history() -> Context {
    Context {
        system: Some("base".into()),
        messages: vec![
            Message::user("first"),
            Message::assistant("answer"),
            Message::system("new α"),
            Message::user("later"),
            Message::system("second"),
        ],
        ..Context::default()
    }
}

#[test]
fn instruction_role_round_trips_and_has_no_assistant_metadata() {
    let message = Message::system("α\nline");
    let encoded = serde_json::to_value(&message).expect("encode");
    assert_eq!(
        encoded,
        json!({"role":"system","content":[{"type":"text","text":"α\nline"}]})
    );
    assert_eq!(
        serde_json::from_value::<Message>(encoded).expect("decode"),
        message
    );
}

#[test]
fn opted_in_chat_preserves_exact_instruction_positions_and_declared_role() {
    for developer in [false, true] {
        let mut model = plain_model();
        model.compat = Some(Compat {
            supports_mid_convo_system_messages: Some(true),
            supports_developer_role: Some(developer),
            ..Compat::default()
        });
        let ctx = history();
        let saved = ctx.clone();
        let body = adapter("openai-completions")
            .prepare(&model, &ctx, &Options::default())
            .expect("body")
            .0;
        let role = if developer { "developer" } else { "system" };
        assert_eq!(
            body["messages"],
            json!([
                {"role":role,"content":"base"},{"role":"user","content":"first"},
                {"role":"assistant","content":"answer"},{"role":role,"content":"new α"},
                {"role":"user","content":"later"},{"role":role,"content":"second"}
            ])
        );
        assert_eq!(ctx, saved);
    }
}

#[test]
fn default_and_false_fold_instruction_text_in_order_without_mutating_history() {
    for api in [
        "openai-completions",
        "openai-responses",
        "anthropic-messages",
        "google-generative-ai",
        "bedrock-converse-stream",
        "decisions",
    ] {
        for flag in [None, Some(false)] {
            let mut model = plain_model();
            model.compat = Some(Compat {
                supports_mid_convo_system_messages: flag,
                ..Compat::default()
            });
            let mut ctx = history();
            ctx.messages.insert(0, Message::system("before"));
            ctx.messages.insert(2, Message::system(""));
            ctx.messages[4].content.push(Content::Text {
                text: " suffix".into(),
                signature: None,
            });
            let saved = ctx.clone();
            let folded = context(api, &model, &ctx).expect("fold");
            assert_eq!(
                folded.system.as_deref(),
                Some("base\n\nbefore\n\nnew α suffix\n\nsecond")
            );
            assert_eq!(
                folded.messages,
                vec![
                    Message::user("first"),
                    Message::assistant("answer"),
                    Message::user("later")
                ]
            );
            assert_eq!(ctx, saved);
        }
    }
}

#[test]
fn unsupported_positional_adapter_and_nontext_instruction_messages_fail_before_encoding() {
    let mut model = plain_model();
    model.compat = Some(Compat {
        supports_mid_convo_system_messages: Some(true),
        ..Compat::default()
    });
    for api in [
        "anthropic-messages",
        "openai-responses",
        "google-generative-ai",
        "bedrock-converse-stream",
        "custom",
    ] {
        assert!(
            context(api, &model, &history())
                .expect_err("unsupported")
                .contains("cannot preserve")
        );
        assert!(context(api, &model, &Context::default()).is_ok());
    }
    for bad in [
        json!({"type":"image","media_type":"image/png","data":"abcd"}),
        json!({"type":"thinking","thinking":"hidden"}),
        json!({"type":"text","text":"signed","signature":"opaque"}),
        json!({"type":"tool_call","id":"a","name":"echo","arguments":{}}),
        json!({"type":"tool_result","id":"a","name":"echo","content":"result","is_error":false}),
    ] {
        let mut ctx = history();
        ctx.messages[2].content = vec![serde_json::from_value(bad).expect("block")];
        for flag in [true, false] {
            model
                .compat
                .as_mut()
                .expect("compat")
                .supports_mid_convo_system_messages = Some(flag);
            assert!(
                adapter("openai-completions")
                    .prepare(&model, &ctx, &Options::default())
                    .expect_err("invalid")
                    .contains("unsigned text")
            );
        }
    }
    for which in 0..3 {
        let mut ctx = history();
        match which {
            0 => ctx.messages[2].usage = Some(Usage::default()),
            1 => ctx.messages[2].stop_reason = Some(StopReason::EndTurn),
            _ => ctx.messages[2].error = Some("error".into()),
        }
        assert!(context("openai-completions", &model, &ctx).is_err());
    }
}

#[test]
fn unchanged_and_empty_instructions_keep_legacy_context_and_override_precedence() {
    let mut model = plain_model();
    for system in [None, Some(String::new()), Some("base".into())] {
        let ctx = Context {
            system,
            messages: vec![Message::user("hi")],
            ..Context::default()
        };
        assert_eq!(
            context("openai-completions", &model, &ctx).expect("unchanged"),
            ctx
        );
        let mut empty = ctx.clone();
        empty.messages.insert(0, Message::system(""));
        assert_eq!(
            context("openai-completions", &model, &empty).expect("empty"),
            ctx
        );
    }
    let enabled = Compat {
        supports_mid_convo_system_messages: Some(true),
        ..Compat::default()
    };
    assert!(
        Compat::default()
            .over(Some(enabled))
            .resolve()
            .supports_mid_convo_system_messages
    );
    let disabled = Compat {
        supports_mid_convo_system_messages: Some(false),
        ..Compat::default()
    };
    model.compat = Some(enabled);
    let wire = adapter("openai-completions");
    let before = wire
        .prepare(&model, &history(), &Options::default())
        .expect("before")
        .1
        .expect("frame");
    model.compat = Some(disabled.over(Some(enabled)));
    assert!(
        !model
            .compat
            .expect("compat")
            .resolve()
            .supports_mid_convo_system_messages
    );
    let after = wire
        .prepare(&model, &history(), &Options::default())
        .expect("after")
        .1
        .expect("frame");
    assert_ne!(before.policy, after.policy);
    assert_eq!(before.dialect, after.dialect);
}

#[test]
fn every_registered_api_serializes_folded_history_as_the_same_explicit_leading_prompt() {
    let apis = crate::mind::lua::adapter::engine_with_builtins()
        .expect("builtins")
        .apis();
    assert_eq!(
        apis,
        [
            "anthropic-messages",
            "azure-openai-responses",
            "decisions",
            "google-generative-ai",
            "google-vertex",
            "mistral-conversations",
            "openai-codex-responses",
            "openai-completions",
            "openai-responses",
            "pi-messages",
        ]
    );
    for name in apis {
        let mut model = plain_model();
        model.api = serde_json::from_value(json!(name)).expect("known API");
        let ctx = history();
        let saved = ctx.clone();
        let explicit = Context {
            system: Some("base\n\nnew α\n\nsecond".into()),
            messages: vec![
                Message::user("first"),
                Message::assistant("answer"),
                Message::user("later"),
            ],
            ..Context::default()
        };
        let wire = adapter(&name);
        let expected = wire
            .prepare(&model, &explicit, &Options::default())
            .expect("explicit prompt");
        let actual = wire
            .prepare(&model, &ctx, &Options::default())
            .expect("folded prompt");
        assert_eq!(actual, expected, "{name}");
        let mut values = vec![&actual.0];
        let mut retained = false;
        while let Some(value) = values.pop() {
            match value {
                serde_json::Value::String(text) => {
                    retained |= text == "base\n\nnew α\n\nsecond";
                }
                serde_json::Value::Array(items) => values.extend(items),
                serde_json::Value::Object(fields) => values.extend(fields.values()),
                _ => {}
            }
        }
        assert!(retained, "{name} must transmit every instruction");
        assert_eq!(
            wire.request_endpoint("https://fixture.invalid/v1", &model, &actual.0)
                .expect("prepared endpoint"),
            wire.endpoint("https://fixture.invalid/v1", &model),
            "{name} must retain its ordinary endpoint"
        );
        assert_eq!(ctx, saved);
    }
}
