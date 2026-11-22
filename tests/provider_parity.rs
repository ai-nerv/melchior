#![allow(clippy::expect_used, clippy::indexing_slicing)]

use melchior::mind::catalog::Catalog;
use melchior::mind::lua::engine::Engine;
use melchior::mind::provider::endpoint::Provider;
use melchior::scratch::Scratch;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[path = "provider-parity/transport.rs"]
mod transport;

#[path = "provider-parity/receipt.rs"]
mod receipt;

const MANIFEST: &str = include_str!("provider-parity/manifest.json");
const REGISTRY: &str = include_str!("provider-parity/reference-registry.ts");
const CASES: &[&str] = &[
    "catalog",
    "authentication",
    "text",
    "tool-loop",
    "handoff",
    "classification",
    "usage",
    "cancellation",
    "errors",
    "discovery",
    "capabilities",
];

#[derive(Deserialize)]
struct Manifest {
    schema: u32,
    reference: Reference,
    providers: Vec<Entry>,
    retained: Vec<Entry>,
}

#[derive(Deserialize)]
struct Reference {
    revision: String,
    registry_sha256: String,
}

#[derive(Deserialize)]
struct Entry {
    id: String,
    apis: Vec<String>,
    operations: Vec<String>,
    required_cases: Vec<String>,
    source: Option<Source>,
    live: Value,
}

#[derive(Deserialize)]
struct Source {
    path: String,
    sha256: String,
}

fn reference_ids() -> BTreeSet<String> {
    let mut imports = BTreeMap::new();
    for line in REGISTRY.lines() {
        if let Some((function, file)) = line
            .strip_prefix("import { ")
            .and_then(|line| line.split_once(" } from \"./"))
            && function.ends_with("Provider")
            && let Some(file) = file.strip_suffix(".ts\";")
        {
            imports.insert(function, file);
        }
    }
    let body = REGISTRY
        .split_once("export function builtinProviders()")
        .expect("pinned registry function")
        .1
        .split_once("\n\t];")
        .expect("pinned provider list")
        .0;
    body.lines()
        .filter_map(|line| line.trim().strip_suffix("(),"))
        .map(|name| {
            imports
                .get(name)
                .expect("registered provider import")
                .to_string()
        })
        .collect()
}

fn manifest() -> Manifest {
    let manifest: Manifest = serde_json::from_str(MANIFEST).expect("parity manifest");
    assert_eq!(manifest.schema, 1);
    assert_eq!(manifest.reference.revision.len(), 40);
    assert!(
        manifest
            .reference
            .revision
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    );
    assert_eq!(
        manifest.reference.registry_sha256,
        format!("{:x}", Sha256::digest(REGISTRY.as_bytes()))
    );
    let listed: BTreeSet<_> = manifest
        .providers
        .iter()
        .map(|row| row.id.clone())
        .collect();
    assert_eq!(
        listed,
        reference_ids(),
        "manifest must match the pinned executable registry"
    );
    assert_eq!(
        listed.len(),
        manifest.providers.len(),
        "duplicate reference provider"
    );
    let mut all = listed;
    for row in &manifest.retained {
        assert!(all.insert(row.id.clone()), "duplicate retained provider");
    }
    assert!(all.contains("ollama") && all.contains("decisions"));
    for row in manifest.providers.iter().chain(&manifest.retained) {
        assert!(!row.apis.is_empty() && !row.operations.is_empty());
        let cases: BTreeSet<_> = row.required_cases.iter().map(String::as_str).collect();
        assert_eq!(cases.len(), row.required_cases.len());
        assert!(cases.contains("catalog") && cases.contains("authentication"));
        assert!(
            cases.contains("usage") && cases.contains("cancellation") && cases.contains("errors")
        );
        assert!(cases.iter().all(|case| CASES.contains(case)));
        if row.operations.iter().any(|operation| operation == "chat") {
            assert!(
                cases.contains("text") && cases.contains("tool-loop") && cases.contains("handoff")
            );
        }
        if row
            .operations
            .iter()
            .any(|operation| operation == "classifier")
        {
            assert!(cases.contains("classification"));
        }
        if let Some(source) = &row.source {
            assert_eq!(
                source.path,
                format!("packages/ai/src/providers/{}.ts", row.id)
            );
            assert_eq!(source.sha256.len(), 64);
            assert!(source.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()));
        } else {
            assert!(manifest.retained.iter().any(|extra| extra.id == row.id));
        }
        assert!(matches!(
            row.live["status"].as_str(),
            Some("not-run" | "blocked" | "verified")
        ));
        if row.live["status"] == "verified" {
            assert!(
                !row.live["receipts"]
                    .as_array()
                    .expect("live receipts")
                    .is_empty()
            );
        }
    }
    manifest
}

