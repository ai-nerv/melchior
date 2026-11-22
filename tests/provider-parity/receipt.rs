use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub fn retain(report: &Path, log: &Path, status: &str) -> PathBuf {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("receipt timestamp");
    let destination = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/provider-parity")
        .join(format!("run-{}-{}", now.as_nanos(), std::process::id()));
    std::fs::create_dir_all(&destination).expect("receipt directory");
    std::fs::copy(log, destination.join("worker.log")).expect("retained worker output");
    if report.is_file() {
        std::fs::copy(report, destination.join("report.json")).expect("retained case receipt");
    }
    let executable =
        std::fs::read(std::env::current_exe().expect("test executable")).expect("test image bytes");
    let digest = |bytes: &[u8]| format!("{:x}", Sha256::digest(bytes));
    let metadata = json!({
        "schema":1, "created_unix_seconds":now.as_secs(), "worker_status":status,
        "test_binary_sha256":digest(&executable),
        "manifest_sha256":digest(super::MANIFEST.as_bytes()),
        "registry_sha256":digest(super::REGISTRY.as_bytes()),
        "shipped_providers_sha256":digest(include_bytes!("../../config/providers.lua")),
        "shipped_apis_sha256":digest(include_bytes!("../../config/apis.lua")),
        "pinned_model_catalog_sha256":digest(include_bytes!("../../catalog/generated/pi-1.6.0.json")),
        "cargo_lock_sha256":digest(include_bytes!("../../Cargo.lock")),
        "live_requests":0
    });
    std::fs::write(
        destination.join("runner.json"),
        serde_json::to_vec_pretty(&metadata).expect("metadata JSON"),
    )
    .expect("runner receipt");
    destination
}
