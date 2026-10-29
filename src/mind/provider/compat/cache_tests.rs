use super::{Compat, PromptCache, resolve};
use serde_json::json;

#[test]
fn undeclared_null_and_empty_cache_capabilities_resolve_off() {
    assert_eq!(resolve(None).prompt_cache, PromptCache::None);
    for declaration in [json!({}), json!({"prompt_cache":null})] {
        let compat: Compat = serde_json::from_value(declaration).expect("compat");
        assert_eq!(compat.prompt_cache, None);
        assert_eq!(resolve(Some(compat)).prompt_cache, PromptCache::None);
        assert!(
            serde_json::to_value(compat)
                .expect("encode")
                .get("prompt_cache")
                .is_none()
        );
    }
}

#[test]
fn every_provider_model_cache_override_combination_is_fieldwise() {
    let modes = [
        None,
        Some(PromptCache::None),
        Some(PromptCache::AnthropicEphemeral),
    ];
    for provider_mode in modes {
        for model_mode in modes {
            let provider = Compat {
                prompt_cache: provider_mode,
                supports_store: Some(true),
                ..Compat::default()
            };
            let model = Compat {
                prompt_cache: model_mode,
                ..Compat::default()
            };
            let layered = model.over(Some(provider));
            assert_eq!(layered.prompt_cache, model_mode.or(provider_mode));
            assert_eq!(
                layered.resolve().prompt_cache,
                model_mode.or(provider_mode).unwrap_or_default()
            );
            assert!(layered.resolve().supports_store);
        }
    }
}

#[test]
fn cache_modes_have_explicit_names_and_unknown_modes_refuse() {
    for (name, mode) in [
        ("none", PromptCache::None),
        ("anthropic_ephemeral", PromptCache::AnthropicEphemeral),
    ] {
        let value = json!({"prompt_cache":name});
        let compat: Compat = serde_json::from_value(value.clone()).expect("mode");
        assert_eq!(compat.prompt_cache, Some(mode));
        assert_eq!(serde_json::to_value(compat).expect("encoded"), value);
        assert_eq!(
            serde_json::to_value(compat.resolve()).expect("resolved")["prompt_cache"],
            name
        );
    }
    for mode in [
        json!(true),
        json!("automatic"),
        json!("anthropic-ephemeral"),
        json!({"ttl":"1h"}),
    ] {
        assert!(serde_json::from_value::<Compat>(json!({"prompt_cache":mode})).is_err());
    }
}
