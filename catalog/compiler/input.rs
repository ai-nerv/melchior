use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::Path;

const LIMIT: usize = 8 * 1024 * 1024;

pub fn records(raw: &[u8], id: &str, digest: &str) -> Result<Vec<Value>> {
    ensure!(raw.len() <= LIMIT, "catalog source exceeds its size limit");
    ensure!(
        format!("{:x}", Sha256::digest(raw)) == digest,
        "catalog source digest differs"
    );
    let catalog: Value = serde_json::from_slice(raw).context("catalog is not JSON")?;
    let mut identities = BTreeSet::new();
    let mut rows = Vec::new();
    for (api, models) in catalog.as_object().context("catalog API map is missing")? {
        for (key, model) in models.as_object().context("catalog model map is missing")? {
            let name = text(model, "id")?;
            let operation = text(model, "type")?;
            ensure!(
                !name.trim().is_empty()
                    && !name.chars().any(char::is_control)
                    && matches!(operation, "chat" | "classifier" | "image")
                    && text(model, "provider")? == id
                    && text(model, "api")? == api
                    && *key == format!("{operation}:{name}")
                    && identities.insert((operation.to_owned(), name.to_owned())),
                "catalog identity is inconsistent"
            );
            rows.push(model.clone());
            ensure!(rows.len() <= 4096, "catalog source exceeds its model limit");
        }
    }
    Ok(rows)
}

pub fn read(root: &Path, manifest: &Value) -> Result<BTreeMap<String, Vec<Value>>> {
    let providers = manifest
        .get("providers")
        .and_then(Value::as_array)
        .context("providers")?;
    let mut expected = BTreeSet::new();
    let mut all = BTreeMap::new();
    let mut totals = BTreeMap::new();
    for provider in providers {
        let id = text(provider, "id")?;
        ensure!(
            !id.is_empty()
                && id
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-'),
            "invalid provider identity"
        );
        ensure!(
            expected.insert(format!("{id}.json")),
            "duplicate provider identity"
        );
        let catalog = provider
            .pointer("/models/catalog")
            .context("catalog provenance")?;
        ensure!(
            text(catalog, "path")? == format!("package/dist/providers/data/{id}.json"),
            "catalog provenance path differs"
        );
        let path = root.join(format!("{id}.json"));
        ensure!(
            std::fs::symlink_metadata(&path)?.file_type().is_file(),
            "catalog source is not a regular file"
        );
        let mut raw = Vec::new();
        std::fs::File::open(&path)?
            .take((LIMIT + 1) as u64)
            .read_to_end(&mut raw)?;
        let rows = records(&raw, id, text(catalog, "sha256")?)
            .with_context(|| format!("provider {id}"))?;
        let mut counts = BTreeMap::new();
        let mut apis = BTreeSet::new();
        for row in &rows {
            *counts
                .entry(text(row, "type")?.to_owned())
                .or_insert(0_usize) += 1;
            *totals
                .entry(text(row, "type")?.to_owned())
                .or_insert(0_usize) += 1;
            apis.insert(text(row, "api")?.to_owned());
        }
        ensure!(
            catalog.get("records") == Some(&serde_json::to_value(counts)?),
            "catalog operation counts differ for {id}"
        );
        ensure!(
            catalog.get("apis") == Some(&serde_json::to_value(apis)?),
            "catalog APIs differ for {id}"
        );
        all.insert(id.to_owned(), rows);
    }
    let actual = std::fs::read_dir(root)?
        .map(|entry| {
            let name = entry?.file_name();
            name.into_string()
                .map_err(|_| anyhow::anyhow!("invalid catalog filename"))
        })
        .collect::<Result<BTreeSet<_>>>()?;
    ensure!(actual == expected, "catalog file inventory differs");
    ensure!(
        manifest.pointer("/reference/model_catalog/records")
            == Some(&serde_json::to_value(totals)?),
        "catalog totals differ"
    );
    Ok(all)
}

pub fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    match value.get(key).and_then(Value::as_str) {
        Some(text) => Ok(text),
        None => bail!("missing text field {key}"),
    }
}
