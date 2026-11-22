use super::*;

fn source(api: &str, flag: &str, enabled: bool) -> Value {
    json!({"id":"one","provider":"fixture","api":api,"type":"chat","name":"One",
        "baseUrl":"https://fixture.invalid/v1","reasoning":false,"input":["text"],
        "contextWindow":8192,"maxTokens":1024,"cost":{"input":1,"output":2},"compat":{flag:enabled}})
}

#[test]
fn strict_compatibility_preserves_opt_out_and_reports_missing_adapters() {
    for enabled in [false, true] {
        let record =
            compiler::convert(&source("openai-completions", "supportsStrictMode", enabled))
                .expect("OpenAI");
        assert_eq!(
            record["candidate"]["compat"]["supports_strict_mode"],
            enabled
        );
        assert_eq!(record["candidate"]["capabilities"]["strict_tools"], enabled);
        assert_eq!(record["pending"], json!([]));
    }
    let record = compiler::convert(&source(
        "bedrock-converse-stream",
        "supportsStrictMode",
        true,
    ))
    .expect("unsupported adapter");
    assert_eq!(record["candidate"]["compat"]["supports_strict_mode"], true);
    assert_eq!(record["candidate"]["capabilities"]["strict_tools"], false);
    assert!(
        record["pending"]
            .as_array()
            .expect("pending")
            .contains(&json!("compat.supportsStrictMode:adapter"))
    );
    let record = compiler::convert(&source("anthropic-messages", "supportsStrictTools", true))
        .expect("Anthropic");
    assert_eq!(record["candidate"]["capabilities"]["strict_tools"], true);
    assert_eq!(record["pending"], json!([]));
}
