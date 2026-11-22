use serde::Deserialize;
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::LazyLock,
};

const SOURCE: &str = "pi-1.6.0";
const BYTES: &str = include_str!("../../../catalog/generated/pi-1.6.0.json");
const MOST: usize = 4096;
const KEY_BYTES: usize = 2048;
static SNAPSHOT: LazyLock<Result<Snapshot, &'static str>> =
    LazyLock::new(|| Snapshot::parse(BYTES));

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    source: String,
    provider: String,
    models: Vec<String>,
}

#[derive(Deserialize)]
struct Record {
    id: String,
    source_key: String,
    operation: String,
    pending: Vec<String>,
    candidate: Map<String, Value>,
}

struct Snapshot(BTreeMap<String, BTreeMap<String, Record>>);

impl Snapshot {
    fn parse(bytes: &str) -> Result<Self, &'static str> {
        #[derive(Deserialize)]
        struct Rows {
            id: String,
            models: Vec<Record>,
        }
        #[derive(Deserialize)]
        struct Report {
            schema: u32,
            reference: Value,
            providers: Vec<Rows>,
        }
        if bytes.len() > 8 * 1024 * 1024 {
            return Err("embedded catalog exceeds its byte limit");
        }
        let report: Report = serde_json::from_str(bytes).map_err(|_| "invalid embedded catalog")?;
        if report.schema != 1
            || report
                .reference
                .pointer("/model_catalog/version")
                .and_then(Value::as_str)
                != Some("1.6.0")
            || report.providers.len() > 128
        {
            return Err("unsupported embedded catalog version or provider count");
        }
        let mut providers = BTreeMap::new();
        let mut total = 0_usize;
        for provider in report.providers {
            if provider.id.is_empty() || provider.id.len() > 256 {
                return Err("invalid embedded catalog provider identity");
            }
            let mut records = BTreeMap::new();
            for record in provider.models {
                total += 1;
                if total > 8192
                    || record.source_key.len() > KEY_BYTES
                    || record.id.is_empty()
                    || !matches!(record.operation.as_str(), "chat" | "classifier")
                    || record.source_key != format!("{}:{}", record.operation, record.id)
                    || record.candidate.get("id").and_then(Value::as_str)
                        != Some(record.id.as_str())
                    || record.candidate.get("provider").and_then(Value::as_str)
                        != Some(provider.id.as_str())
                {
                    return Err("invalid embedded catalog model identity or count");
                }
                if records.insert(record.source_key.clone(), record).is_some() {
                    return Err("duplicate embedded catalog model identity");
                }
            }
            if providers.insert(provider.id, records).is_some() {
                return Err("duplicate embedded catalog provider identity");
            }
        }
        Ok(Self(providers))
    }

    fn selected(&self, selection: &Selection) -> Result<Vec<Value>, &'static str> {
        if selection.source != SOURCE {
            return Err("catalog.source must name an embedded pinned source");
        }
        let records = self
            .0
            .get(&selection.provider)
            .ok_or("catalog.provider is not in the pinned source")?;
        let mut seen = BTreeSet::new();
        let mut models = Vec::with_capacity(selection.models.len());
        for key in &selection.models {
            if !seen.insert(key) {
                return Err("catalog.models contains duplicate identities");
            }
            let record = records
                .get(key)
                .ok_or("catalog.models contains an unknown operation-qualified identity")?;
            if record.operation != "chat" {
                return Err(
                    "catalog.models selected an operation not supported by the chat model role",
                );
            }
            if !record.pending.is_empty() {
                return Err(
                    "catalog.models selected a record with unresolved mappings; inspect the pinned candidate report",
                );
            }
            let mut model = record.candidate.clone();
            model.remove("capabilities");
            models.push(Value::Object(model));
        }
        Ok(models)
    }
}

pub(super) fn apply(provider: &mut Map<String, Value>) -> Result<(), &'static str> {
    let Some(raw) = provider.get("catalog") else {
        return Ok(());
    };
    if raw
        .get("source")
        .and_then(Value::as_str)
        .is_none_or(|source| source.len() > 64)
        || raw
            .get("provider")
            .and_then(Value::as_str)
            .is_none_or(|provider| provider.len() > 256)
    {
        return Err("catalog source and provider must be bounded strings");
    }
    let keys = raw
        .get("models")
        .and_then(Value::as_array)
        .filter(|keys| !keys.is_empty() && keys.len() <= MOST)
        .ok_or("catalog.models must explicitly select between 1 and 4096 identities")?;
    if keys
        .iter()
        .any(|key| key.as_str().is_none_or(|key| key.len() > KEY_BYTES))
    {
        return Err("catalog.models identities must be strings of at most 2048 bytes");
    }
    let selection = Selection::deserialize(raw)
        .map_err(|_| "catalog requires source, provider and models only")?;
    let snapshot = SNAPSHOT.as_ref().map_err(|error| *error)?;
    let mut models = snapshot.selected(&selection)?;
    let overrides = match provider.get("models") {
        None => &[][..],
        Some(Value::Array(models)) if models.len() <= MOST => models.as_slice(),
        Some(Value::Object(models)) if models.is_empty() => &[][..],
        _ => return Err("models must be a list of at most 4096 declarations"),
    };
    let mut positions: BTreeMap<String, usize> = models
        .iter()
        .enumerate()
        .map(|(index, model)| {
            let id = model.get("id").and_then(Value::as_str).unwrap_or_default();
            (id.to_owned(), index)
        })
        .collect();
    let mut seen = BTreeSet::new();
    for model in overrides {
        let fields = model
            .as_object()
            .ok_or("models entries must be declaration tables")?;
        let id = fields
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty() && id.len() <= KEY_BYTES)
            .ok_or("models entries require a nonempty bounded id")?;
        if !seen.insert(id) {
            return Err("models contains duplicate identities");
        }
        if let Some(index) = positions.get(id).copied() {
            let target = models
                .get_mut(index)
                .and_then(Value::as_object_mut)
                .ok_or("invalid imported model declaration")?;
            target.extend(
                fields
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone())),
            );
        } else {
            if models.len() >= MOST {
                return Err("combined catalog and local models exceed 4096 entries");
            }
            positions.insert(id.to_owned(), models.len());
            models.push(model.clone());
        }
    }
    provider.remove("catalog");
    provider.insert("models".into(), Value::Array(models));
    Ok(())
}

#[cfg(test)]
mod tests;
