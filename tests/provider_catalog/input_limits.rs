use super::*;

fn raw(limits: Value) -> Value {
    json!({"id":"m","name":"M","provider":"fixture","api":"openai-completions",
        "type":"chat","contextWindow":8192,"maxTokens":1024,"input":["text","image"],
        "inputLimits":limits})
}

#[test]
fn qualified_limits_are_mapped_and_unsupported_adapter_limits_remain_pending() {
    let record = compiler::convert(&raw(json!({"maxRequestBytes":1024,"images":{
        "maxPerMessage":20,"maxPerRequest":100,"resize":{"maxWidth":2000,"maxHeight":1500,
        "maxBytes":4718592,"jpegQuality":80}}})))
    .expect("converted");
    assert_eq!(
        record["candidate"]["input_limits"],
        json!({"max_request_bytes":1024,"images":{
        "max_per_message":20,"max_per_request":100,"resize":{"max_width":2000,"max_height":1500,
        "max_bytes":4718592,"jpeg_quality":80}}})
    );
    assert_eq!(record["pending"], json!([]));
    let record = compiler::convert(&raw(
        json!({"maxRequestBytes":1024,"images":{"maxPerMessage":20}}),
    ))
    .expect("counts");
    assert_eq!(record["pending"], json!([]));
    let mut bedrock =
        raw(json!({"images":{"maxPerRequest":20,"resize":{"maxWidth":2000,"maxBytes":1000}}}));
    bedrock["api"] = json!("bedrock-converse-stream");
    assert_eq!(
        compiler::convert(&bedrock).expect("bedrock")["pending"],
        json!(["inputLimits.images:adapter"])
    );
}

#[test]
fn unknown_limit_paths_stay_pending_and_invalid_known_values_refuse_generation() {
    let record = compiler::convert(&raw(
        json!({"future":1,"images":{"future":2,"resize":{"future":3}}}),
    ))
    .expect("unknowns");
    for path in [
        "inputLimits.future",
        "inputLimits.images.future",
        "inputLimits.images.resize.future",
    ] {
        assert!(
            record["pending"]
                .as_array()
                .expect("pending")
                .contains(&json!(path))
        );
    }
    for value in [
        json!({"maxRequestBytes":0}),
        json!({"maxRequestBytes":null}),
        json!({"images":{"maxPerRequest":-1}}),
        json!({"images":{"resize":{"jpegQuality":101}}}),
        json!({"images":false}),
    ] {
        assert!(compiler::convert(&raw(value)).is_err());
    }
}

#[test]
fn every_pinned_input_limit_value_survives_native_conversion() {
    let manifest: Value = serde_json::from_str(MANIFEST).expect("manifest");
    let source = compiler::input::read(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("catalog/reference/pi-1.6.0/data"),
        &manifest,
    )
    .expect("pinned sources");
    let mut count = 0;
    for row in source
        .values()
        .flatten()
        .filter(|row| row["type"] != "image")
    {
        let Some(limits) = row.get("inputLimits") else {
            continue;
        };
        let record = compiler::convert(row).expect("conversion");
        let native = &record["candidate"]["input_limits"];
        for (upstream, mapped) in [
            ("/maxRequestBytes", "/max_request_bytes"),
            ("/images/maxPerMessage", "/images/max_per_message"),
            ("/images/maxPerRequest", "/images/max_per_request"),
            ("/images/resize/maxWidth", "/images/resize/max_width"),
            ("/images/resize/maxHeight", "/images/resize/max_height"),
            ("/images/resize/maxBytes", "/images/resize/max_bytes"),
            ("/images/resize/jpegQuality", "/images/resize/jpeg_quality"),
        ] {
            assert_eq!(
                limits.pointer(upstream),
                native.pointer(mapped),
                "{} {upstream}",
                row["id"]
            );
        }
        assert!(
            !record["pending"]
                .as_array()
                .expect("pending")
                .contains(&json!("inputLimits.images.resize:history"))
        );
        count += 1;
    }
    assert_eq!(count, 1290);
}
