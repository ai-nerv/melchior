//! One model in full, for a harness to draw a card from: what the catalog knows — its limits and
//! prices — and, from a provider that publishes it, what the model is for, how it scores, and who
//! serves it how reliably. Asked for on demand, never on a turn's path.

use crate::mind::provider::endpoint::Provider;
use crate::mind::provider::model::Model;
use serde::Serialize;
use serde_json::Value;

/// A model, described.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Described {
    /// `provider/model`, as a card id.
    pub id: String,
    pub name: String,
    pub context_window: u64,
    pub max_output: u64,
    pub reasons: bool,
    pub price: Price,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub knowledge_cutoff: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modality: Option<String>,
    pub benchmarks: Vec<Score>,
    pub endpoints: Vec<Serving>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokenizer: Option<String>,
    /// What it takes in and gives back: `text`, `image`, `audio`, `file`.
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    /// The request parameters it accepts, as the provider lists them: `tools`, `reasoning`, ….
    pub features: Vec<String>,
    /// When it was published, in Unix seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moderated: Option<bool>,
}

/// Dollars per million tokens.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Price {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

/// A benchmark score out of a hundred.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Score {
    pub name: String,
    pub score: f64,
}

/// One provider serving the model.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Serving {
    pub provider: String,
    /// The routing slug a request names it by, `open-inference/fp8`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantization: Option<String>,
    /// What this provider charges, which is not what the model's listing says.
    pub price: Price,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output: Option<u64>,
    /// Median time to the first token over the last half hour, in milliseconds as published.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<f64>,
    /// Median tokens a second over the last half hour.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub throughput: Option<f64>,
    /// Percent of the last half hour it answered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uptime_30m: Option<f64>,
}

/// Describe `model` of `provider`: the catalog's facts, and a provider's own where it publishes them.
#[must_use]
pub fn describe(provider: &Provider, model: &Model) -> Described {
    let mut out = Described {
        id: format!("{}/{}", provider.id, model.id),
        name: model.name.clone(),
        context_window: model.context_window,
        max_output: model.max_tokens,
        reasons: model.reasoning,
        price: Price {
            input: model.cost.input,
            output: model.cost.output,
            cache_read: model.cost.cache_read,
            cache_write: model.cost.cache_write,
        },
        ..Described::default()
    };
    // Only OpenRouter publishes all of this, and without a key; anyone else is told from the catalog.
    let Some(base) = provider
        .base_url
        .as_deref()
        .filter(|base| base.contains("openrouter.ai"))
    else {
        return out;
    };
    let key = crate::mind::discovering::key_for(provider);
    if let Some(entry) = listed(base, key.as_deref(), &model.id) {
        published(&mut out, &entry);
    }
    let url = format!(
        "{}/models/{}/endpoints",
        base.trim_end_matches('/'),
        model.id
    );
    if let Some(body) = get(&url, key.as_deref()) {
        out.endpoints = servings(&body);
    }
    out
}

/// This model's entry in the provider's `/models`, kept whole for a day: the list is large, and the
/// card is opened far more often than it changes.
fn listed(base: &str, key: Option<&str>, id: &str) -> Option<Value> {
    let path = crate::mind::discovering::cache_path("openrouter-listing");
    let kept = path.as_ref().and_then(|path| {
        let fresh = std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|when| std::time::SystemTime::now().duration_since(when).ok())
            .is_some_and(|age| age < crate::mind::discovering::FRESH);
        fresh
            .then(|| std::fs::read_to_string(path).ok())
            .flatten()
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
    });
    let body = match kept {
        Some(body) => body,
        None => {
            let body = get(&format!("{}/models", base.trim_end_matches('/')), key)?;
            if let Some(path) = &path {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(path, body.to_string());
            }
            body
        }
    };
    body.get("data")?
        .as_array()?
        .iter()
        .find(|entry| entry.get("id").and_then(Value::as_str) == Some(id))
        .cloned()
}

