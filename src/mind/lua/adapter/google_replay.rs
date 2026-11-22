use super::{support::*, *};
use crate::mind::model::{Content, Message, Role, StopReason};
use crate::mind::provider::model::Api;
use serde_json::{Value, json};

const PREFIX: &str = "melchior.google.v1:";
const APIS: [(&str, Api); 2] = [
    ("google-generative-ai", Api::GoogleGenerativeAi),
    ("google-vertex", Api::GoogleVertex),
];

fn model(api: Api) -> Model {
    let mut model = plain_model();
    model.api = api;
    model.id = "gemini-3-flash-preview".into();
    model.base_url = Some("https://fixture.invalid/v1".into());
    model
}

fn options() -> Options {
    Options {
        continuation_binding: Some("credential-lineage-a".into()),
        ..Default::default()
    }
}

fn begin(api: &LuaAdapter, model: &Model) -> StreamState {
    let (body, _) = api
        .prepare(model, &plain_context(), &options())
        .expect("request");
    StreamState {
        scratch: body["__scratch"].clone(),
        ..Default::default()
    }
}

fn event(api: &LuaAdapter, state: &mut StreamState, value: Value) -> Vec<Delta> {
    api.on_event(
        state,
        &sse::Event {
            name: String::new(),
            data: value.to_string(),
        },
    )
}

fn parts() -> Vec<Value> {
    vec![
        json!({"text":"thought α", "thought":true, "thoughtSignature":"dGhvdWdodA=="}),
        json!({"text":"first", "thoughtSignature":"dGV4dA=="}),
        json!({"text":"second"}),
        json!({"text":"", "thoughtSignature":"ZW1wdHk="}),
        json!({"functionCall":{"id":"a","name":"echo","args":{"v":[null,[],{}]}},"thoughtSignature":"Y2FsbGE="}),
        json!({"text":"between", "thought":true, "thoughtSignature":"YmV0d2Vlbg=="}),
        json!({"functionCall":{"id":"b","name":"echo","args":{"v":u64::MAX}},"thoughtSignature":"Y2FsbGI="}),
    ]
}

fn stream(api: &LuaAdapter, model: &Model, parts: &[Value], reason: &str) -> Vec<Delta> {
    let mut state = begin(api, model);
    let mut deltas = Vec::new();
    for part in parts {
        let next = event(
            api,
            &mut state,
            json!({"candidates":[{"content":{"parts":[part]}}]}),
        );
        assert!(
            !next
                .iter()
                .any(|delta| matches!(delta, Delta::Signature(_)))
        );
        deltas.extend(next);
    }
    deltas.extend(event(
        api,
        &mut state,
        json!({"candidates":[{"finishReason":reason}]}),
    ));
    deltas
}

fn neutral(deltas: &[Delta]) -> Message {
    let (mut thinking, mut text, mut signature) = (String::new(), String::new(), None);
    let mut calls = Vec::<(String, String, String)>::new();
    for delta in deltas {
        match delta {
            Delta::Thinking(value) => thinking.push_str(value),
            Delta::Text(value) => text.push_str(value),
            Delta::Signature(value) => signature = Some(value.clone()),
            Delta::ToolCallStart { id, name } => {
                calls.push((id.clone(), name.clone(), String::new()))
            }
            Delta::ToolCallArgs(value) => calls.last_mut().expect("call").2.push_str(value),
            _ => {}
        }
    }
    let mut content = vec![
        Content::Thinking {
            thinking,
            signature,
        },
        Content::Text {
            text,
            signature: None,
        },
    ];
    content.extend(
        calls
            .into_iter()
            .map(|(id, name, arguments)| Content::ToolCall {
                id,
                name,
                arguments: serde_json::from_str(&arguments).expect("arguments"),
                thought_signature: None,
            }),
    );
    Message {
        role: Role::Assistant,
        content,
        stop_reason: Some(StopReason::ToolUse),
        usage: None,
        error: None,
    }
}

