use crate::mind::model::overflow::{Detail, Kind};
use serde::Deserialize;

#[derive(Deserialize)]
struct Envelope {
    error: Failure,
}

#[derive(Deserialize)]
struct Failure {
    message: String,
    #[serde(rename = "type")]
    kind: String,
    code: u16,
    #[serde(default)]
    param: Option<String>,
}

pub(super) fn classified(body: &str) -> Option<Detail> {
    let envelope = serde_json::from_str::<Envelope>(body).ok()?;
    let error = envelope.error;
    if error.kind != "BadRequestError" || error.code != 400 || error.param.is_some() {
        return None;
    }
    let mut text = error
        .message
        .strip_prefix("'max_tokens' or 'max_completion_tokens' is too large: ")?;
    let output = count(&mut text, ". This model's maximum context length is ")?;
    let limit = count(&mut text, " tokens and your request has ")?;
    let prompt = count(&mut text, " input tokens (")?;
    if count(&mut text, " > ")? != output
        || count(&mut text, " - ")? != limit
        || count(&mut text, ").")? != prompt
        || !text.is_empty()
        || prompt >= limit
        || output <= limit.checked_sub(prompt)?
    {
        return None;
    }
    Some(Detail {
        kind: Kind::OutputBudget,
        prompt_tokens: Some(prompt),
        limit_tokens: Some(limit),
        requested_output_tokens: Some(output),
    })
}

fn count(text: &mut &str, separator: &str) -> Option<u64> {
    let (number, rest) = text.split_once(separator)?;
    if number.is_empty()
        || number.len() > 20
        || !number.bytes().all(|byte| byte.is_ascii_digit())
        || (number.len() > 1 && number.starts_with('0'))
    {
        return None;
    }
    let number = number.parse().ok()?;
    *text = rest;
    Some(number)
}

#[cfg(test)]
mod tests;
