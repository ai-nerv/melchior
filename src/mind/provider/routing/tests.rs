use super::*;
use serde_json::json;

fn provider() -> Provider {
    serde_json::from_value(json!({"id":"fixture", "name":"Fixture",
        "api":"openai-completions", "base_url":"https://example.test/v1",
        "auth":{"kind":"api-key", "vars":["FIXTURE_KEY"]},
        "models":[{"id":"m", "name":"M", "context_window":8192, "max_tokens":128}]
    }))
    .expect("provider")
}

#[test]
fn endpoints_inherit_and_overrides_require_exact_origin_binding() {
    let mut provider = provider();
    let mut model = provider.models[0].clone();
    assert_eq!(
        base(&provider, &model).expect("default"),
        "https://example.test/v1"
    );
    model.base_url = Some("https://EXAMPLE.test:443/v2".into());
    assert!(
        destination(
            &provider,
            &model,
            base(&provider, &model).expect("override")
        )
        .is_ok()
    );
    for target in [
        "https://other.test/v1",
        "https://example.test:8443/v1",
        "http://localhost/v1",
    ] {
        model.base_url = Some(target.into());
        assert_eq!(
            destination(&provider, &model, target),
            Err("credential_origins")
        );
    }
    provider
        .credential_origins
        .push("https://other.test".into());
    assert!(destination(&provider, &model, "https://other.test/v1/chat/completions").is_ok());
    assert_eq!(
        destination(&provider, &model, "https://other.test.attacker.invalid/v1"),
        Err("credential_origins")
    );
    provider.base_url = None;
    model.base_url = Some("https://other.test/v1".into());
    assert!(
        destination(
            &provider,
            &model,
            base(&provider, &model).expect("model-only endpoint")
        )
        .is_ok()
    );
    provider.credential_origins.clear();
    assert_eq!(
        destination(&provider, &model, "https://other.test/v1"),
        Err("credential_origins")
    );
}

#[test]
fn binding_and_endpoint_errors_never_echo_url_secrets() {
    let mut provider = provider();
    let model = provider.models[0].clone();
    for target in [
        "https://private-sentinel@example.test/v1",
        "https://example.test/v1#private-sentinel",
        "http://remote.invalid/?key=private-sentinel",
    ] {
        let error = destination(&provider, &model, target).expect_err("invalid endpoint");
        assert!(!error.contains("private-sentinel"));
    }
    for binding in [
        "https://*.example.test",
        "https://example.test/path",
        "https://example.test?private-sentinel",
        "https://private-sentinel@example.test",
        "http://remote.invalid",
    ] {
        provider.credential_origins = vec![binding.into()];
        assert!(validate_provider(&provider).is_err(), "{binding}");
    }
}

#[test]
fn headers_merge_case_insensitively_without_authentication_overrides() {
    let mut provider = provider();
    let mut model = provider.models[0].clone();
    provider.headers.insert("X-Route".into(), "provider".into());
    provider.headers.insert("X-Provider".into(), "p".into());
    model.headers.insert("x-ROUTE".into(), "model".into());
    let defaults = vec![
        ("x-route".into(), "protocol".into()),
        ("authorization".into(), "Bearer private-sentinel".into()),
    ];
    let merged = headers(&provider, &model, defaults).expect("merged");
    assert_eq!(merged["x-route"], "model");
    assert_eq!(merged["x-provider"], "p");
    assert_eq!(merged["authorization"], "Bearer private-sentinel");
    assert!(merged["authorization"].is_sensitive());
    assert!(merged["x-route"].is_sensitive());
    assert!(!format!("{merged:?}").contains("private-sentinel"));
    for name in [
        "Authorization",
        "Proxy-Authorization",
        "X-API-KEY",
        "api-key",
        "x-goog-api-key",
        "x-amz-date",
        "x-amz-security-token",
        "Host",
        "Content-Length",
        "Transfer-Encoding",
        "Cookie",
        "content-type",
    ] {
        model.headers.clear();
        model.headers.insert(name.into(), "private-sentinel".into());
        assert_eq!(validate(&provider, &model), Err("model.headers"));
        let error = headers(&provider, &model, Vec::new()).expect_err("protected header");
        assert!(!error.contains("private-sentinel"));
    }
}

#[test]
fn header_duplicates_controls_and_resource_excesses_are_refused() {
    let mut provider = provider();
    let mut model = provider.models[0].clone();
    provider.headers = BTreeMap::from([
        ("X-Test".into(), "one".into()),
        ("x-test".into(), "two".into()),
    ]);
    assert_eq!(validate_provider(&provider), Err("headers"));
    provider.headers.clear();
    for (name, value) in [
        ("bad name", "private-sentinel"),
        ("x-test", "private-sentinel\r\nHeader: injected"),
    ] {
        model.headers = BTreeMap::from([(name.into(), value.into())]);
        assert_eq!(validate(&provider, &model), Err("model.headers"));
    }
    model.headers = BTreeMap::from([("x-test".into(), "a".repeat(HEADER_BYTES))]);
    assert!(headers(&provider, &model, Vec::new()).is_err());
    model.headers = (0..=HEADER_COUNT)
        .map(|i| (format!("x-{i}"), "v".into()))
        .collect();
    assert!(headers(&provider, &model, Vec::new()).is_err());
}