fn body(api: &LuaAdapter, model: &Model, message: Message, options: &Options) -> Value {
    api.prepare(
        model,
        &Context {
            messages: vec![message],
            ..Context::default()
        },
        options,
    )
    .expect("prepare")
    .0
}

#[test]
fn signed_parts_keep_their_exact_order_and_original_carriers() {
    for (name, kind) in APIS {
        let api = adapter(name);
        let model = model(kind);
        let parts = parts();
        let deltas = stream(&api, &model, &parts, "STOP");
        assert_eq!(
            deltas
                .iter()
                .filter(|d| matches!(d, Delta::Signature(_)))
                .count(),
            1
        );
        assert_eq!(deltas.last(), Some(&Delta::Stop(StopReason::ToolUse)));
        let message = neutral(&deltas);
        assert!(
            serde_json::to_string(&message)
                .expect("JSON")
                .contains(PREFIX)
        );
        let body = body(&api, &model, message, &options());
        assert_eq!(body["contents"][0]["parts"], json!(parts), "{name}");
    }
}

#[test]
fn empty_text_empty_thought_and_signature_only_parts_survive() {
    for (name, kind) in APIS {
        let api = adapter(name);
        let model = model(kind);
        for part in [
            json!({"text":"","thoughtSignature":"ZW1wdHk="}),
            json!({"text":"","thought":true,"thoughtSignature":"ZW1wdHk="}),
            json!({"thoughtSignature":"ZW1wdHk="}),
        ] {
            let deltas = stream(&api, &model, std::slice::from_ref(&part), "STOP");
            assert_eq!(deltas.last(), Some(&Delta::Stop(StopReason::EndTurn)));
            let body = body(&api, &model, neutral(&deltas), &options());
            assert_eq!(body["contents"][0]["parts"], json!([part]), "{name}");
        }
    }
}

#[test]
fn origin_changes_and_edited_visible_history_do_not_replay_private_parts() {
    for (name, kind) in APIS {
        let api = adapter(name);
        let model = model(kind);
        let message = neutral(&stream(&api, &model, &parts(), "STOP"));
        for change in 0..10 {
            let (mut target, mut context, mut options) =
                (model.clone(), message.clone(), options());
            match change {
                0 => target.provider = "other".into(),
                1 => target.id = "gemini-3-pro-preview".into(),
                2 => target.base_url = Some("https://other.invalid/v1".into()),
                3 => options.continuation_binding = Some("credential-lineage-b".into()),
                4 => options.continuation_binding = None,
                5 => {
                    if let Content::Thinking { thinking, .. } = &mut context.content[0] {
                        thinking.push('!');
                    }
                }
                6 => {
                    if let Content::Text { text, .. } = &mut context.content[1] {
                        text.push('!');
                    }
                }
                7 => {
                    if let Content::ToolCall { arguments, .. } = &mut context.content[2] {
                        *arguments = json!({"changed":true});
                    }
                }
                8 => {
                    if let Content::ToolCall { id, .. } = &mut context.content[2] {
                        *id = "changed".into();
                    }
                }
                _ => {
                    if let Content::Thinking { signature, .. } = &mut context.content[0] {
                        *signature = Some("aW1wb3J0ZWQ=".into());
                    }
                    if let Content::Text { signature, .. } = &mut context.content[1] {
                        *signature = Some("aW1wb3J0ZWQ=".into());
                    }
                    if let Content::ToolCall {
                        thought_signature, ..
                    } = &mut context.content[2]
                    {
                        *thought_signature = Some("aW1wb3J0ZWQ=".into());
                    }
                }
            }
            let body = body(&api, &target, context, &options);
            assert!(
                !body.to_string().contains("thoughtSignature"),
                "{name} change {change}: {body}"
            );
            assert!(!body.to_string().contains(PREFIX));
            assert!(body.to_string().contains("firstsecond"));
            assert!(body.to_string().contains("functionCall"));
        }
    }
}

