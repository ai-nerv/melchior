//! Lua provider declarations, model cards and secret-free setup status.

use crate::mind::lua::LuaError;
use crate::mind::lua::engine::Engine;
use crate::mind::provider::endpoint::Provider;
use crate::mind::provider::model::Model;
use crate::mind::wire::Card;
use std::path::{Path, PathBuf};

/// The protocols this build ships, for a machine with no configuration of its own.
const APIS: &str = include_str!("../../config/apis.lua");

/// The catalog this build ships, for the same reason.
const PROVIDERS: &str = include_str!("../../config/providers.lua");

#[cfg(test)]
mod cache_tests;

mod declarations;
mod imports;
mod readiness;
use declarations::assemble;

#[cfg(test)]
mod declaration_tests;

/// Everything a config declared.
pub struct Catalog {
    /// Discovery failures, identified by provider rather than endpoint or credentials.
    pub discovery_failed: Vec<String>,
    /// The endpoints, in the order they were declared.
    pub providers: Vec<Provider>,
    /// The VM the protocol descriptions live in, because an adapter *is* a Lua function.
    pub engine: Engine,
    protocols: std::collections::BTreeSet<String>,
}

impl Catalog {
    /// Read the configuration and register what it declares. A shipped file that will not
    /// compile or raises is fatal.
    pub fn load(dir: &Path) -> Result<Self, LuaError> {
        Self::load_with_refresh(dir, false)
    }

    /// Load configuration and optionally bypass discovery caches.
    pub fn load_with_refresh(dir: &Path, refresh: bool) -> Result<Self, LuaError> {
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
        if let Some(given) =
            crate::mind::setup::given().and_then(|path| crate::mind::setup::read_given(&path))
        {
            engine.run(&given, "given")?;
        }
        engine.harvest();
        let config = engine.config();
        let providers: Vec<Provider> = config
            .all("provider")
            .into_iter()
            .map(|(name, value)| assemble(name, value))
            .collect::<Result<_, _>>()?;
        drop(config);
        let protocols = engine.usable_apis().into_iter().collect();
        // Ask the providers that asked to be asked.
        let mut providers = providers;
        let discovery_failed =
            crate::mind::discovering::discover_with_refresh(&mut providers, refresh);
        crate::mind::provider::copilot::apply_catalog(&mut providers);
        Ok(Self {
            providers,
            engine,
            discovery_failed,
            protocols,
        })
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
        let store = readiness::store(&self.providers);
        let now = crate::mind::provider::oauth::now();
        self.providers
            .iter()
            .flat_map(|provider| {
                let setup = readiness::provider(provider, &store, now);
                provider
                    .models
                    .iter()
                    .map(|model| {
                        let mut setup = readiness::model(provider, model, setup.clone());
                        if !self.protocols.contains(model.api.as_str()) {
                            setup.state = crate::mind::wire::SetupState::ProtocolUnavailable;
                        }
                        card(provider, model, setup)
                    })
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

#[cfg(test)]
mod price_presence_tests {
    use super::*;

    #[test]
    fn actual_lua_declarations_preserve_absent_zero_and_positive_cached_prices() {
        let mut engine = Engine::new();
        engine
            .run(
                r#"
            local models = {}
            for i, cost in ipairs({ {input=3}, {input=3,cache_read=0}, {input=3,cache_read=1} }) do
                models[i] = {id=tostring(i), name=tostring(i), context_window=8192,
                             max_tokens=128, cost=cost}
            end
            melchior.provider("prices", {name="Prices", api="openai-completions",
                              auth={kind="none"}, models=models})
        "#,
                "prices.lua",
            )
            .expect("source declarations");
        engine.harvest();
        let config = engine.config();
        let (name, value) = config.all("provider").into_iter().next().expect("provider");
        let provider = assemble(name, value).expect("catalog models");
        let usage = crate::mind::model::Usage {
            cache_read: 1_000_000,
            ..Default::default()
        };
        let actual: Vec<_> = provider
            .models
            .iter()
            .map(|model| usage.price(&model.cost))
            .collect();
        assert_eq!(actual, vec![3.0, 0.0, 1.0]);
        for model in provider.models {
            let value = serde_json::to_value(&model).expect("model");
            assert_eq!(value["cost"]["presence"]["version"], 1);
            let reloaded: Model = serde_json::from_value(value).expect("model reload");
            assert_eq!(usage.price(&model.cost), usage.price(&reloaded.cost));
        }
    }
}

/// One model, described without naming a secret. `needs` is the first of the several variables
/// a key-authenticated provider would accept, not all of them.
fn card(provider: &Provider, model: &Model, setup: crate::mind::wire::Setup) -> Card {
    let ready = setup.state == crate::mind::wire::SetupState::Ready;
    let needs = setup
        .variables
        .first()
        .and_then(|group| group.first())
        .cloned();
    Card {
        vision: model.accepts_images(),
        id: format!("{}/{}", provider.id, model.id),
        provider: provider.id.clone(),
        name: model.id.clone(),
        display_name: (!model.name.trim().is_empty() && model.name != model.id)
            .then(|| model.name.clone()),
        api: serde_json::to_value(model.api)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_default(),
        context_window: Some(model.context_window).filter(|n| *n > 0),
        max_output: Some(model.max_tokens).filter(|n| *n > 0),
        pricing: model.pricing.clone().or_else(|| model.cost.pricing()),
        popularity_rank: model.popularity_rank,
        capabilities: Some(session_capabilities(model)),
        reasons: model.reasoning,
        ready,
        needs: if ready { None } else { needs },
        setup: Some(setup),
    }
}

fn session_capabilities(model: &Model) -> crate::mind::model::Capabilities {
    use crate::mind::{model::Capabilities, provider::model::Modality};
    let mut caps =
        crate::mind::provider::thinking::capabilities(model).unwrap_or_else(|| Capabilities {
            input: model
                .input
                .iter()
                .map(|input| match input {
                    Modality::Text => "text".into(),
                    Modality::Image => "image".into(),
                })
                .collect(),
            output: vec!["text".into()],
            ..Capabilities::default()
        });
    caps.session_identity = Some(true);
    caps
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

    #[test]
    fn a_declared_model_card_says_vision_and_prices_from_its_cost() {
        let cards = Catalog::load(&shipped()).expect("load").cards();
        let find = |id: &str| cards.iter().find(|card| card.id == id).expect(id);
        let opus = find("anthropic/claude-opus-4-6");
        assert!(opus.vision);
        assert_eq!(
            opus.pricing,
            Some(crate::mind::model::Pricing {
                input: Some("5".into()),
                output: Some("25".into()),
            })
        );
        assert_eq!(opus.popularity_rank, None);
        let chat = find("deepseek/deepseek-v4-pro");
        assert!(!chat.vision);
        assert_eq!(
            chat.pricing.as_ref().and_then(|p| p.output.as_deref()),
            Some("3.96")
        );
        let flash = find("deepseek/deepseek-flash");
        assert!(flash.vision);
        assert_eq!(
            flash.pricing.as_ref().and_then(|p| p.output.as_deref()),
            Some("1.2")
        );
        let jev = find("decisions/typesafe/jev-1.13");
        assert_eq!(
            jev.pricing.as_ref().and_then(|p| p.input.as_deref()),
            Some("0.042")
        );
        assert_eq!(
            jev.pricing.as_ref().and_then(|p| p.output.as_deref()),
            Some("0")
        );
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
