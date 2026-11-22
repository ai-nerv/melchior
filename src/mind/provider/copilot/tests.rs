use super::*;
use crate::mind::model::{Context, Message};
use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::json;

fn provider() -> Provider {
    serde_json::from_value(json!({
        "id":"fixture", "name":"Copilot", "api":"openai-completions",
        "base_url":"https://api.individual.githubcopilot.com", "auth":{"kind":"github-copilot"}
    }))
    .expect("provider")
}

#[test]
fn proxy_endpoints_are_subscription_bound_and_never_arbitrary_destinations() {
    let provider = provider();
    for tier in ["individual", "business", "enterprise"] {
        assert_eq!(
            endpoint(
                &provider,
                &format!("token;proxy-ep=proxy.{tier}.githubcopilot.com;")
            )
            .expect("endpoint"),
            format!("https://api.{tier}.githubcopilot.com")
        );
    }
    for host in [
        "proxy.evil.invalid",
        "proxy.individual.githubcopilot.com.evil.invalid",
        "user@api.individual.githubcopilot.com",
        "api.individual.githubcopilot.com:444",
        "api.individual.githubcopilot.com/path",
        "api.individual.githubcopilot.com?key=secret",
        "api.individual.githubcopilot.com#fragment",
    ] {
        assert!(
            endpoint(&provider, &format!("token;proxy-ep={host};")).is_err(),
            "{host}"
        );
    }
    assert!(Config::default().valid());
    assert!(
        !Config {
            oauth_url: "http://foreign.invalid".into(),
            ..Default::default()
        }
        .valid()
    );
}

#[test]
fn copilot_headers_use_sensitive_bearer_and_native_role_and_image_metadata() {
    let context = Context {
        messages: vec![Message::user("hello")],
        ..Default::default()
    };
    let mut output = HeaderMap::new();
    output.insert("x-api-key", HeaderValue::from_static("old-secret"));
    output.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
    headers(&context, &mut output, "private").expect("headers");
    assert!(!output.contains_key("x-api-key"));
    assert!(output.get("authorization").expect("bearer").is_sensitive());
    assert_eq!(
        output.get("authorization").expect("bearer"),
        "Bearer private"
    );
    assert_eq!(output.get("x-initiator").expect("initiator"), "user");
    assert!(!output.contains_key("copilot-vision-request"));
    let context: Context = serde_json::from_value(json!({"messages":[{
        "role":"tool", "content":[{"type":"tool_result", "id":"call", "name":"fixture", "is_error":false, "content":"ok",
        "images":[{"media_type":"image/png", "data":"YQ=="}]}]
    }]}))
    .expect("context");
    headers(&context, &mut output, "private").expect("headers");
    assert_eq!(output.get("x-initiator").expect("initiator"), "agent");
    assert_eq!(
        output.get("copilot-vision-request").expect("vision"),
        "true"
    );
    assert!(headers(&context, &mut output, "secret\ninvalid").is_err());
}

#[test]
fn saved_accounts_require_exact_configuration_and_trusted_metadata() {
    let provider = provider();
    let mut store = oauth::Store::default();
    store.put(
        "fixture",
        oauth::Tokens {
            access: "access".into(),
            refresh: Some("github".into()),
            expires_at: u64::MAX,
            binding: None,
        },
    );
    store.copilot.insert(
        "fixture".into(),
        Account {
            base_url: provider.base_url.clone().expect("base"),
            models: BTreeSet::new(),
        },
    );
    assert!(!signed_in(&store, &provider));
    store
        .configurations
        .insert("fixture".into(), provider.credential_scope());
    assert!(signed_in(&store, &provider));
    store.copilot.get_mut("fixture").expect("account").base_url = "https://foreign.invalid".into();
    assert!(!signed_in(&store, &provider));
}
