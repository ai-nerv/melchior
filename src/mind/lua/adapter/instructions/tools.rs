use crate::mind::model::{Content, Context, Role, Tool};
use std::collections::{BTreeMap, BTreeSet};

const ACTIVE: usize = 4096;
const CHANGES: usize = 16384;
const BYTES: usize = 8 * 1024 * 1024;

struct Budget(usize);

impl std::io::Write for Budget {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self.0.checked_sub(bytes.len()).ok_or_else(|| {
            std::io::Error::other("tool history exceeds its declaration byte limit")
        })?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Default)]
struct State<'a> {
    positions: BTreeMap<&'a str, usize>,
    order: Vec<Option<&'a Tool>>,
}

impl<'a> State<'a> {
    fn add(&mut self, tool: &'a Tool) -> Result<(), String> {
        if let Some(at) = self.positions.get(tool.name.as_str()).copied() {
            let previous = self.order.get_mut(at).ok_or("invalid tool history state")?;
            *previous = Some(tool);
        } else {
            if self.positions.len() >= ACTIVE {
                return Err("tool history exceeds 4096 active tools".into());
            }
            self.positions.insert(tool.name.as_str(), self.order.len());
            self.order.push(Some(tool));
        }
        Ok(())
    }
    fn remove(&mut self, name: &str) {
        if let Some(at) = self.positions.remove(name)
            && let Some(slot) = self.order.get_mut(at)
        {
            *slot = None;
        }
    }
    fn owned(&self) -> Vec<Tool> {
        self.order
            .iter()
            .flatten()
            .map(|tool| (*tool).clone())
            .collect()
    }
}

fn name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
        return Err(
            "tool history names must contain 1 to 256 bytes without control characters".into(),
        );
    }
    Ok(())
}

pub(super) fn validate(context: &Context) -> Result<(), String> {
    if !context
        .messages
        .iter()
        .flat_map(|m| &m.content)
        .any(|c| matches!(c, Content::ToolChanges { .. }))
    {
        return Ok(());
    }
    let mut count = context.tools.len();
    if count > CHANGES {
        return Err("tool history exceeds 16384 declarations and removals".into());
    }
    let mut bytes = Budget(BYTES);
    for tool in &context.tools {
        name(&tool.name)?;
    }
    serde_json::to_writer(&mut bytes, &context.tools)
        .map_err(|_| "tool history exceeds 8 MiB of declarations")?;
    for message in &context.messages {
        let mut blocks = 0;
        for content in &message.content {
            if let Content::ToolChanges { added, removed } = content {
                blocks += 1;
                if message.role != Role::System || blocks > 1 {
                    return Err("tool changes require one block per System message".into());
                }
                count = count
                    .saturating_add(added.len())
                    .saturating_add(removed.len());
                if count > CHANGES {
                    return Err("tool history exceeds 16384 declarations and removals".into());
                }
                for tool in added {
                    name(&tool.name)?;
                }
                for removed in removed {
                    name(removed)?;
                }
                serde_json::to_writer(&mut bytes, content)
                    .map_err(|_| "tool history exceeds 8 MiB of declarations")?;
            }
        }
    }
    if count > CHANGES {
        return Err("tool history exceeds 16384 declarations and removals".into());
    }
    Ok(())
}

pub(super) fn apply(context: &mut Context, additions: bool) -> Result<(), String> {
    if !context
        .messages
        .iter()
        .flat_map(|m| &m.content)
        .any(|c| matches!(c, Content::ToolChanges { .. }))
    {
        return Ok(());
    }
    let initial = context.tools.is_empty()
        && context.system.as_deref().is_none_or(str::is_empty)
        && context
            .messages
            .first()
            .is_some_and(|m| m.role == Role::System);
    let mut folded = initial.then_some(0);
    let mut declared: BTreeSet<&str> = context.tools.iter().map(|t| t.name.as_str()).collect();
    for (at, message) in context.messages.iter().enumerate() {
        for content in &message.content {
            if let Content::ToolChanges { added, removed } = content {
                if !removed.is_empty() {
                    folded = Some(at);
                }
                for tool in added {
                    if !declared.insert(&tool.name) {
                        folded = Some(at);
                    }
                }
            }
        }
    }
    let mut state = State::default();
    for tool in &context.tools {
        state.add(tool)?;
    }
    let mut request = if additions && folded.is_none() {
        Some(state.owned())
    } else {
        None
    };
    for (at, message) in context.messages.iter().enumerate() {
        for content in &message.content {
            if let Content::ToolChanges { added, removed } = content {
                for removed in removed {
                    state.remove(removed);
                }
                for tool in added {
                    state.add(tool)?;
                }
            }
        }
        if additions && folded == Some(at) {
            request = Some(state.owned());
        }
    }
    context.tools = if additions {
        request.ok_or("invalid tool history fold boundary")?
    } else {
        state.owned()
    };
    for (at, message) in context.messages.iter_mut().enumerate() {
        message.content.retain(|content| {
            !matches!(content, Content::ToolChanges { .. })
                || (additions && folded.is_none_or(|end| at > end))
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
