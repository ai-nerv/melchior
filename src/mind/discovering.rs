//! Asking a provider what it offers, and remembering the answer. A fetch never writes back to
//! `providers.lua`; what it finds goes to a private identity-keyed cache under
//! `$XDG_CACHE_HOME/melchior/models/v2/`. That
//! cache is read at load time. Missing, expired, or explicitly refreshed catalogs are fetched;
//! a failed fetch leaves whatever the cache last held.

use crate::mind::model::ThinkingLevel;
use crate::mind::provider::endpoint::Provider;
use crate::mind::provider::model::Model;
use std::time::Duration;

pub(crate) mod cache;
mod capabilities;
mod loading;
mod ollama;
mod strategy;
pub use strategy::{Header, Strategy};

#[cfg(test)]
mod cache_tests;

/// How long a fetched catalog is used before it is asked for again.
pub const FRESH: Duration = Duration::from_secs(24 * 60 * 60);

/// How long a fetch may take before the cache — even a stale one — is preferred. This runs while
/// somebody is waiting for a prompt.
pub(crate) const PATIENCE: Duration = Duration::from_secs(5);

/// What a model is assumed to hold when the provider does not say: an underestimate, which costs
/// an early compaction, where an overestimate costs a refused request mid-turn.
const ASSUMED_WINDOW: u64 = 128_000;

/// Fill in the models of every provider that asked to be discovered; one that declares its own
/// models, or neither models nor discovery, is left as it is.
pub fn discover(providers: &mut [Provider]) {
    discover_with_refresh(providers, false);
}

/// Discover catalogs, optionally bypassing fresh caches, and name providers that failed.
pub fn discover_with_refresh(providers: &mut [Provider], refresh: bool) -> Vec<String> {
    loading::discover(providers, refresh)
}

/// A discovered model as its provider serves it: the provider's id, protocol and dialect. Without
/// the dialect it spoke plain OpenAI, and a reasoning level, off included, never reached the provider.
fn joined(mut model: Model, provider: &crate::mind::provider::endpoint::Provider) -> Model {
    model.base_url = None;
    model.headers.clear();
    model.provider.clone_from(&provider.id);
    model.api = provider.api;
    model.compat = Some(model.compat.unwrap_or_default().over(provider.compat));
    model
}

fn cached_models(raw: &str) -> Option<Vec<Model>> {
    let mut values: Vec<serde_json::Value> = serde_json::from_str(raw).ok()?;
    if values.len() > loading::MODELS {
        return None;
    }
    for value in &mut values {
        if let Some(cost) = value
            .get_mut("cost")
            .and_then(serde_json::Value::as_object_mut)
        {
            let presence = cost
                .get("presence")
                .and_then(|value| {
                    serde_json::from_value::<crate::mind::model::PricePresence>(value.clone()).ok()
                })
                .filter(|presence| presence.version == 1)
                .unwrap_or_default();
            cost.insert("presence".into(), serde_json::to_value(presence).ok()?);
        }
    }
    let models: Vec<Model> = values
        .into_iter()
        .map(|value| serde_json::from_value(value).ok())
        .collect::<Option<_>>()?;
    identities(&models).then_some(models)
}

fn identities(models: &[Model]) -> bool {
    let mut ids = std::collections::BTreeSet::new();
    models.iter().all(|model| {
        !model.id.trim().is_empty()
            && !model.id.chars().any(char::is_control)
            && ids.insert(&model.id)
    })
}

/// The key this provider authenticates with, if the environment has one.
pub(crate) fn key_for(provider: &Provider) -> Option<String> {
    provider.resolve_key().ok().flatten()
}

/// Read an OpenAI-shaped `{"data": [...]}` answer into models. What an entry carries varies by
/// provider, so everything but the id is optional and an absent field takes a default rather
/// than dropping the model.
#[must_use]
pub fn parse(body: &serde_json::Value) -> Vec<Model> {
    let Some(entries) = body.get("data").and_then(|d| d.as_array()) else {
        return Vec::new();
    };
    entries.iter().filter_map(one).collect()
}

