use super::support::{adapter, plain_model};
use crate::mind::model::{Content, Context, Image, Message, Role};
use crate::mind::provider::api::{Adapter, Options};
use crate::mind::provider::model::Modality;

fn context() -> Context {
    let mut user = Message::user("inspect the image");
    user.content.push(Content::Image {
        data: "user-image".into(),
        media_type: "image/png".into(),
    });
    let mut assistant = Message::assistant("");
    for id in ["one", "two"] {
        assistant.content.push(Content::ToolCall {
            id: id.into(),
            name: "read".into(),
            arguments: serde_json::json!({}),
            thought_signature: None,
        });
    }
    let tools = ["one", "two"].map(|id| Message {
        role: Role::Tool,
        content: vec![Content::ToolResult {
            id: id.into(),
            name: "read".into(),
            content: format!("image {id}"),
            is_error: false,
            images: vec![Image {
                data: format!("tool-{id}"),
                media_type: "image/png".into(),
            }],
        }],
        stop_reason: None,
        usage: None,
        error: None,
    });
    Context {
        messages: vec![user, assistant, tools[0].clone(), tools[1].clone()],
        ..Context::default()
    }
}

#[test]
fn every_vision_protocol_delivers_user_and_tool_images() {
    let mut model = plain_model();
    model.input.push(Modality::Image);
    for name in [
        "openai-completions",
        "openai-responses",
        "anthropic-messages",
        "google-generative-ai",
        "pi-messages",
    ] {
        let body = adapter(name).request(&model, &context(), &Options::default());
        let text = serde_json::to_string(&body).expect("body");
        assert!(text.contains("user-image"), "{name}: {body}");
        assert!(text.contains("tool-one"), "{name}: {body}");
        assert!(text.contains("tool-two"), "{name}: {body}");
        match name {
            "openai-completions" => {
                let messages = body["messages"].as_array().expect("messages");
                assert_eq!(messages[0]["content"][1]["type"], "image_url");
                assert_eq!(messages[2]["role"], "tool");
                assert_eq!(messages[3]["role"], "tool");
                assert_eq!(messages[4]["role"], "user");
                assert_eq!(messages[4]["content"][1]["type"], "image_url");
            }
            "openai-responses" => {
                let input = body["input"].as_array().expect("input");
                assert_eq!(input[0]["content"][1]["type"], "input_image");
                assert_eq!(input[3]["type"], "function_call_output");
                assert_eq!(input[3]["output"][1]["type"], "input_image");
            }
            "anthropic-messages" => {
                assert_eq!(
                    body["messages"][2]["content"][0]["content"][1]["type"],
                    "image"
                );
            }
            "google-generative-ai" => {
                assert_eq!(
                    body["contents"][2]["parts"][1]["inlineData"]["data"],
                    "tool-one"
                );
            }
            "pi-messages" => {
                assert_eq!(
                    body["messages"][2]["content"][0]["content"][1]["type"],
                    "image"
                );
            }
            _ => unreachable!(),
        }
    }
}

#[test]
fn text_only_requests_omit_images_without_mutating_history() {
    let context = context();
    let original = context.clone();
    for name in [
        "openai-completions",
        "openai-responses",
        "anthropic-messages",
        "google-generative-ai",
        "pi-messages",
        "decisions",
    ] {
        let body = adapter(name).request(&plain_model(), &context, &Options::default());
        let text = serde_json::to_string(&body).expect("body");
        assert!(!text.contains("user-image"), "{name}: {body}");
        assert!(!text.contains("tool-one"), "{name}: {body}");
        assert!(text.contains("does not support images"), "{name}: {body}");
    }
    assert_eq!(context, original);
    let mut model = plain_model();
    model.input.push(Modality::Image);
    let restored = adapter("openai-completions").request(&model, &context, &Options::default());
    assert_eq!(
        restored["messages"][0]["content"][1]["image_url"]["url"],
        "data:image/png;base64,user-image"
    );
}
