use super::*;
use serde_json::json;

fn message(prompt: u64, limit: u64, output: u64) -> String {
    format!(
        "'max_tokens' or 'max_completion_tokens' is too large: {output}. This model's maximum context length is {limit} tokens and your request has {prompt} input tokens ({output} > {limit} - {prompt})."
    )
}

fn envelope(message: &str) -> String {
    json!({"error":{"message":message,"type":"BadRequestError","param":null,"code":400}})
        .to_string()
}

#[test]
fn verified_output_template_requires_exact_counts_and_safe_available_room() {
    for (prompt, limit, output) in [
        (12000, 16000, 8000),
        (0, 1, 2),
        (u64::MAX - 1, u64::MAX, 2),
        (1, u64::MAX, u64::MAX),
    ] {
        assert_eq!(
            classified(&envelope(&message(prompt, limit, output))),
            Some(Detail {
                kind: Kind::OutputBudget,
                prompt_tokens: Some(prompt),
                limit_tokens: Some(limit),
                requested_output_tokens: Some(output)
            })
        );
    }
    for (prompt, limit, output) in [
        (0, 0, 1),
        (16000, 16000, 8000),
        (16001, 16000, 8000),
        (12000, 16000, 4000),
        (12000, 16000, 0),
        (0, u64::MAX, u64::MAX),
        (u64::MAX, u64::MAX, u64::MAX),
    ] {
        assert_eq!(classified(&envelope(&message(prompt, limit, output))), None);
    }
}

#[test]
fn unverified_output_templates_never_produce_counts_or_output_classification() {
    let known = message(12000, 16000, 8000);
    for invalid in [
        format!("prefix {known}"),
        format!("{known} suffix"),
        known.replace("8000 >", "8001 >"),
        known.replace("16000 -", "16001 -"),
        known.replace("12000).", "12001)."),
        known.replace("12000", "-1"),
        known.replace("8000", "+8000"),
        known.replace("8000", "08000"),
        known.replace("8000", "8000.0"),
        known.replace("8000", "8e3"),
        known.replace("8000", "18446744073709551616"),
        known.replace("input tokens", "at least input tokens"),
        known.replace(" - ", " + "),
        known.trim_end_matches('.').to_owned(),
        "context length exceeded: 12000 + 8000 > 16000".into(),
    ] {
        assert_eq!(classified(&envelope(&invalid)), None, "{invalid}");
    }
    let body = envelope(&known);
    for invalid in [
        body.replace("BadRequestError", "AuthenticationError"),
        body.replace("\"code\":400", "\"code\":429"),
        body.replace("\"code\":400", "\"code\":\"400\""),
        body.replace("\"param\":null", "\"param\":\"max_tokens\""),
        body.replace("\"param\":null", "\"param\":null,\"param\":null"),
        body.replace("\"code\":400", "\"code\":400,\"code\":400"),
        body.replace("\"error\":", "\"error\":{},\"error\":"),
    ] {
        assert_eq!(classified(&invalid), None, "{invalid}");
    }
}

#[test]
fn output_metadata_requires_the_right_protocol_status_and_refusal_class() {
    use crate::mind::provider::{model::Api, retry::RetryClass};
    let body = envelope(&message(12000, 16000, 8000));
    assert_eq!(
        super::super::classified(Api::OpenAiCompletions, 400, RetryClass::Overflow, &body)
            .map(|detail| detail.kind),
        Some(Kind::OutputBudget)
    );
    for api in Api::all()
        .into_iter()
        .filter(|api| *api != Api::OpenAiCompletions)
    {
        assert_eq!(
            super::super::classified(api, 400, RetryClass::Overflow, &body)
                .map(|detail| detail.kind),
            Some(Kind::Unknown)
        );
    }
    for status in [200, 401, 402, 413, 422, 429, 500] {
        assert_eq!(
            super::super::classified(Api::OpenAiCompletions, status, RetryClass::Overflow, &body)
                .map(|detail| detail.kind),
            Some(Kind::Unknown)
        );
    }
    for class in [
        RetryClass::Auth,
        RetryClass::Invalid,
        RetryClass::Throttle,
        RetryClass::Transport,
        RetryClass::Overload,
        RetryClass::Unknown,
    ] {
        assert_eq!(
            super::super::classified(Api::OpenAiCompletions, 400, class, &body),
            None
        );
    }
}
