//! What melchior can talk to, read out of Lua: `providers.lua` says which endpoints exist and
//! what they serve, `apis.lua` says how each is spoken to. Nothing in here resolves a
//! credential — whether a provider is ready is answered by looking for the variable it names and
//! never by reading it, because a card crosses a socket and a key must not.

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
    /// Read the configuration and register what it declares. A shipped file that will not
    /// compile or raises is fatal.
    pub fn load(dir: &Path) -> Result<Self, LuaError> {
        let mut engine = Engine::new();
        // Protocols first: a provider may name one, and a name that resolves to nothing should
        // be a refusal rather than an ordering accident.
        for (name, builtin) in [("apis.lua", APIS), ("providers.lua", PROVIDERS)] {
            // The shipped copy always runs and a person's file runs over the top of it. The
            // registrar replaces on `(registrar, id)`, so a file naming a shipped protocol still
            // means it and one naming something new only adds.
            engine.run(builtin, name)?;

            let path = dir.join(name);
            if let Ok(source) = std::fs::read_to_string(&path) {
                engine.run(&source, &path.display().to_string())?;
            }
        }

        // Then whatever is installed, discovered rather than named: after the shipped files and
        // the config's own copies of them, before the coordinator's. One of these that raises
        // costs itself and nothing else, since it is somebody else's package.
        let known =
            crate::mind::acknowledged::recorded(&crate::mind::acknowledged::manifest_in(dir));
        for (path, trust) in
            crate::mind::plugins::runtimepath(&crate::mind::plugins::Roots::at(dir))
        {
            let Ok(source) = std::fs::read_to_string(&path) else {
                continue;
            };
            // A fetched package runs only once acknowledged; your own files run on sight.
            if trust.needs_acknowledging()
                && !crate::mind::acknowledged::cleared(&known, &path, &source)
            {
                eprintln!(
                    "melchior: {}; run `melchior acknowledge` to clear it",
                    crate::mind::acknowledged::Held {
                        path: path.clone(),
                        known: crate::mind::acknowledged::seen(&known, &path),
                    }
                );
                continue;
            }
            let named = path.display().to_string();
            if let Err(why) = engine.run(&source, &named) {
                eprintln!("melchior: {named}: {why}");
            }
        }

        // Last and over the top: what a coordinator said outranks what is on disk. Nothing there
        // is the ordinary case of a melchior nobody is coordinating.
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
        // Ask the providers that asked to be asked.
        let mut providers = providers;
        crate::mind::discovering::discover(&mut providers);
        Ok(Self { providers, engine })
    }

    /// Where the configuration lives: `$MELCHIOR_CONFIG` first, so a test or a second install
    /// names its own, then `$XDG_CONFIG_HOME/melchior`. Nothing there is not an error, since the
    /// binary carries a copy of both files.
    #[must_use]
    pub fn dir() -> PathBuf {
        if let Some(named) = std::env::var_os("MELCHIOR_CONFIG").filter(|v| !v.is_empty()) {
            return PathBuf::from(named);
        }
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
        // No relative fallback: a `config` resolved against the working directory would make
        // melchior read a sibling checkout's files.
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

/// One declaration, as a `Provider`. The registrar keys by name and the value does not repeat
/// it, so the id, the owning provider and the interface are put back onto each model here.
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
            // A model may name its own interface, one provider serving several, and takes the
            // provider's only when it does not.
            if !model.contains_key("api")
                && let Some(api) = api.clone()
            {
                model.insert("api".into(), api);
            }
        }
    }
    serde_json::from_value(value).ok()
}

/// One model, described without naming a secret. `needs` is the first of the several variables
/// a key-authenticated provider would accept, not all of them.
fn card(provider: &Provider, model: &Model) -> Card {
    let needs = match &provider.auth {
        Auth::ApiKey { vars } => vars.first().cloned(),
        _ => None,
    };
    // Ready when nothing is wanted, or when one accepted name is set to something non-empty: an
    // empty variable is how a shell says unset, and treating it as configured yields a 401.
    let ready = match &provider.auth {
        Auth::ApiKey { vars } => vars
            .iter()
            .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty())),
        Auth::None => true,
        // OAuth and the cloud signatures cannot be answered from the environment.
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
        // A window of zero or of a hundred million never compacts, and fails silently.
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

