use anyhow::{Context, Result, ensure};
use melchior::mind::model::Cost;
use serde_json::{Map, Value};
use std::collections::BTreeSet;

const RATES: &[(&str, &str)] = &[
    ("input", "input"),
    ("output", "output"),
    ("cacheRead", "cache_read"),
    ("cacheWrite", "cache_write"),
];

pub fn convert(raw: &Value, free: bool, pending: &mut BTreeSet<String>) -> Result<Value> {
    let source = raw.as_object().context("cost must be an object")?;
    let mut cost = rates(source, free)?;
    for field in source.keys() {
        if field != "tiers" && !RATES.iter().any(|(name, _)| field == name) {
            pending.insert(format!("cost.{field}"));
        }
    }
    if let Some(tiers) = source.get("tiers") {
        let tiers = tiers.as_array().context("cost tiers must be an array")?;
        ensure!(tiers.len() <= 64, "too many cost tiers");
        let mut converted = Vec::new();
        for (index, tier) in tiers.iter().enumerate() {
            let tier = tier.as_object().context("cost tier must be an object")?;
            let mut mapped = rates(tier, free)?;
            let above = tier
                .get("inputTokensAbove")
                .and_then(Value::as_u64)
                .context("tier threshold must be an unsigned integer")?;
            mapped.insert("input_tokens_above".into(), above.into());
            for field in tier.keys() {
                if field != "inputTokensAbove" && !RATES.iter().any(|(name, _)| field == name) {
                    pending.insert(format!("cost.tiers[{index}].{field}"));
                }
            }
            for (upstream, _) in RATES {
                if !free
                    && tier.get(*upstream).and_then(Value::as_f64) == Some(0.0)
                    && source
                        .get(*upstream)
                        .and_then(Value::as_f64)
                        .is_some_and(|rate| rate > 0.0)
                {
                    pending.insert(format!("cost.tiers[{index}].{upstream}:unknown-override"));
                }
            }
            converted.push(Value::Object(mapped));
        }
        cost.insert("tiers".into(), converted.into());
    }
    let cost: Cost =
        serde_json::from_value(Value::Object(cost)).context("invalid native cost schedule")?;
    Ok(serde_json::to_value(cost)?)
}

fn rates(source: &Map<String, Value>, free: bool) -> Result<Map<String, Value>> {
    let mut rates = Map::new();
    for (upstream, native) in RATES {
        if let Some(value) = source.get(*upstream) {
            let rate = value.as_f64().context("cost rate must be numeric")?;
            ensure!(
                rate.is_finite() && rate >= 0.0,
                "cost rate must be finite and nonnegative"
            );
            ensure!(!free || rate == 0.0, "free model declares a positive price");
            if rate > 0.0 || free {
                rates.insert((*native).into(), value.clone());
            }
        }
    }
    Ok(rates)
}
