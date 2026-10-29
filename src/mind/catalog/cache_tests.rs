use super::Catalog;
use crate::mind::lua::adapter::LuaAdapter;
use crate::mind::model::{Context, Message};
use crate::mind::provider::api::{Adapter, Options};
use crate::mind::provider::compat::{PromptCache, resolve};

fn load(source: &str) -> Catalog {
    let dir = crate::scratch::Scratch::new("melchior-cache-catalog", "declarations");
    std::fs::write(
        dir.join("providers.lua"),
        format!(
            r#"
        for _, id in ipairs({{"openrouter", "ollama"}}) do
            melchior.provider(id, {{name=id, api="openai-completions", auth={{kind="none"}}}})
        end
        {source}
    "#
        ),
    )
    .expect("write fixture declarations");
    Catalog::load(&dir).expect("actual catalog load")
}

fn body(catalog: Catalog, id: &str) -> serde_json::Value {
    let (provider, model) = catalog.find(id).expect("model");
    let (protocol, model) = (provider.api.as_str(), model.clone());
    LuaAdapter::new(catalog.engine, protocol)
        .expect("adapter")
        .request(
            &model,
            &Context {
                system: Some("rules".into()),
                messages: vec![Message::user("hi")],
                ..Context::default()
            },
            &Options::default(),
        )
}

#[test]
fn shipped_cache_capability_is_declared_only_on_the_direct_provider() {
    let catalog = load("");
    for provider in &catalog.providers {
        for model in &provider.models {
            assert_eq!(
                resolve(model.compat).prompt_cache,
                if provider.id == "anthropic" {
                    PromptCache::AnthropicEphemeral
                } else {
                    PromptCache::None
                },
                "{}/{}",
                provider.id,
                model.id
            );
        }
    }
    assert_eq!(
        body(catalog, "anthropic/claude-sonnet-4-5")["system"][0]["cache_control"]["type"],
        "ephemeral"
    );
    assert_eq!(
        body(load(""), "github-copilot/claude-sonnet-4.5")["system"],
        "rules"
    );
}

#[test]
fn overriding_the_builtin_id_with_a_custom_endpoint_does_not_inherit_capability() {
    for url in [
        "https://example.invalid",
        "https://api.anthropic.com.example.invalid",
        "http://localhost:1234",
    ] {
        let catalog = load(&format!(
            r#"
            melchior.provider("anthropic", {{name="Override",api="anthropic-messages",base_url={url:?},
                auth={{kind="none"}},models={{{{id="custom",name="Custom",context_window=8000,max_tokens=900}}}}}})
        "#
        ));
        let (provider, model) = catalog.find("anthropic/custom").expect("replacement");
        assert_eq!(provider.base_url.as_deref(), Some(url));
        assert_eq!(resolve(model.compat).prompt_cache, PromptCache::None);
        assert_eq!(body(catalog, "anthropic/custom")["system"], "rules");
    }
}

#[test]
fn protocol_provider_name_and_even_the_direct_url_do_not_infer_cache_support() {
    let catalog = load(
        r#"
        melchior.provider("anthropic", {name="Anthropic",api="anthropic-messages",base_url="https://api.anthropic.com",
            auth={kind="none"},models={{id="unconfigured",name="Claude",context_window=8000,max_tokens=900}}})
    "#,
    );
    assert_eq!(body(catalog, "anthropic/unconfigured")["system"], "rules");
}

#[test]
fn model_disable_overrides_provider_enable_and_explicit_custom_mode_is_supported() {
    let source = r#"
        melchior.provider("declared", {name="Declared compatible endpoint",api="anthropic-messages",
            base_url="https://explicitly-configured.invalid",auth={kind="none"},
            compat={prompt_cache="anthropic_ephemeral"},models={
                {id="inherit",name="Inherit",context_window=8000,max_tokens=900},
                {id="off",name="Off",context_window=8000,max_tokens=900,compat={prompt_cache="none"}},
                {id="on",name="On",context_window=8000,max_tokens=900,compat={prompt_cache="anthropic_ephemeral"}}
            }})
    "#;
    for (id, enabled) in [("inherit", true), ("off", false), ("on", true)] {
        let body = body(load(source), &format!("declared/{id}"));
        assert_eq!(body["system"].is_array(), enabled, "{id}");
        if !enabled {
            assert_eq!(body["system"], "rules");
        }
    }
}
