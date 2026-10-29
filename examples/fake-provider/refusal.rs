use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Shape {
    #[default]
    Message,
    Anthropic,
    Vllm,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Refusal {
    pub status: u16,
    message: String,
    #[serde(default)]
    shape: Shape,
}

impl Refusal {
    pub fn body(&self) -> Value {
        match self.shape {
            Shape::Message => json!({"error":{"message":self.message}}),
            Shape::Anthropic => json!({"type":"error","error":{
                "type":"invalid_request_error","message":self.message}}),
            Shape::Vllm => json!({"error":{"type":"BadRequestError","code":self.status,
                "param":null,"message":self.message}}),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusal_shapes_are_exact_and_legacy_is_unchanged() {
        for (shape, expected) in [
            (None, json!({"error":{"message":"exact\r\n"}})),
            (
                Some("anthropic"),
                json!({"type":"error","error":{
                "type":"invalid_request_error","message":"exact\r\n"}}),
            ),
            (
                Some("vllm"),
                json!({"error":{"type":"BadRequestError","code":400,
                "param":null,"message":"exact\r\n"}}),
            ),
        ] {
            let mut value = json!({"status":400,"message":"exact\r\n"});
            if let Some(shape) = shape {
                value["shape"] = json!(shape);
            }
            let refusal: Refusal = serde_json::from_value(value).expect("refusal");
            assert_eq!(refusal.body(), expected);
        }
        assert!(
            serde_json::from_value::<Refusal>(json!({"status":400,"message":"x","shape":"future"}))
                .is_err()
        );
        assert!(
            serde_json::from_value::<Refusal>(json!({"status":400,"message":"x","shpae":"vllm"}))
                .is_err()
        );
    }
}
