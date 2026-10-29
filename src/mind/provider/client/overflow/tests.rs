use super::*;
use serde_json::json;

fn envelope(message: &str) -> String {
    json!({"type":"error","error":{"type":"invalid_request_error","message":message}}).to_string()
}

#[test]
fn only_known_http_structure_proves_input_and_never_extracts_prose_counts() {
    for message in [
        "prompt is too long",
        "prompt is too long: 250000 tokens > 200000 maximum",
        "prompt is too long: 18446744073709551616 > -1",
    ] {
        let detail = classified(
            Api::AnthropicMessages,
            400,
            RetryClass::Overflow,
            &envelope(message),
        );
        assert_eq!(
            detail,
            Some(Detail {
                kind: Kind::Input,
                prompt_tokens: None,
                limit_tokens: None,
                requested_output_tokens: None
            })
        );
    }
    for body in [
        envelope("context length exceeded"),
        envelope("prefix: prompt is too long"),
        envelope("prompt is too longer"),
        "prompt is too long".into(),
        r#"{"type":"error","error":{"type":"invalid_request_error","message":"prompt is too long","message":"context length exceeded"}}"#.into(),
        r#"{"type":"error","type":"error","error":{"type":"invalid_request_error","message":"prompt is too long"}}"#.into(),
        json!({"type":"error","error":{"type":"authentication_error","message":"prompt is too long"}}).to_string(),
        json!({"error":{"type":"invalid_request_error","message":"prompt is too long"}}).to_string(),
        json!({"type":"error","error":{"type":"invalid_request_error","message":["prompt is too long"]}}).to_string(),
    ] {
        assert_eq!(classified(Api::AnthropicMessages, 400, RetryClass::Overflow, &body)
            .map(|detail|detail.kind),Some(Kind::Unknown),"{body}");
    }
}

#[test]
fn status_api_and_existing_refusal_class_are_not_overridden_by_body_text() {
    let body = envelope("prompt is too long: 250000 tokens > 200000 maximum");
    for class in [
        RetryClass::Auth,
        RetryClass::Throttle,
        RetryClass::Invalid,
        RetryClass::Transport,
        RetryClass::Overload,
        RetryClass::Unknown,
    ] {
        assert_eq!(classified(Api::AnthropicMessages, 400, class, &body), None);
    }
    for status in [200, 401, 402, 413, 422, 429, 500] {
        assert_eq!(
            classified(Api::AnthropicMessages, status, RetryClass::Overflow, &body)
                .map(|detail| detail.kind),
            Some(Kind::Unknown)
        );
    }
    for api in Api::all()
        .into_iter()
        .filter(|api| *api != Api::AnthropicMessages)
    {
        assert_eq!(
            classified(api, 400, RetryClass::Overflow, &body).map(|detail| detail.kind),
            Some(Kind::Unknown)
        );
    }
}
