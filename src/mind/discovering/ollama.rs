//! What ollama's OpenAI surface leaves out of a model: its window, and whether it reasons or sees.
//! `/v1/models` names a model and nothing else; `/api/show` beside it says the rest.

use crate::mind::model::ThinkingLevel;
use crate::mind::provider::model::{Modality, Model};

/// What ollama calls no reasoning. It refuses "off" outright.
const NONE: &str = "none";

/// `model` with what `/api/show` says of it, or as it was when that cannot be asked.
pub(super) async fn detailed(model: Model, root: &str, client: &reqwest::Client) -> Model {
    let shown = async {
        client
            .post(format!("{root}/api/show"))
            .json(&serde_json::json!({ "model": model.id }))
            .send()
            .await
            .ok()?
            .json::<serde_json::Value>()
            .await
            .ok()
    }
    .await;
    match shown {
        Some(shown) => apply(model, &shown),
        None => model,
    }
}

/// Where `/api/show` lives, from the OpenAI base it sits beside.
pub(super) fn root(base: &str) -> String {
    let base = base.trim_end_matches('/');
    base.strip_suffix("/v1").unwrap_or(base).to_owned()
}

/// `model` with `shown`'s window and capabilities written over what the listing assumed.
pub(super) fn apply(mut model: Model, shown: &serde_json::Value) -> Model {
    let can = |what: &str| {
        shown
            .get("capabilities")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|list| list.iter().any(|it| it.as_str() == Some(what)))
    };
    // Keyed by architecture, `qwen3.context_length` and so on; there is only ever one.
    let window = shown
        .get("model_info")
        .and_then(serde_json::Value::as_object)
        .and_then(|info| {
            info.iter()
                .find(|(key, _)| key.ends_with(".context_length"))
                .and_then(|(_, value)| value.as_u64())
        })
        .filter(|window| *window > 0);
    if let Some(window) = window {
        model.context_window = window;
        model.max_tokens = (window / 4).max(1);
    }
    if can("thinking") {
        model.reasoning = true;
        model
            .thinking
            .insert(ThinkingLevel::Off, Some(NONE.to_owned()));
    }
    if can("vision") && !model.input.contains(&Modality::Image) {
        model.input.push(Modality::Image);
    }
    model
}

#[cfg(test)]
#[path = "ollama/tests.rs"]
mod tests;
