use super::support::{adapter, plain_model};
use crate::mind::model::{Context, Message};
use crate::mind::provider::{
    api::{Adapter, Options, Schema},
    compat::Compat,
};
use serde_json::{Value, json};

fn context(content: Value) -> Context {
    Context {
        system: Some("system".into()),
        messages: vec![
            Message::user("before"),
            serde_json::from_value(json!({"role":"assistant","content":content}))
                .expect("assistant"),
            Message::user("after"),
        ],
        ..Context::default()
    }
}

#[test]
fn required_empty_reasoning_is_scoped_to_declared_reasoning_models_and_assistants() {
    for required in [None, Some(false), Some(true)] {
        for reasoning in [false, true] {
            for content in [
                json!([]),
                json!([{"type":"text","text":"answer"}]),
                json!([{"type":"tool_call","id":"call","name":"lookup","arguments":{"v":[null,[],{},u64::MAX]}}]),
            ] {
                let mut model = plain_model();
                model.reasoning = reasoning;
                model.compat = Some(Compat {
                    requires_reasoning_content_on_assistant_messages: required,
                    ..Compat::default()
                });
                let ctx = context(content.clone());
                let before = serde_json::to_value(&ctx).expect("context");
                let body = adapter("openai-completions").request(&model, &ctx, &Options::default());
                let msg = &body["messages"][2];
                assert_eq!(
                    msg.get("reasoning_content"),
                    (reasoning && required == Some(true)).then_some(&json!(""))
                );
                for index in [0, 1, 3] {
                    assert!(body["messages"][index].get("reasoning_content").is_none());
                }
                if content[0]["type"] == "tool_call" {
                    let arguments: Value = serde_json::from_str(
                        msg["tool_calls"][0]["function"]["arguments"]
                            .as_str()
                            .expect("arguments"),
                    )
                    .expect("JSON");
                    assert_eq!(arguments, content[0]["arguments"]);
                }
                assert_eq!(serde_json::to_value(&ctx).expect("unchanged"), before);
            }
        }
    }
}

#[test]
fn required_field_preserves_reasoning_dialects_without_exposing_opaque_signatures() {
    for signature in [
        Value::Null,
        json!("reasoning_content"),
        json!("reasoning"),
        json!("reasoning_text"),
        json!("melchior.anthropic.v1:opaque-secret"),
    ] {
        for inline in [false, true] {
            let mut model = plain_model();
            model.compat = Some(Compat {
                requires_reasoning_content_on_assistant_messages: Some(true),
                requires_thinking_as_text: Some(inline),
                ..Compat::default()
            });
            let ctx = context(json!([
                {"type":"thinking","thinking":"first α","signature":signature},
                {"type":"thinking","thinking":"\nsecond"},
                {"type":"text","text":"answer"}
            ]));
            let body = adapter("openai-completions").request(&model, &ctx, &Options::default());
            let msg = &body["messages"][2];
            let field = match signature.as_str() {
                Some("reasoning") => "reasoning",
                Some("reasoning_text") => "reasoning_text",
                _ => "reasoning_content",
            };
            if inline {
                assert_eq!(msg["content"], "<thinking>first α\nsecond</thinking>answer");
                assert_eq!(msg["reasoning_content"], "");
                assert!(msg.get("reasoning").is_none());
                assert!(msg.get("reasoning_text").is_none());
            } else {
                assert_eq!(msg["content"], "answer");
                assert_eq!(msg[field], "first α\nsecond");
                if field != "reasoning_content" {
                    assert_eq!(msg["reasoning_content"], "");
                }
            }
            assert!(!body.to_string().contains("opaque-secret"));
        }
    }
}

#[test]
fn reasoning_replay_survives_later_user_turns_and_tool_results() {
    let mut model = plain_model();
    model.compat = Some(Compat {
        requires_reasoning_content_on_assistant_messages: Some(true),
        ..Compat::default()
    });
    let mut ctx = context(json!([
        {"type":"thinking","thinking":"retained","signature":"reasoning_content"},
        {"type":"tool_call","id":"call","name":"lookup","arguments":{}}
    ]));
    ctx.messages.insert(
        2,
        serde_json::from_value(json!({"role":"tool","content":[
            {"type":"tool_result","id":"call","name":"lookup","content":"result","is_error":false}
        ]}))
        .expect("result"),
    );
    ctx.messages
        .push(Message::assistant("answer without thinking"));
    ctx.messages.push(Message::user("another turn"));
    let body = adapter("openai-completions").request(&model, &ctx, &Options::default());
    assert_eq!(body["messages"][2]["reasoning_content"], "retained");
    assert_eq!(
        body["messages"][3],
        json!({"role":"tool","tool_call_id":"call","content":"result"})
    );
    assert_eq!(body["messages"][5]["reasoning_content"], "");
}

#[test]
fn compatibility_overrides_round_trip_and_explicit_false_wins() {
    let base = Compat {
        requires_reasoning_content_on_assistant_messages: Some(true),
        supports_json_schema: Some(false),
        supports_strict_mode: Some(true),
        ..Compat::default()
    };
    let encoded = serde_json::to_value(base).expect("encode");
    assert_eq!(
        serde_json::from_value::<Compat>(encoded).expect("decode"),
        base
    );
    assert_eq!(Compat::default().over(Some(base)), base);
    let overridden = Compat {
        requires_reasoning_content_on_assistant_messages: Some(false),
        supports_json_schema: Some(true),
        ..Compat::default()
    }
    .over(Some(base))
    .resolve();
    assert!(!overridden.requires_reasoning_content_on_assistant_messages);
    assert!(overridden.supports_json_schema);
    assert!(overridden.supports_strict_mode);
    assert!(
        !Compat::default()
            .resolve()
            .requires_reasoning_content_on_assistant_messages
    );
    assert!(Compat::default().resolve().supports_json_schema);
}

#[test]
fn structured_answer_refusal_is_explicit_and_independent_of_strict_function_tools() {
    let options = Options {
        schema: Some(Schema {
            name: "answer".into(),
            schema: json!({"type":"object","properties":{}}),
        }),
        ..Options::default()
    };
    for api in [
        "openai-completions",
        "openai-responses",
        "azure-openai-responses",
        "openai-codex-responses",
    ] {
        for supported in [None, Some(false), Some(true)] {
            let mut model = plain_model();
            model.compat = Some(Compat {
                supports_json_schema: supported,
                supports_strict_mode: Some(true),
                ..Compat::default()
            });
            let result = adapter(api).prepare(&model, &context(json!([])), &options);
            assert_eq!(
                result.is_ok(),
                supported != Some(false),
                "{api}: {result:?}"
            );
            if let Err(error) = result {
                assert!(error.contains("does not support JSON-schema structured answers"));
            }
        }
    }
}
