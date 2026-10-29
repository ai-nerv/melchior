use crate::mind::model::overflow::{Detail, Kind};
use crate::mind::provider::model::Api;
use crate::mind::provider::retry::RetryClass;
use serde::Deserialize;

mod vllm;

#[derive(Deserialize)]
struct Envelope {
    #[serde(rename = "type")]
    kind: String,
    error: Failure,
}

#[derive(Deserialize)]
struct Failure {
    #[serde(rename = "type")]
    kind: String,
    message: String,
}

pub(super) fn classified(api: Api, status: u16, class: RetryClass, body: &str) -> Option<Detail> {
    if class != RetryClass::Overflow {
        return None;
    }
    if api == Api::OpenAiCompletions
        && status == 400
        && let Some(detail) = vllm::classified(body)
    {
        return Some(detail);
    }
    let input = api == Api::AnthropicMessages
        && status == 400
        && serde_json::from_str::<Envelope>(body).is_ok_and(|envelope| {
            envelope.kind == "error"
                && envelope.error.kind == "invalid_request_error"
                && envelope
                    .error
                    .message
                    .strip_prefix("prompt is too long")
                    .is_some_and(|tail| tail.is_empty() || tail.starts_with(':'))
        });
    Some(Detail {
        kind: if input { Kind::Input } else { Kind::Unknown },
        prompt_tokens: None,
        limit_tokens: None,
        requested_output_tokens: None,
    })
}

#[cfg(test)]
mod tests;
