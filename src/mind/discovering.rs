//! Asking a provider what it offers, and remembering the answer. A fetch never writes back to
//! `providers.lua`; what it finds goes to a cache under `$XDG_CACHE_HOME/melchior/models/`. That
//! cache is read at load time. Missing, expired, or explicitly refreshed catalogs are fetched;
//! a failed fetch leaves whatever the cache last held.

use crate::mind::model::ThinkingLevel;
use crate::mind::provider::endpoint::Provider;
use crate::mind::provider::model::Model;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

mod capabilities;
mod ollama;

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
    let mut failed = Vec::new();
    for provider in providers.iter_mut() {
        if !provider.discover {
            continue;
        }
        let Some(base) = provider.base_url.clone() else {
            continue;
        };
        let found = cached(&provider.id)
            .filter(|held| held.fresh && !refresh)
            .map(|held| held.models)
            .or_else(|| {
                let key = key_for(provider);
                let popularity = provider.details.as_deref() == Some("openrouter")
                    || reqwest::Url::parse(&base).is_ok_and(|url| {
                        url.host_str() == Some("openrouter.ai")
                            && url.path().trim_end_matches('/') == "/api/v1"
                    });
                let fetched = fetch(
                    &base,
                    key.as_deref(),
                    provider.details.as_deref(),
                    popularity,
                );
                match fetched {
                    Some(models) => {
                        write_cache(&provider.id, &models);
                        Some(models)
                    }
                    // A failed fetch falls back to a stale cache.
                    _ => {
                        failed.push(provider.id.clone());
                        cached(&provider.id).map(|held| held.models)
                    }
                }
            });
        if let Some(found) = found {
            provider.models = found
                .into_iter()
                .map(|model| joined(model, provider))
                .collect();
        }
    }
    failed
}

/// A discovered model as its provider serves it: the provider's id, protocol and dialect. Without
/// the dialect it spoke plain OpenAI, and a reasoning level, off included, never reached the provider.
fn joined(mut model: Model, provider: &crate::mind::provider::endpoint::Provider) -> Model {
    model.provider.clone_from(&provider.id);
    model.api = provider.api;
    model.compat = Some(model.compat.unwrap_or_default().over(provider.compat));
    model
}

/// A cached catalog, and whether it is still worth using without asking again.
struct Held {
    models: Vec<Model>,
    fresh: bool,
}

/// Where a provider's fetched catalog is kept.
pub(crate) fn cache_path(id: &str) -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    // The provider id comes from a config, so it is flattened to a single path segment.
    let safe: String = id
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    Some(
        base.join("melchior")
            .join("models")
            .join(format!("{safe}.json")),
    )
}

/// Read what was last fetched for this provider.
fn cached(id: &str) -> Option<Held> {
    let path = cache_path(id)?;
    let raw = std::fs::read_to_string(&path).ok()?;
    let models = cached_models(&raw)?;
    let fresh = std::fs::metadata(&path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|when| SystemTime::now().duration_since(when).ok())
        .is_some_and(|age| age < FRESH);
    Some(Held { models, fresh })
}

fn cached_models(raw: &str) -> Option<Vec<Model>> {
    let mut values: Vec<serde_json::Value> = serde_json::from_str(raw).ok()?;
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
    values
        .into_iter()
        .map(|value| serde_json::from_value(value).ok())
        .collect()
}

/// Keep what was fetched, so the next start does not have to ask.
fn write_cache(id: &str, models: &[Model]) {
    let Some(path) = cache_path(id) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string(models) {
        let _ = std::fs::write(&path, text);
    }
}

/// The key this provider authenticates with, if the environment has one.
pub(crate) fn key_for(provider: &Provider) -> Option<String> {
    let crate::mind::provider::endpoint::Auth::ApiKey { vars } = &provider.auth else {
        return None;
    };
    vars.iter().find_map(|name| std::env::var(name).ok())
}

