use super::support::{adapter, plain_context, plain_model};
use crate::mind::model::{Content, Context, Message, StopReason, ThinkingLevel};
use crate::mind::provider::api::{Adapter, Delta, Options, StreamState};
use crate::mind::provider::model::{Api, Model};
use crate::mind::provider::sse::Event;
use serde_json::{Value, json};

mod costs;
mod streaming;

const APIS: [&str; 3] = [
    "openai-responses",
    "azure-openai-responses",
    "openai-codex-responses",
];

fn bound_options() -> Options {
    Options {
        continuation_binding: Some("fixture-principal".into()),
        ..Default::default()
    }
}

fn model(api: &str) -> Model {
    let mut model = plain_model();
    model.api = serde_json::from_value(json!(api)).expect("API");
    model.base_url = Some("https://fixture.invalid/v1".into());
    model
}

fn items() -> Value {
    json!([
        {"type":"reasoning","id":"rs_a","summary":[],"encrypted_content":"opaque\u{1} +/= α"},
        {"type":"message","id":"msg_a","role":"assistant","phase":"commentary",
         "content":[{"type":"output_text","text":"Checking","annotations":[]}]},
        {"type":"reasoning","id":"rs_b","summary":[{"type":"summary_text","text":"private"}],
         "encrypted_content":"second opaque"},
        {"type":"function_call","id":"fc_a","call_id":"call_a","name":"lookup",
         "arguments":"{\"data\":[null,[],{},18446744073709551615]}"}
    ])
}

fn completed(api: &str, output: Value) -> (Vec<Delta>, StreamState) {
    let wire = adapter(api);
    let request = wire.request(&model(api), &plain_context(), &bound_options());
    let mut state = StreamState {
        scratch: request["__scratch"].clone(),
        ..Default::default()
    };
    let deltas = wire.on_event(
        &mut state,
        &Event {
            name: "response.completed".into(),
            data: json!({"type":"response.completed","response":{"output":output,
                "usage":{"input_tokens":100,"output_tokens":20,"input_tokens_details":{"cached_tokens":40}}}})
                .to_string(),
        },
    );
    (deltas, state)
}

fn signature(api: &str) -> String {
    completed(api, items())
        .0
        .into_iter()
        .find_map(|delta| match delta {
            Delta::Signature(value) => Some(value),
            _ => None,
        })
        .expect("terminal continuation")
}

fn context(signature: String) -> Context {
    let mut message = Message::assistant("Checking");
    message.content.insert(
        0,
        Content::Thinking {
            thinking: String::new(),
            signature: Some(signature),
        },
    );
    message.content.push(Content::ToolCall {
        id: "call_a".into(),
        name: "lookup".into(),
        arguments: json!({"data":[null,[],{},u64::MAX]}),
        thought_signature: None,
    });
    Context {
        messages: vec![message],
        ..Default::default()
    }
}

#[test]
fn stateless_requests_include_encryption_even_without_explicit_effort() {
    for api in APIS {
        let wire = adapter(api);
        let mut model = model(api);
        for thinking in [None, Some(ThinkingLevel::Off), Some(ThinkingLevel::High)] {
            let body = wire.request(
                &model,
                &plain_context(),
                &Options {
                    thinking,
                    ..Default::default()
                },
            );
            assert_eq!(body["store"], false);
            assert_eq!(body["include"], json!(["reasoning.encrypted_content"]));
        }
        model.reasoning = false;
        let body = wire.request(&model, &plain_context(), &Options::default());
        assert_eq!(body["store"], false);
        assert!(body.get("include").is_none());
        model.reasoning = true;
        model
            .thinking
            .insert(ThinkingLevel::Off, Some("none".into()));
        let body = wire.request(
            &model,
            &plain_context(),
            &Options {
                thinking: Some(ThinkingLevel::Off),
                ..Default::default()
            },
        );
        assert_eq!(body["reasoning"]["effort"], "none");
    }
}

#[test]
fn terminal_state_replays_all_items_in_order_without_touching_history() {
    for api in APIS {
        let context = context(signature(api));
        let before = context.clone();
        let request = adapter(api).request(&model(api), &context, &bound_options());
        assert_eq!(request["input"], items(), "{api}");
        assert_eq!(context, before);
        let (deltas, mut state) = completed(api, items());
        assert_eq!(deltas.last(), Some(&Delta::Stop(StopReason::ToolUse)));
        assert_eq!(state.usage.input, 60);
        assert_eq!(state.usage.cache_read, 40);
        assert_eq!(state.usage.output, 20);
        assert!(
            adapter(api)
                .on_event(
                    &mut state,
                    &Event {
                        name: String::new(),
                        data: json!({"type":"response.completed","response":{}}).to_string()
                    }
                )
                .is_empty()
        );
    }
}

