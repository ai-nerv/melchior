use super::{
    api::Options,
    compat,
    model::Model,
    strict_schema::{self, Dialect},
};
use crate::mind::model::{Content, Context, Tool, ToolStrictness};
use serde_json::{Value, json};

fn dialect(api: &str, model: &Model) -> Option<Dialect> {
    let compat = compat::resolve(model.compat);
    match api {
        "openai-completions"
            if compat.supports_strict_mode
                && compat.chat_strict_dialect == compat::ChatStrictDialect::DeepSeek =>
        {
            Some(Dialect::DeepSeek)
        }
        "openai-completions"
        | "mistral-conversations"
        | "openai-responses"
        | "azure-openai-responses"
        | "openai-codex-responses"
            if compat.supports_strict_mode =>
        {
            Some(Dialect::OpenAi)
        }
        "anthropic-messages" if compat.supports_strict_tools => Some(Dialect::Anthropic),
        _ => None,
    }
}

pub(crate) fn advertised(model: &Model) -> Option<bool> {
    let compat = model.compat?;
    (compat.supports_strict_mode.is_some() || compat.supports_strict_tools.is_some())
        .then(|| dialect(model.api.as_str(), model).is_some())
}

pub(crate) fn context(
    api: &str,
    model: &Model,
    context: &Context,
    options: &Options,
) -> Result<Value, String> {
    let mut encoded = serde_json::to_value(context).map_err(|_| "could not encode tool context")?;
    let mut declarations: Vec<_> = context.tools.iter().collect();
    for content in context.messages.iter().flat_map(|m| &m.content) {
        if let Content::ToolChanges { added, .. } = content {
            declarations.extend(added);
        }
    }
    let mut values = normalized(api, model, &declarations, options)?.into_iter();
    let fields = encoded.as_object_mut().ok_or("invalid tool context")?;
    if !context.tools.is_empty() {
        fields.insert(
            "tools".into(),
            Value::Array(values.by_ref().take(context.tools.len()).collect()),
        );
    }
    for (message, value) in context.messages.iter().zip(
        fields
            .get_mut("messages")
            .and_then(Value::as_array_mut)
            .ok_or("invalid tool context")?,
    ) {
        for (content, value) in message.content.iter().zip(
            value["content"]
                .as_array_mut()
                .ok_or("invalid tool message")?,
        ) {
            if let Content::ToolChanges { added, .. } = content
                && !added.is_empty()
            {
                value["added"] = Value::Array(values.by_ref().take(added.len()).collect());
            }
        }
    }
    Ok(encoded)
}

fn normalized(
    api: &str,
    model: &Model,
    declarations: &[&Tool],
    options: &Options,
) -> Result<Vec<Value>, String> {
    let dialect = dialect(api, model);
    let mut tools: Vec<Value> = declarations
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<_, _>>()
        .map_err(|_| "could not encode tool declarations")?;
    for (tool, value) in declarations.iter().zip(tools.iter_mut()) {
        let fields = value.as_object_mut().ok_or("invalid tool declaration")?;
        fields.remove("strict");
        let mut strict = false;
        if let Some(policy) = tool.strict.filter(|policy| {
            !options.disable_preferred_strict || *policy == ToolStrictness::Require
        }) {
            let result = match dialect {
                None => Err("strict tools are unsupported"),
                Some(_)
                    if api == "anthropic-messages"
                        && options.schema.is_some()
                        && compat::resolve(model.compat).anthropic_schema
                            == compat::AnthropicSchema::ForcedTool =>
                {
                    Err("structured answers replace the declared tools")
                }
                Some(dialect) => strict_schema::normalize(&tool.parameters, dialect),
            };
            match result {
                Ok(parameters) => {
                    fields.insert("parameters".into(), parameters);
                    strict = true;
                }
                Err(why) if policy == ToolStrictness::Require => {
                    return Err(format!(
                        "tool {:?} requires strict arguments: {why}",
                        tool.name
                    ));
                }
                Err(_) => {}
            }
        }
        if matches!(dialect, Some(Dialect::OpenAi | Dialect::DeepSeek)) || strict {
            fields.insert("__strict".into(), json!(strict));
        }
    }
    if dialect == Some(Dialect::DeepSeek) {
        let count = tools.iter().filter(|tool| tool["__strict"] == true).count();
        if count > 0 && count < tools.len() {
            if declarations
                .iter()
                .any(|tool| tool.strict == Some(ToolStrictness::Require))
            {
                return Err("DeepSeek requires every tool to opt into a supported strict schema when any tool requires strict arguments".into());
            }
            for (tool, value) in declarations.iter().zip(tools.iter_mut()) {
                value["parameters"] = tool.parameters.clone();
                value["__strict"] = json!(false);
            }
        }
    }
    Ok(tools)
}

pub(crate) fn limit_error(api: super::model::Api, status: u16, body: &str) -> bool {
    if api != super::model::Api::AnthropicMessages || status != 400 {
        return false;
    }
    let Ok(detail) = serde_json::from_str::<Value>(body) else {
        return false;
    };
    if detail.pointer("/error/type").and_then(Value::as_str) != Some("invalid_request_error") {
        return false;
    }
    detail
        .pointer("/error/message")
        .and_then(Value::as_str)
        .is_some_and(|message| {
            [
                "The compiled grammar is too large",
                "Too many strict tools (",
                "Schemas contains too many parameters with union types (",
            ]
            .iter()
            .any(|phrase| message.contains(phrase))
        })
}
