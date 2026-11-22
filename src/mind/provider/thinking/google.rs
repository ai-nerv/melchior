use super::{Model, ThinkingLevel};
use crate::mind::provider::{
    compat::{GoogleThinking, resolve},
    model::Api,
};
use serde_json::{Value, json};

const FOUR: &[&str] = &["minimal", "low", "medium", "high"];
const THREE: &[&str] = &["low", "medium", "high"];
const EXTENDED: &[&str] = &["low", "medium", "high", "xhigh"];

#[derive(Clone, Copy)]
enum Profile {
    Legacy,
    Pro25,
    Flash25,
    Lite25,
    Level(&'static [&'static str]),
}

pub(super) const fn is_google(model: &Model) -> bool {
    matches!(model.api, Api::GoogleGenerativeAi | Api::GoogleVertex)
}

fn named(id: &str, name: &str) -> bool {
    id == name
        || id
            .strip_prefix(name)
            .is_some_and(|suffix| suffix.starts_with('-'))
}

fn automatic(model: &Model) -> Profile {
    let id = model
        .id
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if named(&id, "gemini-2.5-pro") {
        return Profile::Pro25;
    }
    if named(&id, "gemini-2.5-flash-lite") {
        return Profile::Lite25;
    }
    if named(&id, "gemini-2.5-flash") {
        return Profile::Flash25;
    }
    if named(&id, "gemma-4") || named(&id, "gemma4") {
        return Profile::Level(&["minimal", "high"]);
    }
    if named(&id, "gemini-4-hydrogen") || named(&id, "gemini-4-neon") {
        return Profile::Level(EXTENDED);
    }
    if named(&id, "gemini-3-pro-image") {
        return Profile::Level(&["high"]);
    }
    if named(&id, "gemini-3.1-flash-image") || named(&id, "gemini-3.1-flash-lite-image") {
        return Profile::Level(&["minimal", "high"]);
    }
    if named(&id, "gemini-3-pro") {
        return Profile::Level(&["low", "high"]);
    }
    if named(&id, "gemini-4")
        || named(&id, "gemini-3.1-pro")
        || named(&id, "gemini-3.7-flash")
        || named(&id, "gemini-3.8-flash")
        || id == "gemini-pro-latest"
    {
        return Profile::Level(THREE);
    }
    if [
        "gemini-3-flash",
        "gemini-3.1-flash-lite",
        "gemini-3.5-flash",
        "gemini-3.6-flash",
        "gemini-robotics-er-2",
    ]
    .iter()
    .any(|name| named(&id, name))
        || matches!(
            id.as_str(),
            "gemini-flash-latest" | "gemini-flash-lite-latest"
        )
    {
        return Profile::Level(FOUR);
    }
    Profile::Legacy
}

fn profile(model: &Model) -> Profile {
    match resolve(model.compat).google_thinking {
        GoogleThinking::Auto => automatic(model),
        GoogleThinking::Level => Profile::Level(&["minimal", "low", "medium", "high", "xhigh"]),
        GoogleThinking::Budget => match automatic(model) {
            profile @ (Profile::Pro25 | Profile::Flash25 | Profile::Lite25) => profile,
            _ => Profile::Legacy,
        },
    }
}

pub(super) fn native_xhigh(model: &Model) -> bool {
    is_google(model)
        && resolve(model.compat).google_thinking == GoogleThinking::Auto
        && matches!(automatic(model), Profile::Level(levels) if levels.contains(&"xhigh"))
}

pub(super) fn known_controls(model: &Model) -> bool {
    is_google(model)
        && model.reasoning
        && (resolve(model.compat).google_thinking != GoogleThinking::Auto
            || !matches!(automatic(model), Profile::Legacy))
}

pub(super) fn mapped(model: &Model, name: &str) -> Option<String> {
    if !model.reasoning {
        return None;
    }
    let name = name.to_ascii_lowercase();
    let supported = match profile(model) {
        Profile::Level(levels) => levels.contains(&name.as_str()),
        Profile::Legacy => {
            ["off", "minimal", "low", "medium", "high", "max"].contains(&name.as_str())
        }
        family => {
            FOUR.contains(&name.as_str())
                || (name == "off" && !matches!(family, Profile::Pro25))
                || (name == "max"
                    && model
                        .thinking
                        .values()
                        .flatten()
                        .any(|value| value.eq_ignore_ascii_case("max")))
        }
    };
    supported.then_some(name)
}

pub(crate) fn configuration(model: &Model, requested: Option<ThinkingLevel>) -> Option<Value> {
    if !is_google(model) || !model.reasoning {
        return None;
    }
    let requested_level = requested.unwrap_or(ThinkingLevel::Off);
    let index = ThinkingLevel::ALL
        .iter()
        .position(|level| *level == requested_level)?;
    let name = ThinkingLevel::ALL
        .get(index..)?
        .iter()
        .copied()
        .chain(ThinkingLevel::ALL.get(..index)?.iter().rev().copied())
        .find_map(|level| super::effective(model, level))?;
    let summaries = requested.is_some_and(|level| level != ThinkingLevel::Off) && name != "off";
    let family = profile(model);
    if matches!(family, Profile::Level(_)) {
        return Some(
            json!({"kind":"level","value":name.to_ascii_uppercase(),"summaries":summaries,"active":true}),
        );
    }
    let budget = if name == "off" {
        0
    } else if matches!(family, Profile::Legacy) || name == "max" {
        -1
    } else {
        match name.as_str() {
            "minimal" => {
                if matches!(family, Profile::Lite25) {
                    512
                } else {
                    128
                }
            }
            "low" => 2048,
            "medium" => 8192,
            "high" => {
                if matches!(family, Profile::Pro25) {
                    32768
                } else {
                    24576
                }
            }
            _ => return None,
        }
    };
    Some(json!({"kind":"budget","value":budget,"summaries":summaries,"active":budget != 0}))
}
