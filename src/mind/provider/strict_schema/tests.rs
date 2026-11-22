use super::*;

fn object(property: Value, required: bool) -> Value {
    json!({"type": "object", "properties": {"value": property},
        "required": if required { vec!["value"] } else { vec![] }})
}

#[test]
fn closes_nested_objects_without_mutating_the_original_schema() {
    let input = json!({"type": "object", "description": "outer", "properties": {
        "name": {"type": "string"},
        "options": {"type": "object", "properties": {"count": {"type": "integer"}}},
        "items": {"type": "array", "items": {"type": "object", "properties": {}}}
    }, "required": ["name", "items"]});
    let before = input.clone();
    let result = normalize(&input, Dialect::OpenAi).expect("supported schema");
    assert_eq!(input, before);
    assert_eq!(result["additionalProperties"], false);
    assert_eq!(result["required"], json!(["name", "options", "items"]));
    assert_eq!(result["description"], "outer");
    assert_eq!(result["properties"]["name"], json!({"type": "string"}));
    let nested = &result["properties"]["options"]["anyOf"][0];
    assert_eq!(nested["additionalProperties"], false);
    assert_eq!(nested["required"], json!(["count"]));
    assert_eq!(
        nested["properties"]["count"],
        json!({"anyOf": [{"type": "integer"}, {"type": "null"}]})
    );
    assert_eq!(
        result["properties"]["items"]["items"]["additionalProperties"],
        false
    );
}

#[test]
fn preserves_existing_nulls_and_required_fields() {
    for property in [
        json!({"type": ["string", "null"]}),
        json!({"type": "null"}),
        json!({"enum": ["a", null]}),
        json!({"type": "string", "const": null}),
        json!({"anyOf": [{"type": "string"}, {"type": "null"}]}),
    ] {
        let output =
            normalize(&object(property.clone(), false), Dialect::OpenAi).expect("nullable");
        assert_eq!(output["properties"]["value"], property);
    }
    let output = normalize(&object(json!({"enum": [1, 2]}), true), Dialect::OpenAi).expect("enum");
    assert_eq!(output["properties"]["value"], json!({"enum": [1, 2]}));
}

#[test]
fn treats_property_names_and_annotation_values_as_data() {
    let input = json!({"type": "object", "properties": {
        "$ref": {"type": "string", "default": "private-sentinel"},
        "options": {"type": "string", "examples": [{"$ref": "literal", "anyOf": false}]}
    }, "required": ["$ref", "options"]});
    let output = normalize(&input, Dialect::OpenAi).expect("annotation data");
    assert_eq!(output["properties"], input["properties"]);
    let error = normalize(
        &object(json!({"$ref": "private-sentinel"}), true),
        Dialect::OpenAi,
    )
    .expect_err("unsupported reference");
    assert!(!error.contains("private-sentinel"));
}

#[test]
fn refuses_unsupported_keywords_at_every_schema_position() {
    for keyword in UNSUPPORTED {
        let property = json!({"type": "string", *keyword: {}});
        for input in [
            object(property.clone(), true),
            object(json!({"type": "array", "items": property.clone()}), true),
            object(json!({"anyOf": [property]}), true),
        ] {
            assert!(normalize(&input, Dialect::OpenAi).is_err(), "{keyword}");
        }
    }
}

#[test]
fn refuses_malformed_and_nonportable_schema_shapes() {
    for input in [
        json!(true),
        json!({}),
        json!({"type": "array"}),
        json!({"type": "object", "properties": []}),
        json!({"type": "object", "required": "x"}),
        json!({"type": "object", "required": [42]}),
        json!({"type": "object", "required": ["missing"]}),
        json!({"type": "object", "additionalProperties": true}),
        json!({"type": "object", "additionalProperties": {"type": "string"}}),
        object(json!(false), true),
        object(json!({"const": "a"}), true),
        object(json!({"anyOf": []}), true),
        object(json!({"anyOf": {}}), true),
        object(json!({"anyOf": [{"type": "object"}]}), true),
        object(json!({"anyOf": [{"type": ["array", "null"]}]}), true),
        object(
            json!({"type": "array", "items": [{"type": "string"}]}),
            true,
        ),
        object(json!({"type": "string", "properties": {}}), true),
    ] {
        assert!(normalize(&input, Dialect::OpenAi).is_err(), "{input}");
    }
}

#[test]
fn applies_anthropic_constraints_without_changing_openai_support() {
    for property in [
        json!({"type": "integer", "minimum": 0}),
        json!({"type": "string", "format": "regex"}),
        json!({"type": "array", "items": {"type": "integer"}, "minItems": 2}),
        json!({"type": "array", "items": {"type": "integer"}, "maxItems": 1}),
    ] {
        let schema = object(property, true);
        assert!(normalize(&schema, Dialect::OpenAi).is_ok());
        assert!(normalize(&schema, Dialect::Anthropic).is_err());
    }
    for property in [
        json!({"type": "string", "format": "uuid"}),
        json!({"type": "array", "items": {"type": "integer"}, "minItems": 1}),
    ] {
        assert!(normalize(&object(property, true), Dialect::Anthropic).is_ok());
    }
}

#[test]
fn rejects_excessive_schema_work_before_cloning() {
    let mut deep = json!({"type": "string"});
    for _ in 0..70 {
        deep = object(deep, true);
    }
    assert!(normalize(&deep, Dialect::OpenAi).is_err());
    let wide = json!({"type": "object", "examples": vec![0; 16_385]});
    assert!(normalize(&wide, Dialect::OpenAi).is_err());
    let large = json!({"type": "object", "description": "x".repeat(1_048_577)});
    assert!(normalize(&large, Dialect::OpenAi).is_err());
}

#[test]
fn deepseek_retains_supported_constraints_without_inventing_nullable_optionals() {
    for property in [
        json!({"type":"string","pattern":"^a","format":"email"}),
        json!({"type":"number","minimum":0,"maximum":10,"exclusiveMinimum":-1,
            "exclusiveMaximum":11,"multipleOf":0.5,"default":1,"const":1}),
        json!({"type":"array","items":{"type":"object","properties":{"x":{"type":"boolean"}},"required":["x"]}}),
        json!({"anyOf":[{"type":"string","format":"uuid"},{"type":"integer"}]}),
        json!({"type":"string","enum":["a","b"]}),
    ] {
        let input = object(property, true);
        let saved = input.clone();
        let result = normalize(&input, Dialect::DeepSeek).expect("documented strict schema");
        assert_eq!(result["additionalProperties"], false);
        assert_eq!(input, saved);
        if input["properties"]["value"]["type"] != "array" {
            assert_eq!(result["properties"], input["properties"]);
        } else {
            assert_eq!(
                result["properties"]["value"]["items"]["additionalProperties"],
                false
            );
        }
    }
    assert!(
        normalize(&object(json!({"type":"string"}), false), Dialect::DeepSeek)
            .expect_err("optional property")
            .contains("every property")
    );
}
