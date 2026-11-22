use super::model::{Api, Modality, Model};
use crate::mind::model::{Capabilities, ThinkingLevel};

pub(crate) mod google;
pub(crate) mod mistral;

pub fn effective(model: &Model, level: ThinkingLevel) -> Option<String> {
    let mapped = mapped_only(model);
    if (level != ThinkingLevel::Off || mapped) && !model.reasoning {
        return None;
    }
    let declared = model
        .capabilities
        .as_ref()
        .and_then(|caps| caps.thinking_levels.as_ref());
    if declared.is_some_and(|levels| !levels.iter().any(|name| name == level.as_str())) {
        return None;
    }
    let name = match model.thinking.get(&level) {
        Some(name) => name.clone()?,
        None if mapped => return None,
        None if level == ThinkingLevel::XHigh
            && declared.is_none()
            && !google::native_xhigh(model) =>
        {
            return None;
        }
        None => level.as_str().to_owned(),
    };
    if name.is_empty() {
        return None;
    }
    if google::is_google(model) {
        return google::mapped(model, &name);
    }
    if model.api == Api::MistralConversations && !model.thinking.is_empty() {
        return ["none", "low", "medium", "high", "max"]
            .contains(&name.as_str())
            .then_some(name);
    }
    if level == ThinkingLevel::XHigh {
        match model.api {
            Api::OpenAiCompletions => {
                use super::compat::ThinkingFormat;
                let compat = super::compat::resolve(model.compat);
                let supported = match compat.thinking_format {
                    ThinkingFormat::OpenRouter | ThinkingFormat::AntLing => true,
                    ThinkingFormat::OpenAi
                    | ThinkingFormat::DeepSeek
                    | ThinkingFormat::Baseten
                    | ThinkingFormat::Together
                    | ThinkingFormat::Qwen => compat.supports_reasoning_effort,
                    ThinkingFormat::Zai => false,
                };
                if !supported {
                    return None;
                }
            }
            Api::AnthropicMessages
                if super::compat::resolve(model.compat).anthropic_thinking
                    == super::compat::AnthropicThinking::AdaptiveEffort => {}
            Api::AnthropicMessages | Api::GoogleGenerativeAi | Api::GoogleVertex
                if !["off", "minimal", "low", "medium", "high", "max"].contains(&name.as_str()) =>
            {
                return None;
            }
            Api::BedrockConverseStream | Api::Decisions => return None,
            _ => {}
        }
    }
    Some(name)
}

pub fn controls(model: &Model) -> Option<Vec<String>> {
    if model.thinking.is_empty()
        && !mapped_only(model)
        && !google::known_controls(model)
        && model
            .capabilities
            .as_ref()
            .and_then(|caps| caps.thinking_levels.as_ref())
            .is_none()
    {
        return None;
    }
    Some(
        ThinkingLevel::ALL
            .into_iter()
            .filter(|level| effective(model, *level).is_some())
            .map(|level| level.as_str().to_owned())
            .collect(),
    )
}

fn mapped_only(model: &Model) -> bool {
    (model.api == Api::MistralConversations && !model.thinking.is_empty())
        || model.api == Api::OpenAiCompletions
            && super::compat::resolve(model.compat).thinking_format
                == super::compat::ThinkingFormat::AntLing
}

pub fn capabilities(model: &Model) -> Option<Capabilities> {
    let levels = controls(model);
    let strict = super::strict::advertised(model);
    if model.capabilities.is_none()
        && levels.is_none()
        && strict.is_none()
        && model.input_limits.is_none()
    {
        return None;
    }
    let mut caps = model.capabilities.clone().unwrap_or_else(|| Capabilities {
        input: model
            .input
            .iter()
            .map(|modality| match modality {
                Modality::Text => "text".into(),
                Modality::Image => "image".into(),
            })
            .collect(),
        output: vec!["text".into()],
        ..Default::default()
    });
    caps.thinking_levels = levels;
    caps.strict_tools = strict;
    caps.input_limits.clone_from(&model.input_limits);
    Some(caps)
}

#[cfg(test)]
mod tests;
