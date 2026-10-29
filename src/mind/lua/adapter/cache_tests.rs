use super::support::{adapter, plain_model};
use crate::mind::model::{Content, Context, Image, Message, Role, Tool};
use crate::mind::provider::api::{Adapter, Options, Schema};
use crate::mind::provider::model::Modality;
use serde_json::{Value, json};

fn request(context: &Context, options: &Options) -> Value {
    let mut model = plain_model();
    model.input.push(Modality::Image);
    adapter("anthropic-messages").request(&model, context, options)
}

fn tool(parameters: Value) -> Tool {
    Tool {
        name: "lookup".into(),
        description: "Find a value".into(),
        parameters,
    }
}

#[test]
fn capability_off_keeps_the_actual_anthropic_request_golden() {
    let context = Context {
        system: Some("Static system".into()),
        messages: vec![Message::user("hi")],
        tools: vec![tool(
            json!({"type":"object","properties":{"key":{"type":"string"}},"required":["key"]}),
        )],
    };
    let expected = json!({
        "model":"m-1","stream":true,"max_tokens":8192,"system":"Static system",
        "messages":[{"role":"user","content":[{"type":"text","text":"hi"}]}],
        "tools":[{"name":"lookup","description":"Find a value",
            "input_schema":{"type":"object","properties":{"key":{"type":"string"}},"required":["key"]}}]
    });
    assert_eq!(request(&context, &Options::default()), expected);
}

#[test]
fn capability_off_preserves_signed_thinking_tool_units_and_opaque_image_payloads() {
    let args = json!({"query":" whitespace\r\n","values":[1,true,"cache_control"],
        "object":{"cache_control":"literal"}});
    let mut user = Message::user("inspect");
    user.content.push(Content::Image {
        data: "user-image-bytes".into(),
        media_type: "image/png".into(),
    });
    let mut assistant = Message::assistant("");
    assistant.content = vec![
        Content::Thinking {
            thinking: "reasoning bytes".into(),
            signature: Some("opaque-signature".into()),
        },
        Content::Text {
            text: "answer".into(),
            signature: None,
        },
        Content::ToolCall {
            id: "provider-call-1".into(),
            name: "lookup".into(),
            arguments: args.clone(),
            thought_signature: None,
        },
    ];
    let context = Context {
        system: None,
        tools: Vec::new(),
        messages: vec![
            user,
            assistant,
            Message {
                role: Role::Tool,
                content: vec![Content::ToolResult {
                    id: "provider-call-1".into(),
                    name: "lookup".into(),
                    content: "tool result\n".into(),
                    is_error: false,
                    images: vec![Image {
                        data: "tool-image-bytes".into(),
                        media_type: "image/png".into(),
                    }],
                }],
                stop_reason: None,
                usage: None,
                error: None,
            },
        ],
    };
    let body = request(&context, &Options::default());
    assert_eq!(
        body,
        json!({"model":"m-1","stream":true,"max_tokens":8192,"messages":[
            {"role":"user","content":[{"type":"text","text":"inspect"},
                {"type":"image","source":{"type":"base64","media_type":"image/png","data":"user-image-bytes"}}]},
            {"role":"assistant","content":[{"type":"thinking","thinking":"reasoning bytes","signature":"opaque-signature"},
                {"type":"text","text":"answer"},{"type":"tool_use","id":"provider-call-1","name":"lookup","input":args}]},
            {"role":"user","content":[{"type":"tool_result","tool_use_id":"provider-call-1","is_error":false,
                "content":[{"type":"text","text":"tool result\n"},
                    {"type":"image","source":{"type":"base64","media_type":"image/png","data":"tool-image-bytes"}}]}]}
        ]})
    );
}

#[test]
fn actual_requests_preserve_empty_containers_null_members_and_literal_marker_keys() {
    let args = json!({"array":[],"object":{},"null":null,"nested":[null,[],{}],
        "cache_control":{"type":"literal-user-data"}});
    let parameters = json!({"type":"object","properties":{"choices":{"type":"array","default":[]}},"required":[]});
    let mut assistant = Message::assistant("");
    assistant.content = vec![Content::ToolCall {
        id: "same-id".into(),
        name: "lookup".into(),
        arguments: args.clone(),
        thought_signature: None,
    }];
    let context = Context {
        system: None,
        messages: vec![Message::user("call"), assistant],
        tools: vec![tool(parameters.clone())],
    };
    let body = request(&context, &Options::default());
    assert_eq!(
        body["messages"][1]["content"][0]["input"], args,
        "wire serialization must not erase nulls or change array/object identity"
    );
    assert_eq!(body["tools"][0]["input_schema"], parameters);
}

#[test]
fn actual_forced_schema_requests_preserve_empty_schema_arrays_and_objects() {
    let schema =
        json!({"type":"object","properties":{},"required":[],"additionalProperties":false});
    let options = Options {
        schema: Some(Schema {
            name: "answer".into(),
            schema: schema.clone(),
        }),
        ..Options::default()
    };
    let body = request(
        &Context {
            messages: vec![Message::user("answer")],
            ..Context::default()
        },
        &options,
    );
    assert_eq!(body["tool_choice"], json!({"type":"tool","name":"answer"}));
    assert_eq!(body["tools"][0]["input_schema"], schema);
}

#[test]
fn ordinary_appended_rounds_preserve_the_unmarked_earlier_content_and_tool_frame() {
    let mut context = Context {
        system: Some("Stable".into()),
        messages: vec![Message::user("first"), Message::assistant("answer")],
        tools: vec![tool(json!({"type":"object"}))],
    };
    let wire = adapter("anthropic-messages");
    let model = plain_model();
    let first = wire.request(&model, &context, &Options::default());
    let repeated = wire.request(&model, &context, &Options::default());
    assert_eq!(
        serde_json::to_vec(&first).expect("first bytes"),
        serde_json::to_vec(&repeated).expect("repeated bytes")
    );
    context
        .messages
        .extend([Message::user("next"), Message::assistant("next answer")]);
    let next = wire.request(&model, &context, &Options::default());
    assert_eq!(next["system"], first["system"]);
    assert_eq!(next["tools"], first["tools"]);
    assert_eq!(
        next["messages"]
            .as_array()
            .expect("appended messages")
            .get(..2),
        first["messages"].as_array().map(Vec::as_slice)
    );
}