/// Ask `<base>/models` what there is, on a thread with a runtime of its own: the synchronous
/// caller is reached from an async daemon too, and building a runtime inside a running one
/// panics.
fn fetch(
    base: &str,
    key: Option<&str>,
    details: Option<&str>,
    popularity: bool,
) -> Option<Vec<Model>> {
    let mut url = reqwest::Url::parse(&format!("{}/models", base.trim_end_matches('/'))).ok()?;
    if popularity {
        url.query_pairs_mut().append_pair("sort", "most-popular");
    }
    let key = key.map(ToOwned::to_owned);
    let ollama = (details == Some("ollama")).then(|| ollama::root(base));
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .ok()?;
        runtime.block_on(async move {
            let client = crate::mind::provider::http()
                .timeout(PATIENCE)
                .build()
                .ok()?;
            let mut request = client.get(url);
            if let Some(key) = key {
                request = request.bearer_auth(key);
            }
            let body: serde_json::Value = request
                .send()
                .await
                .ok()?
                .error_for_status()
                .ok()?
                .json()
                .await
                .ok()?;
            let entries = body.get("data")?.as_array()?;
            let listed = ranked(&body, popularity);
            if !entries.is_empty() && listed.is_empty() {
                return None;
            }
            // Fetch Ollama metadata before storing the refreshed cards.
            let Some(root) = ollama else {
                return Some(listed);
            };
            let mut detailed = Vec::with_capacity(listed.len());
            for model in listed {
                detailed.push(ollama::detailed(model, &root, &client).await);
            }
            Some(detailed)
        })
    })
    .join()
    .ok()
    .flatten()
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
        // Overwritten by the provider, which knows its own id and protocol.
        provider: String::new(),
        api: crate::mind::provider::model::Api::OpenAiCompletions,
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
    if mandatory {
        out.insert(ThinkingLevel::Off, Some(least(reasoning)));
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
    entry
        .get("supported_parameters")
        .and_then(|p| p.as_array())
        .is_some_and(|params| {
            params
                .iter()
                .filter_map(|p| p.as_str())
                .any(|p| p == "reasoning" || p == "include_reasoning" || p == "reasoning_effort")
        })
}

/// Price per million tokens, from the dollars-per-token decimal string providers quote; an
/// unreadable price is no price rather than a wrong one.
fn cost(entry: &serde_json::Value) -> crate::mind::model::Cost {
    crate::mind::model::Cost {
        input: rate(entry, "prompt").unwrap_or_default(),
        output: rate(entry, "completion").unwrap_or_default(),
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
        let path = cache_path("../../etc/passwd").expect("a path");
        assert!(
            path.ends_with("------etc-passwd.json"),
            "{}",
            path.display()
        );
    }

    #[test]
    fn a_declared_catalog_is_never_asked_about() {
        let mut providers = vec![crate::mind::provider::endpoint::Provider {
            id: "fixed".into(),
            name: "Fixed".into(),
            base_url: Some("http://127.0.0.1:1/v1".into()),
            api: crate::mind::provider::model::Api::OpenAiCompletions,
            auth: crate::mind::provider::endpoint::Auth::None,
            compat: None,
            models: Vec::new(),
            discover: false,
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
            api: crate::mind::provider::model::Api::OpenAiCompletions,
            auth: crate::mind::provider::endpoint::Auth::None,
            compat: Some(Compat {
                thinking_format: Some(ThinkingFormat::OpenRouter),
                ..Compat::default()
            }),
            models: Vec::new(),
            discover: true,
            details: None,
            avoid: Vec::new(),
        };
        let model = joined(parse(&bare()).remove(0), &provider);
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
        assert_eq!(usage().price(model.cost), 6.0);
        let model = reload(json!({"input":3,"cache_read":1,"cache_write":5}));
        assert_eq!(usage().price(model.cost), 6.0);
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
            assert_eq!(usage().price(model.cost), 6.0);
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
            let model = reload(serde_json::to_value(cost).expect("cache writer"));
            assert_eq!(usage().price(model.cost), expected);
            assert_eq!(model.cost, cost);
        }
        let discovered = parse(&json!({"data":[{"id":"d","pricing":{"prompt":"0.000003"}}]}));
        let reloaded = cached_models(&serde_json::to_string(&discovered).expect("discovery cache"))
            .expect("offline reload");
        assert_eq!(usage().price(reloaded[0].cost), 6.0);
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
        assert!(levels(&card(serde_json::json!({ "supported_efforts": ["low"] }))).is_empty());
    }
}
