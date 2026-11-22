use super::*;
use crate::mind::{
    lua::{adapter::LuaAdapter, engine::Engine},
    model::Context,
    provider::{api::Adapter, compat::Compat, endpoint::Provider},
};
use serde_json::json;

fn provider() -> Provider {
    serde_json::from_value(json!({"id":"p","name":"P","api":"openai-completions","base_url":"https://fixture.invalid/v1","auth":{"kind":"none"},"models":[{"id":"m","name":"M","context_window":8192,"max_tokens":128}]})).expect("provider")
}

fn adapter() -> LuaAdapter {
    let mut engine = Engine::new();
    engine
        .run(include_str!("../../../../../config/apis.lua"), "apis")
        .expect("APIs");
    LuaAdapter::new(engine, "openai-completions").expect("Chat")
}

#[test]
fn input_bounds_are_utf8_bytes_control_free_and_do_not_echo_the_identifier() {
    for (id, valid) in [
        (None, true),
        (Some("a".repeat(256)), true),
        (Some("é".repeat(128)), true),
        (Some("a".repeat(257)), false),
        (Some("é".repeat(129)), false),
        (Some(String::new()), false),
        (Some("private-sentinel\r\nheader".into()), false),
    ] {
        let options = Options {
            session_id: id,
            ..Options::default()
        };
        let result = validate(&options);
        assert_eq!(result.is_ok(), valid);
        if let Err(message) = result {
            assert!(!message.contains("private-sentinel"));
        }
    }
}

#[test]
fn mistral_affinity_is_sensitive_and_preserves_explicit_header_overrides() {
    let mut model = provider().models[0].clone();
    model.api = Api::MistralConversations;
    model.compat = Some(Compat {
        send_session_affinity_headers: Some(true),
        ..Default::default()
    });
    let mut actual = HeaderMap::new();
    headers(&model, Some("bound"), &mut actual).expect("headers");
    assert_eq!(actual.len(), 1);
    assert_eq!(actual["x-affinity"], "bound");
    assert!(actual["x-affinity"].is_sensitive());
    actual.insert("x-affinity", HeaderValue::from_static("explicit"));
    headers(&model, Some("bound"), &mut actual).expect("headers");
    assert_eq!(actual["x-affinity"], "explicit");
}

#[test]
fn scoped_identity_is_stable_and_changes_for_sessions_models_accounts_endpoints_and_providers() {
    let adapter = adapter();
    let provider = provider();
    let mut model = provider.models[0].clone();
    model.compat = Some(Compat {
        supports_prompt_cache_key: Some(true),
        ..Compat::default()
    });
    let context = Context::default();
    let options = Options {
        session_id: Some("private-session".into()),
        ..Options::default()
    };
    let call = Call {
        adapter: &adapter,
        provider: &provider,
        model: &model,
        context: &context,
        options: &options,
    };
    let expected = bound(
        &call,
        &options,
        "account-a",
        "https://fixture.invalid/v1/chat/completions",
    )
    .expect("key");
    assert_eq!(expected.len(), 64);
    assert!(!expected.contains("private"));
    assert_eq!(
        bound(
            &call,
            &options,
            "account-a",
            "https://fixture.invalid/v1/chat/completions"
        ),
        Some(expected.clone())
    );
    assert_ne!(
        bound(
            &call,
            &options,
            "account-b",
            "https://fixture.invalid/v1/chat/completions"
        ),
        Some(expected.clone())
    );
    assert_ne!(
        bound(
            &call,
            &options,
            "account-a",
            "https://fixture.invalid/v2/chat/completions"
        ),
        Some(expected.clone())
    );
    let changed = Options {
        session_id: Some("another-session".into()),
        ..options.clone()
    };
    assert_ne!(
        bound(
            &call,
            &changed,
            "account-a",
            "https://fixture.invalid/v1/chat/completions"
        ),
        Some(expected.clone())
    );
    let mut other_model = model.clone();
    other_model.id = "n".into();
    let mut other_provider = provider.clone();
    other_provider.id = "q".into();
    for (provider, model) in [(&provider, &other_model), (&other_provider, &model)] {
        let call = Call {
            provider,
            model,
            ..call
        };
        assert_ne!(
            bound(
                &call,
                &options,
                "account-a",
                "https://fixture.invalid/v1/chat/completions"
            ),
            Some(expected.clone())
        );
    }
}

#[test]
fn absent_disabled_and_other_protocols_never_gain_session_headers() {
    for api in Api::all() {
        for enabled in [false, true] {
            for present in [false, true] {
                let mut model = provider().models[0].clone();
                model.api = api;
                model.compat = Some(Compat {
                    send_session_affinity_headers: Some(enabled),
                    ..Compat::default()
                });
                let mut actual = HeaderMap::new();
                headers(&model, present.then_some("opaque"), &mut actual).expect("headers");
                assert_eq!(
                    actual.len(),
                    if enabled && present {
                        match api {
                            Api::OpenAiCompletions => 2,
                            Api::AnthropicMessages | Api::MistralConversations => 1,
                            _ => 0,
                        }
                    } else {
                        0
                    }
                );
            }
        }
    }
}

