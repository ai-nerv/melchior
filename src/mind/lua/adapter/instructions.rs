use crate::mind::model::{Content, Context, Role};
use crate::mind::provider::{compat, model::Model};

mod tools;

pub(super) fn context(api: &str, model: &Model, original: &Context) -> Result<Context, String> {
    tools::validate(original)?;
    let mut context = original.clone();
    for message in &context.messages {
        if message.role != Role::System {
            continue;
        }
        if message.stop_reason.is_some()
            || message.usage.is_some()
            || message.error.is_some()
            || message.content.iter().any(|c| {
                !matches!(
                    c,
                    Content::Text {
                        signature: None,
                        ..
                    } | Content::ToolChanges { .. }
                )
            })
        {
            return Err(
                "system messages must contain only unsigned text or tool changes without assistant metadata".into(),
            );
        }
    }
    let compat = compat::resolve(model.compat);
    tools::apply(
        &mut context,
        api == "openai-completions"
            && compat.supports_mid_convo_system_messages
            && compat.supports_mid_convo_tool_additions,
    )?;
    if !context.messages.iter().any(|m| m.role == Role::System) {
        return Ok(context);
    }
    if compat.supports_mid_convo_system_messages {
        if !matches!(api, "openai-completions" | "mistral-conversations") {
            return Err("this adapter cannot preserve mid-conversation system messages".into());
        }
        return Ok(context);
    }
    let mut instructions: Vec<_> = context
        .system
        .clone()
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect();
    context.messages.retain(|message| {
        if message.role == Role::System {
            let text = message.text();
            if !text.is_empty() {
                instructions.push(text);
            }
            false
        } else {
            true
        }
    });
    if !instructions.is_empty() {
        context.system = Some(instructions.join("\n\n"));
    }
    Ok(context)
}

#[cfg(test)]
mod tests;