#[cfg(test)]
mod layering_tests {
    use super::*;
    use crate::scratch::Scratch;

    /// A config directory holding one file.
    fn holding(name: &str, source: &str) -> Scratch {
        let dir = Scratch::new("melchior-layer", name);
        std::fs::write(dir.join(name), source).expect("write");
        dir
    }

    #[test]
    fn a_persons_file_adds_to_the_shipped_protocols_rather_than_replacing_them() {
        // A person's file adds to the shipped protocols rather than replacing them.
        let dir = holding(
            "apis.lua",
            r#"melchior.api("mine-own", { chat = function(ask) return ask end })"#,
        );
        let catalog = Catalog::load(&dir).expect("loads");

        let mut catalog = catalog;
        let known = catalog.engine.apis();
        assert!(
            known.iter().any(|a| a == "mine-own"),
            "the file on disk was read: {known:?}"
        );
        assert!(
            known.iter().any(|a| a == "openai-completions"),
            "and the shipped protocols are still there, which is the whole point: {known:?}"
        );
    }

    #[test]
    fn a_persons_file_may_still_replace_one_by_name() {
        // The registrar replaces on `(registrar, id)`, so a file naming a shipped protocol still
        // replaces that one.
        let dir = holding(
            "apis.lua",
            r#"melchior.api("openai-completions", { chat = function() return "mine" end })"#,
        );
        let mut catalog = Catalog::load(&dir).expect("loads");
        let known = catalog.engine.apis();
        assert!(
            known.iter().any(|a| a == "openai-completions"),
            "still there"
        );
        assert!(
            known.iter().any(|a| a == "anthropic-messages"),
            "and replacing one did not take the rest with it: {known:?}"
        );
    }
}

#[cfg(test)]
mod discovery_tests {
    use super::*;
    use crate::scratch::Scratch;

    #[test]
    fn a_file_dropped_in_plugin_declares_a_protocol_without_touching_anything_shipped() {
        // A protocol may arrive as a file of its own in a directory that nothing else names.
        let dir = Scratch::new("melchior-disc", "dropped");
        std::fs::create_dir_all(dir.join("plugin")).expect("mkdir");
        std::fs::write(
            dir.join("plugin/mine.lua"),
            r#"melchior.api("mine-own", { chat = function(ask) return ask end })"#,
        )
        .expect("write");

        let mut catalog = Catalog::load(&dir).expect("loads");
        let known = catalog.engine.apis();
        assert!(known.iter().any(|api| api == "mine-own"), "{known:?}");
        assert!(
            known.iter().any(|api| api == "openai-completions"),
            "and the shipped protocols are untouched: {known:?}"
        );
    }

    #[test]
    fn after_gets_the_last_word_over_a_plugin() {
        // The registrar replaces on `(registrar, id)`, so `after/` is how a person overrides
        // something a package they installed declared.
        let dir = Scratch::new("melchior-disc", "after");
        std::fs::create_dir_all(dir.join("plugin")).expect("mkdir");
        std::fs::create_dir_all(dir.join("after/plugin")).expect("mkdir");
        for (at, body) in [("plugin/it.lua", "theirs"), ("after/plugin/it.lua", "mine")] {
            std::fs::write(
                dir.join(at),
                format!(
                    r#"melchior.api("contested", {{ chat = function() return "{body}" end }})"#
                ),
            )
            .expect("write");
        }

        let mut catalog = Catalog::load(&dir).expect("loads");
        assert!(catalog.engine.apis().iter().any(|api| api == "contested"));
    }

    #[test]
    fn a_plugin_that_raises_does_not_stop_the_others() {
        // Somebody else's package: a raise costs itself, where the shipped files are fatal.
        let dir = Scratch::new("melchior-disc", "broken");
        std::fs::create_dir_all(dir.join("plugin")).expect("mkdir");
        std::fs::write(dir.join("plugin/a-broken.lua"), "error(\"no\")").expect("write");
        std::fs::write(
            dir.join("plugin/b-fine.lua"),
            r#"melchior.api("survivor", { chat = function(ask) return ask end })"#,
        )
        .expect("write");

        let mut catalog = Catalog::load(&dir).expect("loads despite the broken one");
        let known = catalog.engine.apis();
        assert!(known.iter().any(|api| api == "survivor"), "{known:?}");
    }
}
