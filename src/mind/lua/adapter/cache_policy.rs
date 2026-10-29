use super::support::{adapter, plain_model};
use crate::mind::model::{Content, Context, Message, RequestFrame};
use crate::mind::provider::{
    api::{Adapter, Options},
    compat::{Compat, PromptCache},
    model::Modality,
};
use serde_json::json;

#[test]
fn policy_observes_effective_capabilities_but_never_prompt_or_price() {
    let adapter = adapter("anthropic-messages");
    let mut model = plain_model();
    let context = Context {
        system: Some("SECRET rules".into()),
        messages: vec![Message::user("SECRET question")],
        ..Default::default()
    };
    let options = Options::default();
    let (_, first) = adapter.request_observed(&model, &context, &options);
    let first = first.expect("policy");
    assert_eq!(first.version, 1);
    model.cost.input = 99.0;
    model.name = "different display name".into();
    let changed = Context {
        system: Some("different rules".into()),
        messages: vec![Message::user("different question")],
        ..Default::default()
    };
    assert_eq!(
        adapter.request_observed(&model, &changed, &options).1,
        Some(first)
    );
    model.compat = Some(Compat {
        prompt_cache: Some(PromptCache::AnthropicEphemeral),
        ..Default::default()
    });
    let enabled = adapter
        .request_observed(&model, &context, &options)
        .1
        .expect("enabled");
    assert_ne!(enabled.policy, first.policy);
    assert_eq!(enabled.dialect, first.dialect);
    model.compat = Some(Compat {
        prompt_cache: Some(PromptCache::None),
        ..Default::default()
    });
    assert_eq!(
        adapter.request_observed(&model, &context, &options).1,
        Some(first)
    );
    for change in 0..4 {
        let mut next = model.clone();
        match change {
            0 => next.reasoning = !model.reasoning,
            1 => next.input.push(Modality::Image),
            2 => next.id = "another wire model".into(),
            _ => next.max_tokens += 1,
        }
        assert_ne!(
            adapter
                .request_observed(&next, &context, &options)
                .1
                .expect("changed")
                .policy,
            first.policy
        );
    }
    let text = serde_json::to_string(&first).expect("metadata");
    assert!(!text.contains("SECRET"));
    assert!(text.len() < 350);
}

#[test]
fn image_adaptation_is_counted_where_it_occurs_without_changing_history() {
    let mut user = Message::user("inspect");
    user.content.push(Content::Image {
        data: "USER-IMAGE".into(),
        media_type: "image/png".into(),
    });
    let mut tool = Message::user("");
    tool.content = vec![Content::ToolResult {
        id: "read-1".into(),
        name: "read".into(),
        content: "result".into(),
        is_error: false,
        images: vec![crate::mind::model::Image {
            data: "TOOL-IMAGE".into(),
            media_type: "image/png".into(),
        }],
    }];
    let context = Context {
        messages: vec![user, tool],
        ..Default::default()
    };
    let exact = context.clone();
    let model = plain_model();
    let mut vision = model.clone();
    vision.input.push(Modality::Image);
    for name in [
        "openai-completions",
        "openai-responses",
        "anthropic-messages",
        "google-generative-ai",
        "pi-messages",
    ] {
        let adapter = adapter(name);
        let options = Options::default();
        let (mut body, frame) = adapter.request_observed(&model, &context, &options);
        assert_eq!(frame.expect("frame").omitted_images, 2);
        let budget = body
            .as_object_mut()
            .expect("request")
            .remove("__budget")
            .expect("budget");
        assert_eq!(budget["effective"], 8192);
        assert_eq!(
            body.pointer(budget["path"].as_str().expect("path")),
            Some(&budget["effective"])
        );
        assert_eq!(body, adapter.request(&model, &context, &options));
        assert!(!body.to_string().contains("USER-IMAGE"));
        assert!(!body.to_string().contains("TOOL-IMAGE"));
        let (body, frame) = adapter.request_observed(&vision, &context, &options);
        assert_eq!(frame.expect("vision").omitted_images, 0);
        assert!(body.to_string().contains("USER-IMAGE"));
        assert!(body.to_string().contains("TOOL-IMAGE"));
        assert_eq!(context, exact);
    }
}

#[test]
fn dialect_identity_is_adapter_owned_and_metadata_decoding_is_fixed_size() {
    let model = plain_model();
    let context = Context::default();
    let options = Options::default();
    let (_, first) = adapter("anthropic-messages").request_observed(&model, &context, &options);
    let first = first.expect("first");
    let (_, next) = adapter("openai-completions").request_observed(&model, &context, &options);
    let next = next.expect("next");
    assert_ne!(first.dialect, next.dialect);
    assert_eq!(first.policy, next.policy);
    let mut value = serde_json::to_value(first).expect("value");
    for count in [0, 31, 33, 100] {
        value["policy"] = json!(vec![0; count]);
        assert!(serde_json::from_value::<RequestFrame>(value.clone()).is_err());
    }
    for value in [json!(-1), json!(1.5), json!(null)] {
        let mut invalid = serde_json::to_value(first).expect("value");
        invalid["omitted_images"] = value;
        assert!(serde_json::from_value::<RequestFrame>(invalid).is_err());
    }
}
