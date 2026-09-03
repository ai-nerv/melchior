//! What melchior can talk to, read out of Lua.
//!
//! `providers.lua` says which endpoints exist and what they serve; `apis.lua` says how each is
//! spoken to. Both are descriptions rather than code melchior ships, which is the whole point of
//! there being a VM here: a protocol nobody anticipated is a file, not a release.
//!
//! Nothing in here resolves a credential. Whether a provider is *ready* is answered by looking
//! for the variable it names, never by reading it: a card crosses a socket and a key must not.

use crate::mind::lua::LuaError;
use crate::mind::lua::engine::Engine;
use crate::mind::provider::endpoint::{Auth, Provider};
use crate::mind::provider::model::Model;
use crate::mind::wire::Card;
use std::path::{Path, PathBuf};

/// Everything a config declared.
pub struct Catalog {
    /// The endpoints, in the order they were declared.
    pub providers: Vec<Provider>,
    /// The VM the protocol descriptions live in, because an adapter *is* a Lua function.
    pub engine: Engine,
}

impl Catalog {
    /// Read the configuration and register what it declares.
    ///
    /// # Errors
    /// When a file will not compile or raises while running. Fatal on purpose: a description
    /// that does not load has not expressed an intention, and guessing at one is worse than
    /// stopping.
    pub fn load(dir: &Path) -> Result<Self, LuaError> {
        let mut engine = Engine::new();
        // Protocols first: a provider may name one, and a name that resolves to nothing should
        // be a refusal rather than an ordering accident.
        for name in ["apis.lua", "providers.lua"] {
            let path = dir.join(name);
            if path.exists() {
                engine.run_file(&path)?;
            }
        }
        engine.harvest();
        let config = engine.config();
        let providers: Vec<Provider> = config
            .all("provider")
            .into_iter()
            .filter_map(|(name, value)| assemble(name, value))
            .collect();
        drop(config);
        // Ask the providers that asked to be asked. A hand-written list is stale the day it is
        // written, and openrouter alone offers four hundred models.
        let mut providers = providers;
        crate::mind::discovering::discover(&mut providers);
        Ok(Self { providers, engine })
    }

    /// Where the configuration lives.
    ///
    /// `$MELCHIOR_CONFIG` first, so a test or a second install names its own; then
    /// `$XDG_CONFIG_HOME/melchior`; then the checkout's own `config/`, which is what makes this
    /// runnable before it is installed.
    #[must_use]
    pub fn dir() -> PathBuf {
        if let Some(named) = std::env::var_os("MELCHIOR_CONFIG").filter(|v| !v.is_empty()) {
            return PathBuf::from(named);
        }
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
        match base {
            Some(base) if base.join("melchior/providers.lua").exists() => base.join("melchior"),
            _ => PathBuf::from("config"),
        }
    }

    /// Every model, as a card.
    #[must_use]
    pub fn cards(&self) -> Vec<Card> {
        self.providers
            .iter()
            .flat_map(|provider| {
                provider
                    .models
                    .iter()
                    .map(|model| card(provider, model))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// The provider and model a card id names.
    #[must_use]
    pub fn find(&self, id: &str) -> Option<(&Provider, &Model)> {
        let (provider, model) = id.split_once('/')?;
        let provider = self.providers.iter().find(|p| p.id == provider)?;
        let model = provider.models.iter().find(|m| m.id == model)?;
        Some((provider, model))
    }
}

/// One declaration, as a `Provider`.
///
/// The registrar keys by name and the value does not repeat it, so the id is put back here. Each
/// model is told which provider it belongs to and which interface it is spoken over for the same
/// reason: a config says those once, at the top, and a `Model` carries them.
fn assemble(name: &str, value: &serde_json::Value) -> Option<Provider> {
    let mut value = value.clone();
    let object = value.as_object_mut()?;
    object.insert("id".into(), serde_json::Value::String(name.to_owned()));
    let api = object.get("api").cloned();
    if let Some(serde_json::Value::Array(models)) = object.get_mut("models") {
        for model in models.iter_mut() {
            let Some(model) = model.as_object_mut() else {
                continue;
            };
            model.insert(
                "provider".into(),
                serde_json::Value::String(name.to_owned()),
            );
            // A model may name its own interface -- one provider can serve several -- and takes
            // the provider's only when it does not.
            if !model.contains_key("api")
                && let Some(api) = api.clone()
            {
                model.insert("api".into(), api);
            }
        }
    }
    serde_json::from_value(value).ok()
}

/// One model, described without naming a secret.
///
/// `needs` is the *first* variable a key-authenticated provider would accept. Vendors rename
/// them and people keep old ones exported, so several are tried; naming all of them in a card
/// would be a wall of text where a person wants one line.
fn card(provider: &Provider, model: &Model) -> Card {
    let needs = match &provider.auth {
        Auth::ApiKey { vars } => vars.first().cloned(),
        _ => None,
    };
    // Ready when nothing is wanted, or when one of the names it would accept is set to
    // something. A variable exported as empty is how a shell says "unset" often enough that
    // treating it as configured produces a 401 nobody can explain.
    let ready = match &provider.auth {
        Auth::ApiKey { vars } => vars
            .iter()
            .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty())),
        Auth::None => true,
        // OAuth and the cloud signatures cannot be answered by looking at the environment, and
        // guessing "ready" would offer a model that fails on the first call.
        _ => false,
    };
    Card {
        id: format!("{}/{}", provider.id, model.id),
        provider: provider.id.clone(),
        name: model.id.clone(),
        api: serde_json::to_value(model.api)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_default(),
        context_window: Some(model.context_window).filter(|n| *n > 0),
        max_output: Some(model.max_tokens).filter(|n| *n > 0),
        reasons: model.reasoning,
        ready,
        needs: if ready { None } else { needs },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The checkout's own config, so this runs before anything is installed.
    fn shipped() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("config")
    }

    #[test]
    fn the_shipped_configuration_loads() {
        let catalog = Catalog::load(&shipped()).expect("apis.lua and providers.lua must load");
        assert!(
            !catalog.providers.is_empty(),
            "providers.lua declared nothing"
        );
    }

    #[test]
    fn every_provider_becomes_cards_that_name_an_interface() {
        let catalog = Catalog::load(&shipped()).expect("load");
        let cards = catalog.cards();
        assert!(!cards.is_empty(), "no models");
        for card in &cards {
            assert!(
                card.id.contains('/'),
                "a card id is provider/model: {card:?}"
            );
            assert!(!card.api.is_empty(), "no interface named: {card:?}");
        }
    }

    #[test]
    fn a_card_never_carries_the_credential_it_names() {
        let catalog = Catalog::load(&shipped()).expect("load");
        let text = serde_json::to_string(&catalog.cards()).expect("encode");
        // The variable's name may travel; nothing that looks like a key may.
        assert!(!text.contains("sk-"), "a key reached a card");
    }

    #[test]
    fn a_card_id_finds_the_provider_and_model_it_names() {
        let catalog = Catalog::load(&shipped()).expect("load");
        let first = catalog.cards().first().cloned().expect("a card");
        let (provider, model) = catalog.find(&first.id).expect("found");
        assert_eq!(format!("{}/{}", provider.id, model.id), first.id);
        assert!(catalog.find("nobody/nothing").is_none());
    }
}
