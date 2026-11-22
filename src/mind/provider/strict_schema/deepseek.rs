use serde_json::{Map, Value};

pub(super) fn validate(fields: &Map<String, Value>) -> Result<(), &'static str> {
    for (key, value) in fields {
        let supported = match key.as_str() {
            "type" => matches!(
                value.as_str(),
                Some("object" | "string" | "number" | "integer" | "boolean" | "array")
            ),
            "description" | "title" | "pattern" => value.is_string(),
            "format" => matches!(
                value.as_str(),
                Some("email" | "hostname" | "ipv4" | "ipv6" | "uuid")
            ),
            "minimum" | "maximum" | "exclusiveMinimum" | "exclusiveMaximum" | "multipleOf"
            | "const" => value.is_number(),
            "default" => !value.is_null(),
            "enum" => value.as_array().is_some_and(|values| {
                !values.is_empty()
                    && values
                        .iter()
                        .all(|v| v.is_string() || v.is_number() || v.is_boolean())
            }),
            "properties" | "required" | "additionalProperties" | "items" | "anyOf" => true,
            _ => false,
        };
        if !supported {
            return Err("unsupported or malformed DeepSeek strict schema constraint");
        }
    }
    if fields.get("type").and_then(Value::as_str) == Some("array") && !fields.contains_key("items")
    {
        return Err("DeepSeek strict arrays require an item schema");
    }
    Ok(())
}
