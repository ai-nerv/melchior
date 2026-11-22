use super::*;
use melchior::mind::model::InputLimits;

pub(super) fn convert(raw: &Value, pending: &mut BTreeSet<String>) -> Result<Value> {
    let value = mapped(raw, "inputLimits", pending)?;
    serde_json::from_value::<InputLimits>(value.clone()).context("invalid input limits")?;
    Ok(value)
}

fn mapped(raw: &Value, path: &str, pending: &mut BTreeSet<String>) -> Result<Value> {
    let mut out = Map::new();
    for (field, value) in raw.as_object().context("input limits must be an object")? {
        let native = match (path, field.as_str()) {
            ("inputLimits", "maxRequestBytes") => "max_request_bytes",
            ("inputLimits", "images") => "images",
            ("inputLimits.images", "maxPerMessage") => "max_per_message",
            ("inputLimits.images", "maxPerRequest") => "max_per_request",
            ("inputLimits.images", "resize") => "resize",
            ("inputLimits.images.resize", "maxWidth") => "max_width",
            ("inputLimits.images.resize", "maxHeight") => "max_height",
            ("inputLimits.images.resize", "maxBytes") => "max_bytes",
            ("inputLimits.images.resize", "jpegQuality") => "jpeg_quality",
            _ => {
                pending.insert(format!("{path}.{field}"));
                continue;
            }
        };
        let value = if matches!(native, "images" | "resize") {
            mapped(value, &format!("{path}.{field}"), pending)?
        } else {
            ensure!(
                value.as_u64().is_some_and(|value| value > 0),
                "input limits must be positive integers"
            );
            value.clone()
        };
        out.insert(native.into(), value);
    }
    Ok(Value::Object(out))
}