/// What a `/models` entry says beyond the catalog: what the model is, its cache prices, its output
/// ceiling, and the scores it carries.
fn published(out: &mut Described, entry: &Value) {
    let text = |at: &str| {
        entry
            .pointer(at)
            .and_then(Value::as_str)
            .filter(|said| !said.is_empty())
            .map(ToOwned::to_owned)
    };
    out.description = text("/description");
    out.knowledge_cutoff = text("/knowledge_cutoff");
    out.modality = text("/architecture/modality");
    out.tokenizer = text("/architecture/tokenizer");
    out.inputs = strings(entry, "/architecture/input_modalities");
    out.outputs = strings(entry, "/architecture/output_modalities");
    out.features = strings(entry, "/supported_parameters");
    out.created = entry.get("created").and_then(Value::as_u64);
    out.moderated = entry
        .pointer("/top_provider/is_moderated")
        .and_then(Value::as_bool);
    priced(&mut out.price, entry);
    if let Some(ceiling) = entry
        .pointer("/top_provider/max_completion_tokens")
        .and_then(Value::as_u64)
    {
        out.max_output = ceiling;
    }
    for (index, name) in [
        ("intelligence_index", "intelligence"),
        ("coding_index", "coding"),
        ("agentic_index", "agentic"),
    ] {
        let at = format!("/benchmarks/artificial_analysis/{index}");
        if let Some(score) = entry.pointer(&at).and_then(Value::as_f64) {
            out.benchmarks.push(Score {
                name: name.to_owned(),
                score,
            });
        }
    }
}

/// Every provider an `/endpoints` answer lists.
fn servings(body: &Value) -> Vec<Serving> {
    let Some(endpoints) = body.pointer("/data/endpoints").and_then(Value::as_array) else {
        return Vec::new();
    };
    endpoints
        .iter()
        .filter_map(|endpoint| {
            let mut price = Price::default();
            priced(&mut price, endpoint);
            Some(Serving {
                provider: endpoint.get("provider_name")?.as_str()?.to_owned(),
                tag: endpoint
                    .get("tag")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                quantization: endpoint
                    .get("quantization")
                    .and_then(Value::as_str)
                    .filter(|q| !q.is_empty() && *q != "unknown")
                    .map(ToOwned::to_owned),
                price,
                context: endpoint.get("context_length").and_then(Value::as_u64),
                max_output: endpoint
                    .get("max_completion_tokens")
                    .and_then(Value::as_u64),
                latency_ms: median(endpoint.get("latency_last_30m")),
                throughput: median(endpoint.get("throughput_last_30m")),
                uptime_30m: endpoint.get("uptime_last_30m").and_then(Value::as_f64),
            })
        })
        .collect()
}

/// A listing's or an endpoint's `pricing`, read into dollars per million: each is quoted per
/// token, as a string.
fn priced(price: &mut Price, entry: &Value) {
    for (key, into) in [
        ("prompt", &mut price.input),
        ("completion", &mut price.output),
        ("input_cache_read", &mut price.cache_read),
        ("input_cache_write", &mut price.cache_write),
    ] {
        if let Some(each) = entry
            .pointer(&format!("/pricing/{key}"))
            .and_then(Value::as_str)
            .and_then(|said| said.parse::<f64>().ok())
        {
            *into = each * 1_000_000.0;
        }
    }
}

/// A figure published either bare or as percentiles, as its median.
fn median(value: Option<&Value>) -> Option<f64> {
    let value = value?;
    value
        .as_f64()
        .or_else(|| value.get("p50").and_then(Value::as_f64))
}

