//! Published input, output, and thinking metadata.

use crate::mind::model::Capabilities;
use serde_json::Value;

pub(super) fn names(value: Option<&Value>) -> Vec<String> {
    let mut out = Vec::new();
    for value in value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(32)
    {
        let name = match value {
            Value::Bool(true) => "on",
            Value::Bool(false) => "off",
            Value::String(name) => name,
            _ => continue,
        };
        if name.is_empty()
            || name.len() > 32
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            continue;
        }
        let name = name.to_owned();
        if !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

pub(super) fn from_entry(entry: &Value) -> Capabilities {
    let input = names(
        entry
            .pointer("/architecture/input_modalities")
            .or_else(|| entry.get("input")),
    );
    let output = names(entry.pointer("/architecture/output_modalities"));
    let thinking = names(entry.pointer("/reasoning/supported_efforts"));
    Capabilities {
        input,
        output,
        thinking: (!thinking.is_empty()).then_some(thinking),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn published_modalities_and_native_efforts_survive() {
        let caps = from_entry(&json!({
            "architecture": {"input_modalities": ["text", "image", "audio", "video", "file"],
                "output_modalities": ["text", "audio"]},
            "reasoning": {"supported_efforts": ["low", "high", "xhigh"]}
        }));
        assert_eq!(caps.input, ["text", "image", "audio", "video", "file"]);
        assert_eq!(caps.output, ["text", "audio"]);
        assert_eq!(
            caps.thinking.expect("advertised levels"),
            ["low", "high", "xhigh"]
        );
    }

    #[test]
    fn absent_levels_are_unknown_and_labels_cannot_inject_terminal_controls() {
        assert_eq!(from_entry(&json!({})).thinking, None);
        assert_eq!(
            names(Some(&json!([
                false,
                true,
                "HIGH",
                "high",
                "bad\nname",
                "\u{001b}[0m"
            ]))),
            ["off", "on", "HIGH", "high"]
        );
    }
}
