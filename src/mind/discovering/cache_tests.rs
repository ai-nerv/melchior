use super::{cached_models, joined, parse};
use crate::mind::provider::compat::{PromptCache, resolve};
use crate::mind::provider::endpoint::Provider;
use serde_json::json;

#[test]
fn raw_discovery_cache_does_not_persist_inherited_cache_capability() {
    let fetched = parse(&json!({"data":[{"id":"test","name":"Test"}]}));
    assert_eq!(fetched.len(), 1);
    assert!(fetched.first().expect("fetched model").compat.is_none());
    let stored = serde_json::to_string(&fetched).expect("raw cache encoding");
    assert!(!stored.contains("prompt_cache"));
    for declaration in [
        json!(null),
        json!({"prompt_cache":"none"}),
        json!({"prompt_cache":"anthropic_ephemeral"}),
    ] {
        let provider: Provider = serde_json::from_value(json!({"id":"test","name":"Test",
            "base_url":"https://declared.invalid","api":"anthropic-messages","auth":{"kind":"none"},
            "compat":declaration}))
        .expect("provider");
        let reloaded = cached_models(&stored)
            .expect("actual cache reader")
            .remove(0);
        let model = joined(reloaded, &provider);
        assert_eq!(
            resolve(model.compat).prompt_cache,
            resolve(provider.compat).prompt_cache
        );
    }
}

#[test]
fn an_explicit_cached_model_disable_still_overrides_the_current_provider() {
    let provider: Provider = serde_json::from_value(json!({"id":"test","name":"Test",
        "api":"anthropic-messages","auth":{"kind":"none"},"compat":{"prompt_cache":"anthropic_ephemeral"}})).expect("provider");
    let stored = json!([{"id":"test","name":"Test","context_window":8000,"max_tokens":900,
        "compat":{"prompt_cache":"none"}}])
    .to_string();
    let model = joined(cached_models(&stored).expect("cache").remove(0), &provider);
    assert_eq!(resolve(model.compat).prompt_cache, PromptCache::None);
}