#[test]
fn replay_requires_an_explicit_matching_internal_credential_binding() {
    let original = context(signature(APIS[0]));
    for binding in [None, Some("different".into())] {
        let options = Options {
            continuation_binding: binding,
            ..Default::default()
        };
        let body = adapter(APIS[0]).request(&model(APIS[0]), &original, &options);
        assert_eq!(body["input"].as_array().expect("input").len(), 2);
        assert!(!body.to_string().contains("encrypted_content\":"));
        if options.continuation_binding.is_none() {
            assert!(body.get("__scratch").is_none());
        }
    }
}

#[test]
fn state_never_crosses_provider_model_protocol_or_endpoint() {
    let original = context(signature(APIS[0]));
    for mutation in 0..4 {
        let mut model = model(APIS[0]);
        let api = if mutation == 2 { APIS[1] } else { APIS[0] };
        match mutation {
            0 => model.provider = "other".into(),
            1 => model.id = "other".into(),
            2 => model.api = Api::AzureOpenAiResponses,
            _ => model.base_url = Some("https://other.invalid/v1".into()),
        }
        let request = adapter(api).request(&model, &original, &bound_options());
        assert_eq!(request["input"].as_array().expect("input").len(), 2);
        assert!(!request.to_string().contains("encrypted_content\":"));
        assert_eq!(request["input"][1]["call_id"], "call_a");
    }
}

#[test]
fn responses_state_is_not_forwarded_as_another_protocols_signature() {
    let mut context = context(signature(APIS[0]));
    let Content::Thinking { thinking, .. } = &mut context.messages[0].content[0] else {
        panic!("thinking block");
    };
    *thinking = "visible reasoning".into();
    for api in [
        "anthropic-messages",
        "google-generative-ai",
        "google-vertex",
        "pi-messages",
        "openai-completions",
    ] {
        let body = adapter(api).request(&plain_model(), &context, &Options::default());
        assert!(body.is_object(), "{api}");
        let encoded = body.to_string();
        assert!(!encoded.contains("melchior.responses.v1:"), "{api}");
        assert!(!encoded.contains("second opaque"), "{api}");
        assert!(encoded.contains("Checking"), "{api}");
        assert!(encoded.contains("lookup"), "{api}");
    }
}

#[test]
fn malformed_foreign_or_edited_state_cannot_override_visible_history() {
    let original = signature(APIS[0]);
    for bad in [
        "opaque".into(),
        "melchior.responses.v1:[]".into(),
        "melchior.responses.v1:{".into(),
        format!("{original}{}", "x".repeat(1048576)),
    ] {
        let request = adapter(APIS[0]).request(&model(APIS[0]), &context(bad), &bound_options());
        assert_eq!(request["input"].as_array().expect("input").len(), 2);
    }
    let mut context = context(original);
    context.messages[0].content[1] = Content::Text {
        text: "edited".into(),
        signature: None,
    };
    let request = adapter(APIS[0]).request(&model(APIS[0]), &context, &bound_options());
    assert_eq!(request["input"][0]["content"][0]["text"], "edited");
    assert_eq!(request["input"].as_array().expect("input").len(), 2);
}

#[test]
fn replay_compares_object_data_without_normalizing_original_argument_bytes() {
    let signature = signature(APIS[0]);
    let prefix = "melchior.responses.v1:";
    let mut saved: Value =
        serde_json::from_str(signature.strip_prefix(prefix).expect("prefix")).expect("saved JSON");
    let arguments = "{ \"z\" : false, \"a\" : [] }";
    saved["items"][3]["arguments"] = arguments.into();
    let mut context = context(format!("{prefix}{saved}"));
    let Content::ToolCall {
        arguments: actual, ..
    } = &mut context.messages[0].content[2]
    else {
        panic!("call");
    };
    *actual = json!({"a":[], "z":false});
    let wire = adapter(APIS[0]);
    let request = wire.request(&model(APIS[0]), &context, &bound_options());
    assert_eq!(request["input"], saved["items"]);
    let Content::ToolCall {
        arguments: actual, ..
    } = &mut context.messages[0].content[2]
    else {
        panic!("call");
    };
    *actual = json!({"a":{}, "z":false});
    let request = wire.request(&model(APIS[0]), &context, &bound_options());
    assert_eq!(request["input"].as_array().expect("input").len(), 2);
}

#[test]
fn oversized_continuations_fail_without_losing_measured_usage() {
    for output in [
        json!([{"type":"reasoning","encrypted_content":"x".repeat(1048576)}]),
        Value::Array(vec![
            json!({"type":"reasoning","encrypted_content":"x"});
            257
        ]),
    ] {
        let (deltas, state) = completed(APIS[0], output);
        assert!(
            !deltas
                .iter()
                .any(|delta| matches!(delta, Delta::Signature(_)))
        );
        assert_eq!(deltas.last(), Some(&Delta::Stop(StopReason::Error)));
        assert_eq!(state.usage.output, 20);
    }
}
