use serde_json::{Map, Value, json};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Dialect {
    OpenAi,
    Anthropic,
    DeepSeek,
}

const UNSUPPORTED: &[&str] = &[
    "$ref",
    "$defs",
    "definitions",
    "allOf",
    "oneOf",
    "patternProperties",
    "dependentSchemas",
    "dependencies",
    "unevaluatedProperties",
    "propertyNames",
    "contains",
    "prefixItems",
    "not",
    "if",
    "then",
    "else",
];

pub(crate) fn normalize(schema: &Value, dialect: Dialect) -> Result<Value, &'static str> {
    bounded(schema)?;
    if schema.get("type").and_then(Value::as_str) != Some("object") {
        return Err("root schema must have type object");
    }
    let mut result = schema.clone();
    node(&mut result, dialect)?;
    Ok(result)
}

fn bounded(schema: &Value) -> Result<(), &'static str> {
    let mut pending = vec![(schema, 0)];
    let mut nodes = 0;
    let mut bytes = 0_usize;
    while let Some((value, depth)) = pending.pop() {
        nodes += 1;
        if depth > 64 || nodes > 16_384 {
            return Err("strict schema exceeds depth or node limits");
        }
        match value {
            Value::Object(fields) => {
                if pending.len().saturating_add(fields.len()) > 16_384 {
                    return Err("strict schema exceeds node limits");
                }
                for (name, child) in fields {
                    bytes = bytes.saturating_add(name.len());
                    pending.push((child, depth + 1));
                }
            }
            Value::Array(items) => {
                if pending.len().saturating_add(items.len()) > 16_384 {
                    return Err("strict schema exceeds node limits");
                }
                pending.extend(items.iter().map(|v| (v, depth + 1)));
            }
            Value::String(text) => bytes = bytes.saturating_add(text.len()),
            _ => {}
        }
        if pending.len() > 16_384 || bytes > 1_048_576 {
            return Err("strict schema exceeds size limits");
        }
    }
    Ok(())
}

fn node(schema: &mut Value, dialect: Dialect) -> Result<(), &'static str> {
    let fields = schema
        .as_object_mut()
        .ok_or("boolean schemas are unsupported")?;
    if UNSUPPORTED.iter().any(|key| fields.contains_key(*key)) {
        return Err("unsupported strict schema keyword");
    }
    if dialect == Dialect::Anthropic && fields.iter().any(|(key, v)| anthropic_rejects(key, v)) {
        return Err("unsupported Anthropic strict schema constraint");
    }
    if dialect == Dialect::DeepSeek {
        deepseek::validate(fields)?;
    }
    if !fields.contains_key("type") && !fields.contains_key("anyOf") && !fields.contains_key("enum")
    {
        return Err("schemas without a type are unsupported");
    }
    if let Some(union) = fields.get_mut("anyOf") {
        let variants = union
            .as_array_mut()
            .filter(|v| !v.is_empty())
            .ok_or("anyOf must contain at least one schema")?;
        for variant in variants {
            if structured(variant) {
                return Err("object and array unions are unsupported");
            }
            node(variant, dialect)?;
        }
    }
    if let Some(items) = fields.get_mut("items") {
        if items.is_array() {
            return Err("tuple schemas are unsupported");
        }
        node(items, dialect)?;
    }
    let object = fields.get("type").and_then(Value::as_str) == Some("object");
    if fields.contains_key("properties") && !object {
        return Err("properties require type object");
    }
    if object {
        object_node(fields, dialect)?;
    }
    Ok(())
}

fn object_node(fields: &mut Map<String, Value>, dialect: Dialect) -> Result<(), &'static str> {
    if fields
        .get("additionalProperties")
        .is_some_and(|v| v != &Value::Bool(false))
    {
        return Err("schema-valued or true additionalProperties is unsupported");
    }
    let required = match fields.get("required") {
        None => BTreeSet::new(),
        Some(Value::Array(names)) if names.iter().all(Value::is_string) => names
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        _ => return Err("object required must be a string array"),
    };
    let properties = fields
        .entry("properties")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("object properties must be a schema map")?;
    if required.iter().any(|name| !properties.contains_key(name)) {
        return Err("required contains an unknown property");
    }
    for (name, property) in properties.iter_mut() {
        node(property, dialect)?;
        if dialect == Dialect::DeepSeek && !required.contains(name) {
            return Err("DeepSeek strict schemas require every property explicitly");
        }
        if !required.contains(name) && !allows_null(property) {
            *property = json!({"anyOf": [property.take(), {"type": "null"}]});
        }
    }
    let names: Vec<_> = properties.keys().cloned().collect();
    fields.insert("required".into(), json!(names));
    fields.insert("additionalProperties".into(), Value::Bool(false));
    Ok(())
}

mod deepseek;

fn structured(schema: &Value) -> bool {
    let kind = schema.get("type");
    kind.is_some_and(|v| {
        v == "object"
            || v == "array"
            || v.as_array()
                .is_some_and(|types| types.iter().any(|t| t == "object" || t == "array"))
    }) || schema.get("properties").is_some()
        || schema.get("items").is_some()
}

fn allows_null(schema: &Value) -> bool {
    schema.get("type").is_some_and(|v| {
        v == "null"
            || v.as_array()
                .is_some_and(|types| types.contains(&json!("null")))
    }) || schema.get("const").is_some_and(Value::is_null)
        || schema
            .get("enum")
            .and_then(Value::as_array)
            .is_some_and(|v| v.contains(&Value::Null))
        || schema
            .get("anyOf")
            .and_then(Value::as_array)
            .is_some_and(|v| v.iter().any(allows_null))
}

fn anthropic_rejects(key: &str, value: &Value) -> bool {
    match key {
        "minimum" | "maximum" | "exclusiveMinimum" | "exclusiveMaximum" | "multipleOf"
        | "maxItems" | "uniqueItems" | "minContains" | "maxContains" | "minProperties"
        | "maxProperties" => true,
        "minItems" => value.as_u64().is_none_or(|n| n > 1),
        "format" => !matches!(
            value.as_str(),
            Some(
                "date-time"
                    | "time"
                    | "date"
                    | "duration"
                    | "email"
                    | "hostname"
                    | "uri"
                    | "ipv4"
                    | "ipv6"
                    | "uuid"
            )
        ),
        _ => false,
    }
}

#[cfg(test)]
#[path = "strict_schema/tests.rs"]
mod tests;
