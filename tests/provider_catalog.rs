#![allow(clippy::expect_used, clippy::indexing_slicing)]

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const MANIFEST: &str = include_str!("provider-parity/manifest.json");
const LIMIT: usize = 8 * 1024 * 1024;

#[path = "../catalog/compiler/mod.rs"]
mod compiler;
use compiler::input::records;

#[path = "provider_catalog/conversion.rs"]
mod conversion;

#[path = "provider_catalog/anthropic.rs"]
mod anthropic;

#[path = "provider_catalog/reasoning_content.rs"]
mod reasoning_content;

#[path = "provider_catalog/instructions.rs"]
mod instructions;

#[path = "provider_catalog/tool_history.rs"]
mod tool_history;

#[path = "provider_catalog/zai.rs"]
mod zai;

#[path = "provider_catalog/moonshot.rs"]
mod moonshot;

#[path = "provider_catalog/session.rs"]
mod session;

#[path = "provider_catalog/baseten.rs"]
mod baseten;

#[path = "provider_catalog/fireworks.rs"]
mod fireworks;

#[path = "provider_catalog/qwen.rs"]
mod qwen;

#[path = "provider_catalog/together.rs"]
mod together;

#[path = "provider_catalog/huggingface.rs"]
mod huggingface;

#[path = "provider_catalog/ant_ling.rs"]
mod ant_ling;

#[path = "provider_catalog/google_thinking.rs"]
mod google_thinking;

#[path = "provider_catalog/mistral.rs"]
mod mistral;

#[path = "provider_catalog/strict.rs"]
mod strict;

#[path = "provider_catalog/input_limits.rs"]
mod input_limits;

fn counts(rows: &[Value]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for row in rows {
        *counts
            .entry(row["type"].as_str().expect("operation").to_owned())
            .or_default() += 1;
    }
    counts
}

#[test]
fn pinned_catalog_files_match_the_manifest_without_executing_or_fetching_upstream_code() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("catalog/reference/pi-1.6.0/data");
    let manifest: Value = serde_json::from_str(MANIFEST).expect("manifest");
    let providers = manifest["providers"].as_array().expect("providers");
    let expected_files: BTreeSet<_> = providers
        .iter()
        .map(|row| format!("{}.json", row["id"].as_str().expect("provider ID")))
        .collect();
    let actual_files: BTreeSet<_> = std::fs::read_dir(&root)
        .expect("reference data")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_str()
                .expect("file name")
                .to_owned()
        })
        .collect();
    assert_eq!(actual_files, expected_files);
    assert_eq!(providers.len(), 43);
    let mut all = Vec::new();
    for provider in providers {
        let id = provider["id"].as_str().expect("ID");
        assert!(
            id.bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        );
        let declaration = &provider["models"]["catalog"];
        assert_eq!(
            declaration["path"],
            format!("package/dist/providers/data/{id}.json")
        );
        let raw = std::fs::read(root.join(format!("{id}.json"))).expect("source data");
        let models = records(&raw, id, declaration["sha256"].as_str().expect("digest"))
            .expect("pinned catalog");
        assert_eq!(
            serde_json::to_value(counts(&models)).expect("counts"),
            declaration["records"],
            "{id}"
        );
        let apis: BTreeSet<_> = models
            .iter()
            .map(|model| model["api"].as_str().expect("API"))
            .collect();
        assert_eq!(
            serde_json::to_value(apis).expect("APIs"),
            declaration["apis"],
            "{id}"
        );
        all.extend(models);
    }
    assert_eq!(
        serde_json::to_value(counts(&all)).expect("counts"),
        manifest["reference"]["model_catalog"]["records"]
    );
    let scoped: Vec<_> = all
        .iter()
        .filter(|model| model["type"] != "image")
        .collect();
    assert_eq!(scoped.len(), 1779);
    assert_eq!(
        scoped
            .iter()
            .filter(|model| model["cost"].get("tiers").is_some())
            .count(),
        303
    );
    assert_eq!(
        scoped
            .iter()
            .filter(|model| model.get("headers").is_some())
            .count(),
        48
    );
    assert_eq!(
        scoped
            .iter()
            .filter(|model| model["cost"]["input"].as_f64() == Some(0.0)
                && model["cost"]["output"].as_f64() == Some(0.0))
            .count(),
        143
    );
    assert_eq!(
        scoped.iter().filter(|model| model["free"] == true).count(),
        1
    );
    let compat: BTreeSet<_> = scoped
        .iter()
        .flat_map(|model| {
            model["compat"]
                .as_object()
                .into_iter()
                .flat_map(|map| map.keys())
        })
        .collect();
    assert_eq!(compat.len(), 30);
    let mut identities = BTreeMap::<String, BTreeSet<String>>::new();
    for model in scoped {
        let id = format!(
            "{}/{}",
            model["provider"].as_str().expect("provider"),
            model["id"].as_str().expect("model")
        );
        identities
            .entry(id)
            .or_default()
            .insert(model["type"].as_str().expect("operation").into());
    }
    let collisions: BTreeMap<_, _> = identities
        .into_iter()
        .filter(|(_, operations)| operations.len() > 1)
        .collect();
    assert_eq!(
        serde_json::to_value(collisions).expect("collisions"),
        json!({"openai/gpt-6-luna":["chat","classifier"]})
    );
}

#[test]
fn input_audit_refuses_digest_drift_and_inconsistent_operation_identity() {
    let model = json!({"provider":"fixture","api":"openai-completions","id":"one","type":"chat"});
    let good = json!({"openai-completions":{"chat:one":model}});
    let raw = serde_json::to_vec(&good).expect("json");
    let digest = format!("{:x}", Sha256::digest(&raw));
    assert_eq!(records(&raw, "fixture", &digest).expect("record").len(), 1);
    let mut changed = raw.clone();
    changed.push(b' ');
    assert!(records(&changed, "fixture", &digest).is_err());
    assert!(records(&vec![b' '; LIMIT + 1], "fixture", &digest).is_err());
    for (field, value) in [
        ("provider", "other"),
        ("api", "other"),
        ("type", "unknown"),
        ("type", "classifier"),
        ("id", ""),
        ("id", "bad\n"),
    ] {
        let mut bad = good.clone();
        bad["openai-completions"]["chat:one"][field] = json!(value);
        let raw = serde_json::to_vec(&bad).expect("json");
        let digest = format!("{:x}", Sha256::digest(&raw));
        assert!(
            records(&raw, "fixture", &digest).is_err(),
            "{field}={value:?}"
        );
    }
    let mut duplicate = good.clone();
    let mut other = model.clone();
    other["api"] = json!("openai-responses");
    duplicate["openai-responses"] = json!({"chat:one":other});
    let raw = serde_json::to_vec(&duplicate).expect("json");
    assert!(records(&raw, "fixture", &format!("{:x}", Sha256::digest(&raw))).is_err());
}