#[test]
fn opaque_google_state_never_crosses_into_another_protocol() {
    let google = adapter("google-generative-ai");
    let model = model(Api::GoogleGenerativeAi);
    let message = neutral(&stream(&google, &model, &parts(), "STOP"));
    for name in [
        "openai-completions",
        "openai-responses",
        "anthropic-messages",
        "pi-messages",
        "google-vertex",
    ] {
        let body = body(&adapter(name), &model, message.clone(), &options());
        assert!(!body.to_string().contains(PREFIX), "{name}");
        assert!(!body.to_string().contains("dGhvdWdodA=="), "{name}");
        assert!(!body.to_string().contains("thoughtSignature"), "{name}");
    }
}

#[test]
fn unsuccessful_or_malformed_streams_never_publish_replay_state() {
    for (name, kind) in APIS {
        let api = adapter(name);
        let model = model(kind);
        for reason in [
            "MAX_TOKENS",
            "SAFETY",
            "RECITATION",
            "MALFORMED_FUNCTION_CALL",
            "NEW_FAILURE",
        ] {
            let deltas = stream(&api, &model, &parts(), reason);
            assert!(
                !deltas.iter().any(|d| matches!(d, Delta::Signature(_))),
                "{name}/{reason}"
            );
            let expected = if reason == "MAX_TOKENS" {
                StopReason::Length
            } else {
                StopReason::Error
            };
            assert_eq!(deltas.last(), Some(&Delta::Stop(expected)));
        }
        for part in [
            json!({"text":"bad","thoughtSignature":"invalid"}),
            json!({"inlineData":{"data":"YQ=="}}),
            json!({"functionCall":{"name":"","args":{}}}),
        ] {
            let deltas = stream(&api, &model, &[part], "STOP");
            assert!(!deltas.iter().any(|d| matches!(d, Delta::Signature(_))));
            assert!(
                deltas.contains(&Delta::Stop(StopReason::Error)),
                "{deltas:?}"
            );
        }
    }
}

#[test]
fn bounded_state_refuses_oversize_without_truncating_a_signature() {
    for (name, kind) in APIS {
        let api = adapter(name);
        let model = model(kind);
        for parts in [
            vec![json!({"text":"x".repeat(1048576),"thoughtSignature":"YQ=="})],
            vec![json!({"text":"","thoughtSignature":"YQ=="}); 8193],
        ] {
            let mut state = begin(&api, &model);
            let deltas = event(
                &api,
                &mut state,
                json!({"candidates":[{"content":{"parts":parts},"finishReason":"STOP"}]}),
            );
            assert!(!deltas.iter().any(|d| matches!(d, Delta::Signature(_))));
            assert!(deltas.contains(&Delta::Stop(StopReason::Error)), "{name}");
            assert!(
                state.scratch.to_string().len() < 4096,
                "oversize state retained"
            );
        }
    }
}

#[test]
fn terminal_state_is_emitted_once_and_late_usage_is_still_accounted() {
    for (name, kind) in APIS {
        let api = adapter(name);
        let model = model(kind);
        let mut state = begin(&api, &model);
        let first = event(
            &api,
            &mut state,
            json!({"candidates":[{"content":{"parts":parts()},"finishReason":"STOP"}]}),
        );
        assert_eq!(
            first
                .iter()
                .filter(|d| matches!(d, Delta::Signature(_)))
                .count(),
            1
        );
        let next = event(
            &api,
            &mut state,
            json!({"usageMetadata":{"promptTokenCount":100,"cachedContentTokenCount":40,"candidatesTokenCount":7}}),
        );
        assert!(
            matches!(next.as_slice(), [Delta::Usage(u)] if u.input == 60 && u.cache_read == 40 && u.output == 7)
        );
        assert!(
            event(
                &api,
                &mut state,
                json!({"candidates":[{"finishReason":"STOP"}]})
            )
            .is_empty()
        );
    }
}
