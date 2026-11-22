use super::{Model, ThinkingLevel, effective};

pub(crate) fn requested(model: &Model, selected: Option<ThinkingLevel>) -> Option<String> {
    if !model.reasoning {
        return None;
    }
    if model.thinking.is_empty() {
        return selected
            .filter(|level| *level != ThinkingLevel::Off)
            .and_then(|level| effective(model, level));
    }
    let Some(selected) = selected else {
        return effective(model, ThinkingLevel::Off);
    };
    let at = ThinkingLevel::ALL
        .iter()
        .position(|level| *level == selected)?;
    ThinkingLevel::ALL
        .iter()
        .skip(at)
        .chain(ThinkingLevel::ALL.iter().take(at).rev())
        .find_map(|level| effective(model, *level))
}