/// A list of strings at `at`, empty where there is none.
fn strings(entry: &Value, at: &str) -> Vec<String> {
    entry
        .pointer(at)
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// `GET url` as JSON, on a thread with a runtime of its own, the way the catalog's own fetch runs.
fn get(url: &str, key: Option<&str>) -> Option<Value> {
    let url = url.to_owned();
    let key = key.map(ToOwned::to_owned);
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .ok()?;
        runtime.block_on(async move {
            let client = reqwest::Client::builder()
                .timeout(crate::mind::discovering::PATIENCE)
                .build()
                .ok()?;
            let mut request = client.get(&url);
            if let Some(key) = key {
                request = request.bearer_auth(key);
            }
            request.send().await.ok()?.json().await.ok()
        })
    })
    .join()
    .ok()
    .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_listing_entry_fills_in_what_the_catalog_does_not_know() {
        let entry = serde_json::json!({
            "id": "deepseek/deepseek-chat",
            "description": "A model.",
            "knowledge_cutoff": "2025-01-31",
            "architecture": {"modality": "text->text", "tokenizer": "DeepSeek",
                             "input_modalities": ["text", "image"], "output_modalities": ["text"]},
            "pricing": {"prompt": "0.0000003", "completion": "0.0000012", "input_cache_read": "0.00000003"},
            "top_provider": {"max_completion_tokens": 16384, "is_moderated": false},
            "supported_parameters": ["tools", "reasoning"],
            "created": 1_789_000_000,
            "benchmarks": {"artificial_analysis": {"coding_index": 37.6, "intelligence_index": 50.1}},
        });
        let mut out = Described::default();
        published(&mut out, &entry);
        assert_eq!(out.tokenizer.as_deref(), Some("DeepSeek"));
        assert_eq!(out.inputs, ["text", "image"]);
        assert_eq!(out.outputs, ["text"]);
        assert_eq!(out.features, ["tools", "reasoning"]);
        assert_eq!(out.created, Some(1_789_000_000));
        assert_eq!(out.moderated, Some(false));
        assert_eq!(out.description.as_deref(), Some("A model."));
        assert_eq!(out.modality.as_deref(), Some("text->text"));
        assert!((out.price.input - 0.3).abs() < 1e-9, "{:?}", out.price);
        assert!(
            (out.price.cache_read - 0.03).abs() < 1e-9,
            "{:?}",
            out.price
        );
        assert_eq!(out.max_output, 16_384);
        let names: Vec<&str> = out.benchmarks.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["intelligence", "coding"]);
    }

    #[test]
    fn every_endpoint_is_read_and_an_unknown_quantization_is_left_out() {
        let body = serde_json::json!({"data": {"endpoints": [
            {"provider_name": "DeepInfra", "quantization": "fp4", "uptime_last_30m": 99.1},
            {"provider_name": "Novita", "quantization": "unknown", "uptime_last_30m": null},
        ]}});
        let read = servings(&body);
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].quantization.as_deref(), Some("fp4"));
        assert_eq!(read[1].quantization, None);
        assert_eq!(read[1].uptime_30m, None);
    }

    #[test]
    fn an_endpoint_says_what_it_charges_how_much_it_holds_and_how_fast_it_is() {
        let body = serde_json::json!({"data": {"endpoints": [
            {"provider_name": "OpenInference", "tag": "open-inference/fp8",
             "pricing": {"prompt": "0.00000004", "completion": "0.0000001"},
             "context_length": 1_048_576, "max_completion_tokens": 393_216,
             "latency_last_30m": {"p50": 420.0, "p90": 900.0}, "throughput_last_30m": 85.5},
            {"provider_name": "Bare"},
        ]}});
        let read = servings(&body);
        let first = &read[0];
        assert_eq!(first.tag.as_deref(), Some("open-inference/fp8"));
        assert!((first.price.input - 0.04).abs() < 1e-9, "{:?}", first.price);
        assert!((first.price.output - 0.1).abs() < 1e-9, "{:?}", first.price);
        assert_eq!(first.context, Some(1_048_576));
        assert_eq!(first.max_output, Some(393_216));
        assert_eq!(first.latency_ms, Some(420.0), "the median of percentiles");
        assert_eq!(first.throughput, Some(85.5), "a bare figure as it is");
        assert_eq!(read[1].tag, None);
        assert_eq!(read[1].latency_ms, None);
    }
}