fn ranked(body: &serde_json::Value, popularity: bool) -> Vec<Model> {
    let Some(entries) = body.get("data").and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };
    entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            let mut model = one(entry)?;
            model.popularity_rank = popularity.then_some(index as u64 + 1);
            Some(model)
        })
        .collect()
}

/// One entry of a `/models` answer.
fn one(entry: &serde_json::Value) -> Option<Model> {
    let id = entry.get("id")?.as_str()?.to_owned();
    let name = entry
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or(&id)
        .to_owned();
    let window = number(entry, "context_length")
        .or_else(|| {
            entry
                .get("top_provider")
                .and_then(|t| number(t, "context_length"))
        })
        .unwrap_or(ASSUMED_WINDOW);
    let ceiling = entry
        .get("top_provider")
        .and_then(|t| number(t, "max_completion_tokens"))
        .or_else(|| number(entry, "max_completion_tokens"))
        // A quarter of the window, which is what most providers allow and none forbid.
        .unwrap_or((window / 4).max(1));
    Some(Model {
        id,
        name,
        reasoning: reasons(entry),
        context_window: window,
        max_tokens: ceiling,
        cost: cost(entry),
        pricing: pricing(entry),
        popularity_rank: None,
        capabilities: Some(capabilities::from_entry(entry)),
        input_limits: None,
        // Overwritten by the provider, which knows its own id and protocol.
        provider: String::new(),
        api: crate::mind::provider::model::Api::OpenAiCompletions,
        base_url: None,
        headers: Default::default(),
        input: {
            use crate::mind::provider::model::Modality;
            let mut input = vec![Modality::Text];
            if entry
                .pointer("/architecture/input_modalities")
                .or_else(|| entry.get("input"))
                .and_then(serde_json::Value::as_array)
                .is_some_and(|items| items.iter().any(|item| item.as_str() == Some("image")))
            {
                input.push(Modality::Image);
            }
            input
        },
        thinking: levels(entry),
        compat: None,
    })
}

