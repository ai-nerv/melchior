use super::support::*;
use super::*;
use crate::mind::model::{Content, Message};
use serde_json::{Value, json};

const APIS: [&str; 2] = ["google-generative-ai", "google-vertex"];

fn calls(deltas: &[Delta]) -> Vec<String> {
    deltas
        .iter()
        .filter_map(|delta| match delta {
            Delta::ToolCallStart { id, .. } => Some(id.clone()),
            _ => None,
        })
        .collect()
}

fn event(parts: Value) -> sse::Event {
    sse::Event {
        name: String::new(),
        data: json!({"candidates":[{"content":{"parts":parts}}]}).to_string(),
    }
}

fn history() -> Context {
    let mut assistant = Message::assistant("");
    assistant.content = (1..=2)
        .map(|n| Content::ToolCall {
            id: format!("melchior-google-call-{n}"),
            name: "read".into(),
            arguments: json!({"path":n}),
            thought_signature: None,
        })
        .collect();
    let result: Message = serde_json::from_value(json!({
        "role":"tool", "content":[
            {"type":"tool_result","id":"melchior-google-call-1","name":"read","content":"first","is_error":false},
            {"type":"tool_result","id":"melchior-google-call-2","name":"read","content":"second","is_error":false}
        ]
    }))
    .expect("tool results");
    Context {
        messages: vec![Message::user("read both"), assistant, result],
        ..Context::default()
    }
}

#[test]
fn repeated_tool_names_preserve_distinct_provider_call_ids() {
    for api in APIS {
        let adapter = adapter(api);
        let mut state = StreamState::default();
        let mut deltas = Vec::new();
        for id in ["provider-1", "provider-2"] {
            deltas.extend(adapter.on_event(
                &mut state,
                &event(json!([{"functionCall":{"id":id,"name":"read","args":{"path":id}}}])),
            ));
        }
        assert_eq!(calls(&deltas), ["provider-1", "provider-2"], "{api}");
        let arguments: Vec<_> = deltas
            .iter()
            .filter_map(|delta| match delta {
                Delta::ToolCallArgs(raw) => Some(serde_json::from_str::<Value>(raw).expect("JSON")),
                _ => None,
            })
            .collect();
        assert_eq!(
            arguments,
            [json!({"path":"provider-1"}), json!({"path":"provider-2"})]
        );
    }
}

#[test]
fn streamed_arguments_preserve_null_empty_containers_and_unsigned_values() {
    for api in APIS {
        let adapter = adapter(api);
        for value in [
            Value::Null,
            json!(false),
            json!(0),
            json!([]),
            json!({}),
            json!({"value":[null,[],{},u64::MAX,i64::MIN]}),
        ] {
            let events = adapter.on_event(
                &mut StreamState::default(),
                &event(json!([{"functionCall":{"name":"echo","args":value}}])),
            );
            let arguments: Vec<_> = events
                .iter()
                .filter_map(|delta| match delta {
                    Delta::ToolCallArgs(raw) => {
                        Some(serde_json::from_str::<Value>(raw).expect("JSON"))
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(arguments, [value], "{api}");
        }
        let events = adapter.on_event(
            &mut StreamState::default(),
            &event(json!([{"functionCall":{"name":"echo"}}])),
        );
        assert!(events.contains(&Delta::ToolCallArgs("{}".into())), "{api}");
    }
}

#[test]
fn missing_empty_invalid_and_duplicate_ids_are_unique_across_frames() {
    for api in APIS {
        let adapter = adapter(api);
        let mut state = StreamState::default();
        let mut deltas = Vec::new();
        for id in [
            json!("melchior-google-call-1"),
            Value::Null,
            json!(""),
            json!(17),
            json!(true),
            json!("melchior-google-call-1"),
        ] {
            deltas.extend(adapter.on_event(
                &mut state,
                &event(json!([{"functionCall":{"id":id,"name":"read","args":{}}}])),
            ));
        }
        deltas.extend(adapter.on_event(
            &mut state,
            &event(json!([{"functionCall":{"name":"read","args":{}}}])),
        ));
        let ids = calls(&deltas);
        assert_eq!(ids.len(), 7, "{api}");
        assert_eq!(ids[0], "melchior-google-call-1", "{api}");
        assert!(ids.iter().all(|id| !id.is_empty()), "{api}");
        assert_eq!(
            ids.iter().collect::<std::collections::BTreeSet<_>>().len(),
            7,
            "{api}: {ids:?}"
        );
    }
}

#[test]
fn modern_google_models_replay_matching_call_and_result_ids() {
    for api in APIS {
        let adapter = adapter(api);
        for (id, included) in [
            ("gemini-2.5-pro", false),
            ("gemini-3-flash-preview", true),
            ("gemini-3.1-pro-preview", true),
            ("Gemini-4-argon", true),
            ("gemini-live-3-flash", true),
            ("claude-sonnet-4-6", true),
            ("gpt-oss-120b", true),
            ("m-1", false),
        ] {
            let mut model = plain_model();
            model.id = id.into();
            let body = adapter.request(&model, &history(), &Options::default());
            for index in 0..2 {
                let call = &body["contents"][1]["parts"][index]["functionCall"];
                let result = &body["contents"][2]["parts"][index]["functionResponse"];
                assert_eq!(call["name"], "read");
                assert_eq!(result["name"], "read");
                if included {
                    let expected = format!("melchior-google-call-{}", index + 1);
                    assert_eq!(call["id"], expected, "{api}/{id}");
                    assert_eq!(result["id"], expected, "{api}/{id}");
                } else {
                    assert!(call.get("id").is_none(), "{api}/{id}");
                    assert!(result.get("id").is_none(), "{api}/{id}");
                }
            }
        }
    }
}

#[test]
fn generated_ids_do_not_reuse_historical_call_or_result_ids() {
    for api in APIS {
        let adapter = adapter(api);
        let context = history();
        let body = adapter.request(&plain_model(), &context, &Options::default());
        assert_eq!(
            body,
            adapter.request(&plain_model(), &context, &Options::default())
        );
        let mut state = StreamState {
            scratch: body["__scratch"].clone(),
            ..StreamState::default()
        };
        let deltas = adapter.on_event(
            &mut state,
            &event(json!([{"functionCall":{"name":"read","args":{}}}])),
        );
        assert_eq!(calls(&deltas), ["melchior-google-call-3"], "{api}");
    }
}