#[test]
fn each_header_dialect_is_exact_sensitive_and_bounded_after_merging() {
    for api in [Api::OpenAiCompletions, Api::AnthropicMessages] {
        for (format, names) in [
            (
                SessionAffinityFormat::Generic,
                vec!["x-client-request-id", "x-session-affinity"],
            ),
            (
                SessionAffinityFormat::OpenAi,
                vec!["session_id", "x-client-request-id", "x-session-affinity"],
            ),
            (SessionAffinityFormat::OpenRouter, vec!["x-session-id"]),
        ] {
            let mut model = provider().models[0].clone();
            model.api = api;
            let names =
                if api == Api::AnthropicMessages && format != SessionAffinityFormat::OpenRouter {
                    vec!["x-session-affinity"]
                } else {
                    names
                };
            model.compat = Some(Compat {
                send_session_affinity_headers: Some(true),
                session_affinity_format: Some(format),
                ..Compat::default()
            });
            let mut actual = HeaderMap::new();
            headers(&model, Some("private-sentinel"), &mut actual).expect("headers");
            assert_eq!(actual.len(), names.len());
            for name in names {
                assert_eq!(actual[name], "private-sentinel");
                assert!(actual[name].is_sensitive());
            }
            assert!(!format!("{actual:?}").contains("private-sentinel"));
            let mut crowded = HeaderMap::new();
            for index in 0..128 {
                crowded.insert(
                    reqwest::header::HeaderName::from_bytes(format!("x-extra-{index}").as_bytes())
                        .expect("name"),
                    HeaderValue::from_static("v"),
                );
            }
            assert!(headers(&model, Some("opaque"), &mut crowded).is_err());
            assert!(headers(&model, Some("bad\r\nvalue"), &mut actual).is_err());
        }
    }
}

#[test]
fn protocol_scopes_are_distinct_and_body_cache_opt_in_is_chat_only() {
    let adapter = adapter();
    let provider = provider();
    let context = Context::default();
    let options = Options {
        session_id: Some("same-session".into()),
        ..Default::default()
    };
    let mut keys = std::collections::BTreeSet::new();
    for api in Api::all() {
        for affinity in [false, true] {
            let mut model = provider.models[0].clone();
            model.api = api;
            model.compat = Some(Compat {
                send_session_affinity_headers: Some(affinity),
                supports_prompt_cache_key: Some(true),
                ..Default::default()
            });
            let call = Call {
                adapter: &adapter,
                provider: &provider,
                model: &model,
                context: &context,
                options: &options,
            };
            let key = bound(&call, &options, "account", "https://fixture.invalid/shared");
            assert_eq!(
                key.is_some(),
                matches!(api, Api::OpenAiCompletions | Api::MistralConversations)
                    || (api == Api::AnthropicMessages && affinity)
            );
            if affinity && let Some(key) = key {
                assert!(keys.insert(key));
            }
        }
    }
    assert_eq!(keys.len(), 3);
}

#[test]
fn explicit_false_and_dialect_overrides_preserve_independent_inherited_settings() {
    let base = Some(Compat {
        send_session_affinity_headers: Some(true),
        supports_prompt_cache_key: Some(true),
        session_affinity_format: Some(SessionAffinityFormat::OpenAi),
        ..Compat::default()
    });
    for flag in [None, Some(false), Some(true)] {
        let compat = Compat {
            send_session_affinity_headers: flag,
            supports_prompt_cache_key: flag,
            ..Compat::default()
        }
        .over(base);
        assert_eq!(
            compat.resolve().send_session_affinity_headers,
            flag.unwrap_or(true)
        );
        assert_eq!(
            compat.resolve().supports_prompt_cache_key,
            flag.unwrap_or(true)
        );
        assert_eq!(
            compat.resolve().session_affinity_format,
            SessionAffinityFormat::OpenAi
        );
        assert_eq!(
            serde_json::from_value::<Compat>(serde_json::to_value(compat).expect("JSON"))
                .expect("compat"),
            compat
        );
    }
    assert!(!Compat::default().resolve().send_session_affinity_headers);
    assert!(!Compat::default().resolve().supports_prompt_cache_key);
}

#[test]
fn only_bound_private_options_reach_the_lua_request_and_callers_cannot_forge_them() {
    let wire = adapter();
    let mut model = provider().models[0].clone();
    model.compat = Some(Compat {
        supports_prompt_cache_key: Some(true),
        ..Compat::default()
    });
    let mut options: Options = serde_json::from_value(
        json!({"session_id":"private-session","provider_session":"forged","__session_id":"forged"}),
    )
    .expect("options");
    assert!(options.provider_session.is_none());
    let first = wire.request(&model, &Context::default(), &options);
    assert!(first.get("prompt_cache_key").is_none());
    assert!(!first.to_string().contains("private-session"));
    options.provider_session = Some("a".repeat(64));
    let second = wire.request(&model, &Context::default(), &options);
    assert_eq!(second["prompt_cache_key"], "a".repeat(64));
    assert!(!second.to_string().contains("private-session"));
    assert!(
        serde_json::to_value(options)
            .expect("JSON")
            .get("provider_session")
            .is_none()
    );
}
