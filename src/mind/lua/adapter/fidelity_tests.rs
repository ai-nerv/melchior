use super::support::{adapter, plain_model};
use crate::mind::model::{Content, Context, Message, Tool};
use crate::mind::provider::api::{Adapter, Options, Schema};
use serde_json::{Value, json};

const PROTOCOLS: [&str; 8] = [
    "openai-completions",
    "openai-responses",
    "azure-openai-responses",
    "openai-codex-responses",
    "anthropic-messages",
    "google-generative-ai",
    "google-vertex",
    "pi-messages",
];

fn argument(body: &Value, protocol: &str) -> Value {
    let encoded = match protocol {
        "openai-completions" => &body["messages"][1]["tool_calls"][0]["function"]["arguments"],
        "openai-responses" | "azure-openai-responses" | "openai-codex-responses" => {
            &body["input"][1]["arguments"]
        }
        "anthropic-messages" => return body["messages"][1]["content"][0]["input"].clone(),
        "google-generative-ai" | "google-vertex" => {
            return body["contents"][1]["parts"][0]["functionCall"]["args"].clone();
        }
        "pi-messages" => return body["messages"][1]["content"][0]["arguments"].clone(),
        _ => panic!("unknown protocol {protocol}"),
    };
    serde_json::from_str(encoded.as_str().expect("encoded arguments")).expect("argument JSON")
}

fn parameters<'a>(body: &'a Value, protocol: &str) -> &'a Value {
    match protocol {
        "openai-completions" => &body["tools"][0]["function"]["parameters"],
        "anthropic-messages" => &body["tools"][0]["input_schema"],
        "google-generative-ai" | "google-vertex" => {
            &body["tools"][0]["functionDeclarations"][0]["parameters"]
        }
        _ => &body["tools"][0]["parameters"],
    }
}

fn schema<'a>(body: &'a Value, protocol: &str) -> &'a Value {
    match protocol {
        "openai-completions" => &body["response_format"]["json_schema"]["schema"],
        "anthropic-messages" => &body["tools"][0]["input_schema"],
        "google-generative-ai" | "google-vertex" => &body["generationConfig"]["responseSchema"],
        _ => &body["text"]["format"]["schema"],
    }
}

#[test]
fn actual_protocol_requests_preserve_all_json_argument_forms_and_schema_data() {
    let parameters = json!({"type":"object","properties":{"v":{"default":[null,[],{}]}},
        "required":[],"cache_control":{"type":"literal"}});
    for protocol in PROTOCOLS {
        let wire = adapter(protocol);
        for arguments in [
            json!(null),
            json!(false),
            json!(0),
            json!(u64::MAX),
            json!(i64::MIN),
            json!(0.125),
            json!(""),
            json!([]),
            json!({}),
            json!({"v":[null,[],{}],"cache_control":{"type":"literal"}}),
        ] {
            let mut assistant = Message::assistant("");
            assistant.content = vec![Content::ToolCall {
                id: "same-call".into(),
                name: "lookup".into(),
                arguments: arguments.clone(),
                thought_signature: None,
            }];
            let context = Context {
                system: None,
                messages: vec![Message::user("call"), assistant],
                tools: vec![Tool {
                    name: "lookup".into(),
                    description: "Lookup".into(),
                    parameters: parameters.clone(),
                }],
            };
            let body = wire.request(&plain_model(), &context, &Options::default());
            assert_eq!(argument(&body, protocol), arguments, "{protocol}");
            assert_eq!(self::parameters(&body, protocol), &parameters, "{protocol}");
        }
    }
}

#[test]
fn actual_forced_schemas_preserve_empty_arrays_objects_nulls_and_marker_names() {
    let value = json!({"type":"object","properties":{},"required":[],"default":null,
        "cache_control":{"type":"literal"}});
    for protocol in PROTOCOLS.into_iter().filter(|name| *name != "pi-messages") {
        let body = adapter(protocol).request(
            &plain_model(),
            &Context {
                messages: vec![Message::user("answer")],
                ..Context::default()
            },
            &Options {
                schema: Some(Schema {
                    name: "answer".into(),
                    schema: value.clone(),
                }),
                ..Options::default()
            },
        );
        assert_eq!(schema(&body, protocol), &value, "{protocol}");
    }
}

#[test]
fn generated_empty_conversation_fields_are_arrays_not_objects() {
    for protocol in PROTOCOLS {
        let body =
            adapter(protocol).request(&plain_model(), &Context::default(), &Options::default());
        let key = match protocol {
            "openai-responses" | "azure-openai-responses" | "openai-codex-responses" => "input",
            "google-generative-ai" | "google-vertex" => "contents",
            _ => "messages",
        };
        assert_eq!(body[key], json!([]), "{protocol}");
        assert!(
            body.get("tools").is_none(),
            "no phantom tools for {protocol}"
        );
    }
}