fn declarations() -> BTreeMap<String, Provider> {
    let mut engine = Engine::new();
    engine
        .run(include_str!("../config/apis.lua"), "apis.lua")
        .expect("shipped APIs");
    engine
        .run(include_str!("../config/providers.lua"), "providers.lua")
        .expect("shipped providers");
    engine.harvest();
    let dir = Scratch::new("melchior-parity", "declarations");
    let mut offline = String::new();
    for (id, value) in engine.config().all("provider") {
        if value["discovery"]["kind"] != "bundled"
            && (value["discover"] == true || value.get("discovery").is_some())
        {
            let mut value = value.clone();
            value["discover"] = json!(false);
            value["discovery"] = json!({"kind":"bundled"});
            let encoded = serde_json::to_string(&value.to_string()).expect("Lua JSON string");
            offline.push_str(&format!(
                "melchior.provider({id:?}, melchior.json.decode({encoded}))\n"
            ));
        }
    }
    std::fs::write(dir.join("providers.lua"), offline).expect("offline declaration overlay");
    let catalog = Catalog::load(&dir).expect("assembled shipped declarations");
    assert!(catalog.discovery_failed.is_empty());
    catalog
        .providers
        .into_iter()
        .map(|provider| (provider.id.clone(), provider))
        .collect()
}

#[test]
fn pinned_manifest_covers_the_reference_registry_and_retained_providers() {
    manifest();
}

#[test]
fn provider_parity_contract() {
    let dir = Scratch::new("melchior-parity", "isolated");
    let report = dir.join("report.json");
    let output = dir.join("output.log");
    let file = std::fs::File::create(&output).expect("worker output");
    let mut child = Command::new(std::env::current_exe().expect("test executable"))
        .env_clear()
        .env("HOME", &*dir)
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env("XDG_DATA_HOME", dir.join("data"))
        .env("XDG_CACHE_HOME", dir.join("cache"))
        .env("XDG_RUNTIME_DIR", dir.join("runtime"))
        .env("MELCHIOR_PROVIDER_PARITY_REPORT", &report)
        .current_dir(&*dir)
        .args(["--exact", "isolated_worker", "--nocapture"])
        .stdin(Stdio::null())
        .stdout(file.try_clone().expect("worker stdout"))
        .stderr(file)
        .spawn()
        .expect("isolated parity worker");
    let deadline = Instant::now() + Duration::from_secs(120);
    let status = loop {
        if let Some(status) = child.try_wait().expect("worker status") {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let receipt = receipt::retain(&report, &output, "timed-out");
            panic!(
                "provider parity worker exceeded its deadline; receipt: {}",
                receipt.display()
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let receipt = receipt::retain(&report, &output, &status.to_string());
    println!("provider parity receipt: {}", receipt.display());
    let log = std::fs::read_to_string(&output).expect("worker log");
    assert!(status.success(), "{log}");
    let report: Value = serde_json::from_slice(&std::fs::read(report).expect("parity receipt"))
        .expect("JSON receipt");
    assert_eq!(report["provider_count"], 45);
    assert_eq!(
        report["shipped_provider_count"],
        report["shipped_catalog"]
            .as_array()
            .expect("shipped catalog")
            .len()
    );
    let cases = report["cases"].as_array().expect("nonempty case inventory");
    assert!(!cases.is_empty());
    let pending: Vec<_> = cases
        .iter()
        .filter(|case| case["status"] != "pass")
        .collect();
    for case in &pending {
        println!(
            "PENDING {}/{}",
            case["provider"].as_str().expect("provider"),
            case["case"].as_str().expect("case")
        );
    }
    println!(
        "provider parity: {} passed cases, {} pending; live requests: 0",
        cases.len() - pending.len(),
        pending.len()
    );
    if std::env::var("MELCHIOR_PROVIDER_PARITY_COMPLETE").as_deref() == Ok("1") {
        assert!(
            pending.is_empty(),
            "provider parity remains incomplete; see PENDING cases above"
        );
    }
}

#[test]
fn isolated_worker() {
    let Some(report) = std::env::var_os("MELCHIOR_PROVIDER_PARITY_REPORT") else {
        return;
    };
    let manifest = manifest();
    let providers = declarations();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime");
    let mut cases = Vec::new();
    for row in manifest.providers.iter().chain(&manifest.retained) {
        let mut exercised = BTreeSet::new();
        if let Some(provider) = providers.get(&row.id) {
            let apis: BTreeSet<_> = provider
                .models
                .iter()
                .map(|model| model.api)
                .chain([provider.api])
                .collect();
            for api in apis {
                if transport::supports(api) {
                    runtime.block_on(transport::text_and_usage(provider, api));
                    exercised.insert(api.as_str().to_owned());
                }
            }
        }
        let covered = !row.apis.is_empty() && row.apis.iter().all(|api| exercised.contains(api));
        for case in &row.required_cases {
            let passed = covered && matches!(case.as_str(), "text" | "usage");
            cases.push(json!({"provider":row.id, "case":case,
                "status":if passed {"pass"} else {"pending"},
                "exercised_apis":exercised,
                "evidence":if passed {"local-http-fixture"} else {"not-implemented"}}));
        }
    }
    let report_body = json!({"schema":1,"reference_revision":manifest.reference.revision,
        "provider_count":manifest.providers.len()+manifest.retained.len(),
        "shipped_provider_count":providers.len(),
        "shipped_catalog":providers.values().map(|provider| json!({"id":provider.id,
            "model_ids":provider.models.iter().map(|model| &model.id).collect::<Vec<_>>()
        })).collect::<Vec<_>>(),
        "discovery_in_fixture":"disabled",
        "live_requests":0,"cases":cases});
    std::fs::write(
        report,
        serde_json::to_vec_pretty(&report_body).expect("receipt JSON"),
    )
    .expect("receipt file");
}