/// What each thinking level is called here.
///
/// A model that must reason has no "off", and saying nothing is not the same as saying "off": with
/// no effort named the provider applies its own default, which for these is usually the largest
/// one. A helper given a thousand tokens then spends them all reasoning and answers nothing. So
/// "off" becomes the least reasoning this model will do, which is a request it accepts and a bill
/// the caller meant. `reasoning.mandatory` and `reasoning.supported_efforts` are its own words.
fn levels(entry: &serde_json::Value) -> std::collections::BTreeMap<ThinkingLevel, Option<String>> {
    let mut out = std::collections::BTreeMap::new();
    let reasoning = entry.get("reasoning");
    let mandatory = reasoning
        .and_then(|it| it.get("mandatory"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let efforts = reasoning
        .and_then(|it| it.get("supported_efforts"))
        .and_then(serde_json::Value::as_array);
    if let Some(efforts) = efforts {
        for level in ThinkingLevel::ALL {
            let supported = efforts
                .iter()
                .any(|value| value.as_str() == Some(level.as_str()));
            let name = (supported || (level == ThinkingLevel::Off && !mandatory))
                .then(|| level.as_str().to_owned());
            out.insert(level, name);
        }
    }
    if mandatory {
        let least = least(reasoning);
        let available = efforts.is_none_or(|values| {
            values
                .iter()
                .any(|value| value.as_str() == Some(least.as_str()))
        });
        out.insert(ThinkingLevel::Off, available.then_some(least));
    }
    out
}

/// The least reasoning a model offers, by name. Ranked here rather than taken in the order the
/// provider happens to list them, which is nobody's idea of least-first.
fn least(reasoning: Option<&serde_json::Value>) -> String {
    const LEAST_FIRST: [&str; 6] = ["minimal", "low", "medium", "high", "xhigh", "max"];
    let offered: Vec<&str> = reasoning
        .and_then(|it| it.get("supported_efforts"))
        .and_then(serde_json::Value::as_array)
        .map(|list| list.iter().filter_map(serde_json::Value::as_str).collect())
        .unwrap_or_default();
    LEAST_FIRST
        .iter()
        .find(|want| offered.contains(want))
        .map_or_else(|| "low".to_owned(), |found| (*found).to_owned())
}

/// Whether the provider says this model can reason.
fn reasons(entry: &serde_json::Value) -> bool {
    entry.get("reasoning").and_then(serde_json::Value::as_bool) == Some(true)
        || entry
            .pointer("/reasoning/mandatory")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
        || entry
            .pointer("/reasoning/supported_efforts")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|values| {
                values
                    .iter()
                    .any(|value| value.as_str().is_some_and(|name| name != "off"))
            })
        || entry
            .get("supported_parameters")
            .and_then(|p| p.as_array())
            .is_some_and(|params| {
                params.iter().filter_map(|p| p.as_str()).any(|p| {
                    p == "reasoning" || p == "include_reasoning" || p == "reasoning_effort"
                })
            })
}

/// Price per million tokens, from the dollars-per-token decimal string providers quote; an
/// unreadable price is no price rather than a wrong one.
fn cost(entry: &serde_json::Value) -> crate::mind::model::Cost {
    let input = rate(entry, "prompt");
    let output = rate(entry, "completion");
    let cache_read = rate(entry, "input_cache_read");
    let cache_write = rate(entry, "input_cache_write");
    crate::mind::model::Cost {
        input: input.unwrap_or_default(),
        output: output.unwrap_or_default(),
        cache_read: cache_read.unwrap_or_default(),
        cache_write: cache_write.unwrap_or_default(),
        presence: crate::mind::model::PricePresence {
            input: input.is_some(),
            output: output.is_some(),
            cache_read: cache_read.is_some(),
            cache_write: cache_write.is_some(),
            ..Default::default()
        },
        ..crate::mind::model::Cost::default()
    }
}

fn pricing(entry: &serde_json::Value) -> Option<crate::mind::model::Pricing> {
    let input = rate(entry, "prompt").map(|rate| rate.to_string());
    let output = rate(entry, "completion").map(|rate| rate.to_string());
    (input.is_some() || output.is_some()).then_some(crate::mind::model::Pricing { input, output })
}

fn rate(entry: &serde_json::Value, name: &str) -> Option<f64> {
    let value = entry.get("pricing")?.get(name)?;
    let per_token = value
        .as_f64()
        .or_else(|| value.as_str()?.parse::<f64>().ok())?;
    let per_million = per_token * 1_000_000.0;
    (per_million.is_finite() && per_million >= 0.0).then_some(per_million)
}

/// A number that may have arrived as a float, an integer, or a string.
fn number(value: &serde_json::Value, name: &str) -> Option<u64> {
    let field = value.get(name)?;
    field
        .as_u64()
        .or_else(|| field.as_f64().map(|n| n as u64))
        .or_else(|| field.as_str().and_then(|s| s.parse().ok()))
        .filter(|n| *n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogs_advertise_vision_only_when_the_provider_does() {
        let body = serde_json::json!({"data":[
            {"id":"vision", "architecture":{"input_modalities":["text","image","audio"]}},
            {"id":"text", "architecture":{"input_modalities":["text"]}},
            {"id":"unspecified"},
            {"id":"native", "input":["text","image"]}
        ]});
        let models = parse(&body);
        assert!(models[0].accepts_images());
        assert!(!models[1].accepts_images());
        assert!(!models[2].accepts_images());
        assert!(models[3].accepts_images());
    }

    #[test]
    fn published_prices_preserve_free_unknown_and_invalid_rates() {
        let entry = serde_json::json!({"pricing":{"prompt":"0","completion":0.000002}});
        let prices = pricing(&entry).expect("published prices");
        assert_eq!(prices.input.as_deref(), Some("0"));
        assert_eq!(prices.output.as_deref(), Some("2"));
        assert_eq!(cost(&entry).output, 2.0);
        assert!(pricing(&serde_json::json!({})).is_none());
        for invalid in ["-1", "NaN", "inf", "1e308", "not-a-price"] {
            let entry = serde_json::json!({"pricing":{"prompt":invalid,"completion":"0"}});
            let prices = pricing(&entry).expect("free output is known");
            assert!(prices.input.is_none());
            assert_eq!(prices.output.as_deref(), Some("0"));
            assert_eq!(cost(&entry).input, 0.0);
        }
    }

    #[test]
    fn only_a_popularity_sorted_catalog_supplies_weekly_positions() {
        let body = serde_json::json!({"data":[{"id":"popular"},{"not_an_id":true},{"id":"next"}]});
        let found = ranked(&body, true);
        assert_eq!(found[0].popularity_rank, Some(1));
        assert_eq!(found[1].popularity_rank, Some(3));
        assert!(
            ranked(&body, false)
                .iter()
                .all(|model| model.popularity_rank.is_none())
        );
        assert!(
            parse(&body)
                .iter()
                .all(|model| model.popularity_rank.is_none())
        );
    }

    /// One entry in the shape OpenRouter answers with.
    fn rich() -> serde_json::Value {
        serde_json::json!({ "data": [{
            "id": "anthropic/claude-opus-5",
            "name": "Claude Opus 5",
            "context_length": 1000000,
            "pricing": { "prompt": "0.000005", "completion": "0.000025" },
            "top_provider": { "max_completion_tokens": 128000 },
            "supported_parameters": ["reasoning", "tools"],
        }]})
    }

    /// What a local Ollama answers with: an id and almost nothing else.
    fn bare() -> serde_json::Value {
        serde_json::json!({ "data": [{ "id": "qwen3:8b" }] })
    }

    #[test]
    fn a_rich_entry_keeps_everything_it_was_told() {
        let found = parse(&rich());
        assert_eq!(found.len(), 1);
        let model = &found[0];
        assert_eq!(model.id, "anthropic/claude-opus-5");
        assert_eq!(model.name, "Claude Opus 5");
        assert_eq!(model.context_window, 1_000_000);
        assert_eq!(model.max_tokens, 128_000);
        assert!(model.reasoning);
    }

    #[test]
    fn a_price_per_token_becomes_a_price_per_million() {
        let model = &parse(&rich())[0];
        assert!(
            (model.cost.input - 5.0).abs() < 1e-9,
            "{}",
            model.cost.input
        );
        assert!(
            (model.cost.output - 25.0).abs() < 1e-9,
            "{}",
            model.cost.output
        );
    }

    #[test]
    fn a_bare_entry_is_kept_rather_than_dropped() {
        let found = parse(&bare());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "qwen3:8b");
        assert_eq!(found[0].name, "qwen3:8b", "the id stands in for a name");
        assert_eq!(found[0].context_window, ASSUMED_WINDOW);
        assert!(!found[0].reasoning);
    }

    #[test]
    fn an_answer_that_is_not_a_catalog_is_no_models_rather_than_a_panic() {
        assert!(parse(&serde_json::json!({ "error": "no" })).is_empty());
        assert!(parse(&serde_json::json!([])).is_empty());
        assert!(parse(&serde_json::json!({ "data": "nonsense" })).is_empty());
    }

    #[test]
    fn an_entry_with_no_id_is_skipped_and_the_rest_survive() {
        let body = serde_json::json!({ "data": [{ "name": "nameless" }, { "id": "real" }] });
        let found = parse(&body);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "real");
    }

    #[test]
    fn a_provider_id_cannot_escape_the_cache_directory() {
        let path = cache::path("models", &[b"../../etc/passwd"]).expect("a path");
        assert!(
            path.parent()
                .expect("cache directory")
                .ends_with("models/v2")
        );
        let name = path.file_stem().expect("filename").to_str().expect("UTF-8");
        assert_eq!(name.len(), 64);
        assert!(name.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }

    #[test]
    fn a_declared_catalog_is_never_asked_about() {
        let mut providers = vec![crate::mind::provider::endpoint::Provider {
            id: "fixed".into(),
            name: "Fixed".into(),
            base_url: Some("http://127.0.0.1:1/v1".into()),
            credential_origins: Vec::new(),
            headers: Default::default(),
            api: crate::mind::provider::model::Api::OpenAiCompletions,
            auth: crate::mind::provider::endpoint::Auth::None,
            compat: None,
            models: Vec::new(),
            discover: false,
            discovery: None,
            details: None,
            avoid: Vec::new(),
        }];
        discover(&mut providers);
        assert!(providers[0].models.is_empty());
    }

    #[test]
    fn a_discovered_model_speaks_its_providers_dialect() {
        use crate::mind::provider::compat::{Compat, ThinkingFormat, resolve};
        let provider = crate::mind::provider::endpoint::Provider {
            id: "router".into(),
            name: "Router".into(),
            base_url: None,
            credential_origins: Vec::new(),
            headers: Default::default(),
            api: crate::mind::provider::model::Api::OpenAiCompletions,
            auth: crate::mind::provider::endpoint::Auth::None,
            compat: Some(Compat {
                thinking_format: Some(ThinkingFormat::OpenRouter),
                ..Compat::default()
            }),
            models: Vec::new(),
            discover: true,
            discovery: None,
            details: None,
            avoid: Vec::new(),
        };
        let mut model = parse(&bare()).remove(0);
        model.base_url = Some("https://untrusted.invalid".into());
        model
            .headers
            .insert("x-untrusted".into(), "private-sentinel".into());
        let model = joined(model, &provider);
        assert!(model.base_url.is_none());
        assert!(model.headers.is_empty());
        assert_eq!(model.provider, "router");
        assert_eq!(
            resolve(model.compat).thinking_format,
            ThinkingFormat::OpenRouter
        );
    }
}

#[cfg(test)]
mod cached_price_tests {
    use super::*;
    use crate::mind::model::{Cost, Usage};

    #[test]
    fn published_free_and_cached_rates_survive_normalized_cache_roundtrips() {
        for (read, expected) in [
            (serde_json::json!("0"), 0.0),
            (serde_json::json!("0.000001"), 1.0),
            (serde_json::Value::Null, 3.0),
        ] {
            let models = parse(&serde_json::json!({"data":[{"id":"priced","pricing":{
                "prompt":"0.000003","completion":"0","input_cache_read":read}}]}));
            let cached =
                cached_models(&serde_json::to_string(&models).expect("cache")).expect("reload");
            let usage = Usage {
                cache_read: 1_000_000,
                ..Default::default()
            };
            assert_eq!(usage.price(&cached[0].cost), expected);
            assert!(cached[0].cost.presence.output);
            assert_eq!(cached[0].cost.presence.cache_read, !read.is_null());
        }
    }
    use serde_json::json;

    fn reload(cost: serde_json::Value) -> Model {
        let text = json!([{"id":"cached", "name":"Cached", "context_window":8192,
            "max_tokens":128, "cost":cost}])
        .to_string();
        cached_models(&text).expect("offline models").remove(0)
    }

    fn usage() -> Usage {
        Usage {
            cache_read: 1_000_000,
            cache_write: 1_000_000,
            ..Usage::default()
        }
    }

    #[test]
    fn legacy_generated_zero_prices_are_not_deliberately_free() {
        let model = reload(json!({"input":3,"output":15,"cache_read":0,"cache_write":0}));
        assert_eq!(usage().price(&model.cost), 6.0);
        let model = reload(json!({"input":3,"cache_read":1,"cache_write":5}));
        assert_eq!(usage().price(&model.cost), 6.0);
    }

    #[test]
    fn malformed_or_future_presence_only_invalidates_affected_price_metadata() {
        for presence in [
            json!(null),
            json!({}),
            json!({"version":"1"}),
            json!({"version":1,"cache_read":"yes"}),
            json!({"version":2,"cache_read":true,"cache_write":true}),
        ] {
            let model = reload(json!({"input":3,"cache_read":0,"cache_write":0,
                "presence":presence}));
            assert_eq!(model.id, "cached");
            assert_eq!(usage().price(&model.cost), 6.0);
        }
    }

    #[test]
    fn current_cache_roundtrips_preserve_unknown_explicit_zero_and_positive_rates() {
        for (cost, expected) in [
            (
                Cost {
                    input: 3.0,
                    ..Cost::default()
                },
                6.0,
            ),
            (
                serde_json::from_value(json!({"input":3,"cache_read":0,"cache_write":0}))
                    .expect("free source prices"),
                0.0,
            ),
            (
                serde_json::from_value(json!({"input":3,"cache_read":1,"cache_write":5}))
                    .expect("source prices"),
                6.0,
            ),
        ] {
            let model = reload(serde_json::to_value(&cost).expect("cache writer"));
            assert_eq!(usage().price(&model.cost), expected);
            assert_eq!(model.cost, cost);
        }
        let discovered = parse(&json!({"data":[{"id":"d","pricing":{"prompt":"0.000003"}}]}));
        let reloaded = cached_models(&serde_json::to_string(&discovered).expect("discovery cache"))
            .expect("offline reload");
        assert_eq!(usage().price(&reloaded[0].cost), 6.0);
    }
}

#[cfg(test)]
mod reasoning_tests {
    use super::*;

    fn card(reasoning: serde_json::Value) -> serde_json::Value {
        serde_json::json!({ "reasoning": reasoning })
    }

    #[test]
    fn a_model_that_must_reason_takes_the_least_it_will_do_as_its_off() {
        // Not "say nothing": with no effort named the provider applies `default_effort`, which
        // here is `max`. A helper on a small budget then spends all of it reasoning.
        let held = levels(&card(serde_json::json!({
            "mandatory": true,
            "default_effort": "max",
            "supported_efforts": ["max", "high", "low"],
        })));
        assert_eq!(held.get(&ThinkingLevel::Off), Some(&Some("low".to_owned())));
    }

    #[test]
    fn the_least_is_ranked_rather_than_taken_in_the_order_they_are_listed() {
        // OpenRouter lists these largest-first, so the last one is not reliably the smallest.
        assert_eq!(
            least(Some(&serde_json::json!({
                "supported_efforts": ["high", "medium", "low", "minimal"]
            }))),
            "minimal"
        );
        assert_eq!(
            least(Some(&serde_json::json!({
                "supported_efforts": ["max", "high"]
            }))),
            "high"
        );
    }

    #[test]
    fn a_model_that_names_no_efforts_still_gets_a_small_one() {
        // Mandatory with nothing said about which levels it takes: anything is better than
        // letting its own default stand.
        let held = levels(&card(serde_json::json!({ "mandatory": true })));
        assert_eq!(held.get(&ThinkingLevel::Off), Some(&Some("low".to_owned())));
    }

    #[test]
    fn a_model_that_may_reason_keeps_its_off() {
        let held = levels(&card(serde_json::json!({ "mandatory": false })));
        assert!(held.is_empty(), "{held:?}");
    }

    #[test]
    fn a_provider_that_says_nothing_about_it_is_left_alone() {
        // Every provider but OpenRouter says nothing here, and silence must not take a level away.
        assert!(levels(&serde_json::json!({})).is_empty());
        assert!(levels(&card(serde_json::json!("yes"))).is_empty());
        let held = levels(&card(serde_json::json!({ "supported_efforts": ["low"] })));
        assert_eq!(held.get(&ThinkingLevel::Low), Some(&Some("low".into())));
        assert_eq!(held.get(&ThinkingLevel::High), Some(&None));
    }
}
