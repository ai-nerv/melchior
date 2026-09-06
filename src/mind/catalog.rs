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

/// The protocols this build ships, for a machine with no configuration of its own.
const APIS: &str = include_str!("../../config/apis.lua");

/// The catalog this build ships, for the same reason.
const PROVIDERS: &str = include_str!("../../config/providers.lua");

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
        for (name, builtin) in [("apis.lua", APIS), ("providers.lua", PROVIDERS)] {
            let path = dir.join(name);
            match std::fs::read_to_string(&path) {
                Ok(source) => engine.run(&source, &path.display().to_string())?,
                // The copy in the binary. Without it melchior would only work where a config
                // happened to be — and the relative fallback made that *whatever* `config/` sat
                // next to the working directory, so running from magi's checkout loaded magi's
                // protocols, which name a different global and fail at the first index.
                Err(_) => engine.run(builtin, name)?,
            }
        }
        // Last, and over the top. What a coordinator said outranks what is on disk: magi is
        // deciding, and a file that quietly won would be the disagreement this exists to end.
        // Nothing there is the ordinary case of a melchior nobody is coordinating.
        if let Ok(given) = std::fs::read_to_string(crate::mind::setup::given()) {
            engine.run(&given, "given")?;
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
    /// `$XDG_CONFIG_HOME/melchior`. Nothing there is not an error: the binary carries a copy of
    /// both files, so melchior works on a machine that has never been configured.
    #[must_use]
    pub fn dir() -> PathBuf {
        if let Some(named) = std::env::var_os("MELCHIOR_CONFIG").filter(|v| !v.is_empty()) {
            return PathBuf::from(named);
        }
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
        // No relative fallback. `config` resolved against the working directory, so melchior
        // run from a sibling's checkout read that sibling's files. Nothing there means the copy
        // in the binary.
        base.map_or_else(
            || PathBuf::from("/nonexistent"),
            |base| base.join("melchior"),
        )
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

/// What the shipped catalog must be true of, whoever edits it.
///
/// These came across with the catalog itself. They were magi's while magi held the providers,
/// and they assert about the same file — a description that says nothing useful is a model
/// nobody can reach, and the failure shows up as "no such model" a long way from the cause.
#[cfg(test)]
mod shape {
    use super::*;

    fn shipped() -> Catalog {
        Catalog::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("config")).expect("load")
    }

    #[test]
    fn provider_ids_are_unique() {
        let catalog = shipped();
        let mut seen = std::collections::BTreeSet::new();
        for provider in &catalog.providers {
            assert!(seen.insert(provider.id.clone()), "twice: {}", provider.id);
        }
    }

    #[test]
    fn a_provider_either_lists_its_models_or_asks_for_them() {
        // Neither is a provider that offers nothing, which reads from the outside as the
        // provider being broken rather than the declaration being empty.
        for provider in &shipped().providers {
            assert!(
                !provider.models.is_empty() || provider.discover,
                "{} lists no models and does not discover",
                provider.id
            );
        }
    }

    #[test]
    fn every_model_is_stamped_with_its_provider_and_an_interface() {
        // A config says these once at the top; a `Model` carries them. Assembling them wrongly
        // is how a model ends up spoken to over the wrong protocol.
        let catalog = shipped();
        for provider in &catalog.providers {
            for model in &provider.models {
                assert_eq!(model.provider, provider.id, "{} is misstamped", model.id);
            }
        }
        for card in catalog.cards() {
            assert!(!card.api.is_empty(), "{} names no interface", card.id);
        }
    }

    #[test]
    fn context_windows_are_plausible() {
        // A window of zero cannot be over, so it never compacts; one of a hundred million
        // compacts never. Both fail silently, which is why this is asserted rather than trusted.
        for provider in &shipped().providers {
            for model in &provider.models {
                assert!(
                    (1000..=10_000_000).contains(&model.context_window),
                    "{}: {} is not a plausible window",
                    model.id,
                    model.context_window
                );
            }
        }
    }

    #[test]
    fn the_registration_name_becomes_the_id() {
        // The id comes from the registration rather than the table, so a config cannot declare
        // one name and register another — and a loop over a directory names each by its key.
        for provider in &shipped().providers {
            assert!(!provider.id.is_empty());
        }
        let declared = assemble(
            "named-here",
            &serde_json::json!({
                "name": "Something Else",
                "api": "openai-completions",
                "auth": { "kind": "none" },
            }),
        )
        .expect("assembles");
        assert_eq!(
            declared.id, "named-here",
            "the key registered it, not the table"
        );
        assert_eq!(declared.name, "Something Else", "the table still names it");
    }

    #[test]
    fn a_config_may_declare_providers_in_a_loop() {
        // The point of the config being Lua. A provider declared in a loop is the same table as
        // one written out by hand.
        let dir = crate::scratch::Scratch::new("melchior-loop", "one");
        std::fs::write(
            dir.join("providers.lua"),
            r#"
            for _, host in ipairs({ "one", "two", "three" }) do
              melchior.provider(host, {
                name = host,
                api = "openai-completions",
                base_url = "http://" .. host .. "/v1",
                auth = { kind = "none" },
                models = { { id = "m", name = "M", context_window = 8000, max_tokens = 900 } },
              })
            end
            "#,
        )
        .expect("write");

        let catalog = Catalog::load(&dir).expect("load");
        for host in ["one", "two", "three"] {
            assert!(
                catalog.providers.iter().any(|p| p.id == host),
                "{host} was not declared"
            );
        }
    }
}
