#![allow(clippy::expect_used, clippy::indexing_slicing)]

#[path = "copilot/fixture.rs"]
mod fixture;
use fixture::*;
use melchior::mind::provider::oauth::{self, Store};
use serde_json::json;
use std::time::{Duration, Instant};

#[test]
fn rate_limits_wait_and_malformed_or_oversized_control_replies_never_persist() {
    let mock = Mock::new();
    let dir = config(&mock.url);
    let serving = mock.serve(vec![
        device(&mock.url, 30),
        step("/oauth", json!({})).status(429),
        granted(),
        token(),
        models(),
    ]);
    let start = Instant::now();
    let login = auth(&dir, "login");
    assert_eq!(login.reply()["event"], "device");
    assert!(login.reply()["error"].is_null());
    assert!(start.elapsed() >= Duration::from_secs(7));
    serving.join().expect("server");
    for body in [
        "private-sentinel not JSON".into(),
        "x".repeat(1024 * 1024 + 1),
    ] {
        let mock = Mock::new();
        let dir = config(&mock.url);
        let serving = mock.serve(vec![device(&mock.url, 30).body(body)]);
        assert!(auth(&dir, "login").reply()["error"].is_string());
        serving.join().expect("server");
        assert!(
            Store::load_from(&path(&dir))
                .expect("store")
                .get("fixture")
                .is_none()
        );
    }
}

#[test]
fn failed_renewal_preserves_credentials_and_unauthorized_model_never_dispatches() {
    let mock = Mock::new();
    let dir = config(&mock.url);
    let serving = mock.serve(vec![
        device(&mock.url, 30),
        granted(),
        token(),
        models(),
        step("/token", json!({"error":GITHUB})).status(401),
    ]);
    let login = auth(&dir, "login");
    assert_eq!(login.reply()["event"], "device");
    assert!(login.reply()["error"].is_null());
    let mut store = Store::load_from(&path(&dir)).expect("store");
    store
        .providers
        .get_mut("fixture")
        .expect("token")
        .expires_at = 0;
    store.save_to(&path(&dir)).expect("expire");
    let before = std::fs::read(path(&dir)).expect("credentials");
    let refused = ask(&dir, "disabled");
    assert!(!String::from_utf8_lossy(&refused.stdout).contains("\"text\":\"ok\""));
    let refused = ask(&dir, "mc");
    assert!(!String::from_utf8_lossy(&refused.stdout).contains("\"text\":\"ok\""));
    serving.join().expect("server");
    assert_eq!(before, std::fs::read(path(&dir)).expect("unchanged"));
}

#[test]
fn device_login_lists_only_enabled_models_and_calls_all_three_protocols() {
    let mock = Mock::new();
    let dir = config(&mock.url);
    let serving = mock.serve(vec![
        device(&mock.url, 30),
        granted(),
        token(),
        models(),
        chat(),
        anthropic(),
        responses(),
    ]);
    let login = auth(&dir, "login");
    let prompt = login.reply();
    assert_eq!(prompt["event"], "device");
    assert_eq!(prompt["code"], "ABCD-EFGH");
    assert!(login.reply()["error"].is_null());
    let store = Store::load_from(&path(&dir)).expect("store");
    assert_eq!(
        store.get("fixture").expect("token").refresh.as_deref(),
        Some(GITHUB)
    );
    assert_eq!(store.copilot["fixture"].models.len(), 3);
    let output = command(&dir)
        .args(["models", "--json"])
        .output()
        .expect("models");
    let cards: serde_json::Value = serde_json::from_slice(&output.stdout).expect("cards");
    let listed: Vec<_> = cards["result"]
        .as_array()
        .expect("cards")
        .iter()
        .filter(|c| c["provider"] == "fixture")
        .collect();
    assert_eq!(listed.len(), 3);
    assert!(listed.iter().all(|c| c["ready"] == true));
    for model in ["mc", "ma", "mr"] {
        assert_answer(ask(&dir, model));
    }
    let requests = serving.join().expect("server");
    for request in requests.iter().skip(4) {
        assert!(
            request
                .headers
                .to_lowercase()
                .contains(&format!("authorization: bearer {COPILOT}"))
        );
        assert!(!request.headers.to_lowercase().contains("x-api-key:"));
        assert!(request.headers.to_lowercase().contains("x-initiator: user"));
        assert!(
            request
                .headers
                .to_lowercase()
                .contains("copilot-integration-id: vscode-chat")
        );
    }
    assert!(requests[0].body.contains("scope=read%3Auser"));
    assert!(
        requests[1]
            .body
            .contains("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code")
    );
    assert!(requests[2].headers.contains(GITHUB));
    assert!(!requests[3].headers.contains(GITHUB));
    let removed = auth(&dir, "remove").reply();
    assert!(removed["error"].is_null());
    let store = Store::load_from(&path(&dir)).expect("removed");
    assert!(store.get("fixture").is_none() && !store.copilot.contains_key("fixture"));
}

#[test]
fn expired_access_is_renewed_once_across_two_native_processes() {
    let mock = Mock::new();
    let dir = config(&mock.url);
    let serving = mock.serve(vec![
        device(&mock.url, 30),
        granted(),
        token(),
        models(),
        token(),
        models(),
        chat(),
        chat(),
    ]);
    let login = auth(&dir, "login");
    assert_eq!(login.reply()["event"], "device");
    assert!(login.reply()["error"].is_null());
    let mut store = Store::load_from(&path(&dir)).expect("store");
    let binding = store.get("fixture").expect("token").binding.clone();
    store
        .providers
        .get_mut("fixture")
        .expect("token")
        .expires_at = 0;
    store.save_to(&path(&dir)).expect("expire");
    let first = ask_start(&dir, "mc");
    let second = ask_start(&dir, "mc");
    assert_answer(finish(first));
    assert_answer(finish(second));
    let requests = serving.join().expect("server");
    assert_eq!(requests.iter().filter(|r| r.path == "/token").count(), 2);
    assert_eq!(
        Store::load_from(&path(&dir))
            .expect("store")
            .get("fixture")
            .expect("token")
            .binding,
        binding
    );
    assert!(
        oauth::now()
            < Store::load_from(&path(&dir))
                .expect("store")
                .get("fixture")
                .expect("token")
                .expires_at
    );
}

#[test]
fn logout_fences_a_pending_device_login_and_cancellation_reaps_it() {
    let mock = Mock::new();
    let dir = config(&mock.url);
    let serving = mock.serve(vec![device(&mock.url, 30), granted(), token(), models()]);
    let login = auth(&dir, "login");
    assert_eq!(login.reply()["event"], "device");
    assert!(auth(&dir, "remove").reply()["error"].is_null());
    assert!(login.reply()["error"].is_string());
    serving.join().expect("server");
    assert!(
        Store::load_from(&path(&dir))
            .expect("store")
            .get("fixture")
            .is_none()
    );

    let mock = Mock::new();
    let dir = config(&mock.url);
    let serving = mock.serve(vec![device(&mock.url, 30)]);
    let login = auth(&dir, "login");
    assert_eq!(login.reply()["event"], "device");
    let pid = login.child.id();
    drop(login);
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
    serving.join().expect("server");
    assert!(
        Store::load_from(&path(&dir))
            .expect("store")
            .get("fixture")
            .is_none()
    );
}

#[test]
fn device_polling_honors_pending_slowdown_and_expiration() {
    let mock = Mock::new();
    let dir = config(&mock.url);
    let serving = mock.serve(vec![
        device(&mock.url, 30),
        step("/oauth", json!({"error":"authorization_pending"})),
        step("/oauth", json!({"error":"slow_down"})),
        granted(),
        token(),
        models(),
    ]);
    let start = Instant::now();
    let login = auth(&dir, "login");
    assert_eq!(login.reply()["event"], "device");
    assert!(login.reply()["error"].is_null());
    assert!(start.elapsed() >= Duration::from_secs(8));
    serving.join().expect("server");
    let mock = Mock::new();
    let dir = config(&mock.url);
    let serving = mock.serve(vec![device(&mock.url, 1)]);
    let login = auth(&dir, "login");
    assert_eq!(login.reply()["event"], "device");
    assert!(login.reply()["error"].is_string());
    serving.join().expect("server");
}

#[test]
fn denial_phishing_and_untrusted_token_endpoints_do_not_store_credentials() {
    for mode in ["denied", "phishing", "endpoint"] {
        let mock = Mock::new();
        let dir = config(&mock.url);
        let steps = match mode {
            "denied" => vec![
                device(&mock.url, 30),
                step(
                    "/oauth",
                    json!({"error":"access_denied","error_description":GITHUB}),
                ),
            ],
            "phishing" => vec![device("https://foreign.invalid", 30)],
            _ => vec![
                device(&mock.url, 30),
                granted(),
                step(
                    "/token",
                    json!({"token":"private-sentinel;proxy-ep=proxy.evil.invalid;","expires_at":oauth::now()+3600}),
                ),
            ],
        };
        let serving = mock.serve(steps);
        let login = auth(&dir, "login");
        let reply = login.reply();
        let reply = if reply["event"] == "device" {
            login.reply()
        } else {
            reply
        };
        assert!(reply["error"].is_string(), "{reply}");
        serving.join().expect("server");
        assert!(
            Store::load_from(&path(&dir))
                .expect("store")
                .get("fixture")
                .is_none()
        );
    }
}

#[test]
fn generic_github_token_does_not_unlock_copilot_and_configuration_changes_refuse() {
    let mock = Mock::new();
    let dir = config(&mock.url);
    let output = command(&dir)
        .env("GITHUB_TOKEN", GITHUB)
        .args(["models", "--json"])
        .output()
        .expect("models");
    let cards: serde_json::Value = serde_json::from_slice(&output.stdout).expect("cards");
    assert_eq!(
        cards["result"]
            .as_array()
            .expect("cards")
            .iter()
            .filter(|c| c["provider"] == "fixture")
            .count(),
        4
    );
    assert!(
        cards["result"]
            .as_array()
            .expect("cards")
            .iter()
            .filter(|c| c["provider"] == "fixture")
            .all(|c| c["ready"] == false)
    );
    let serving = mock.serve(vec![device(&mock.url, 30), granted(), token(), models()]);
    let login = auth(&dir, "login");
    assert_eq!(login.reply()["event"], "device");
    assert!(login.reply()["error"].is_null());
    serving.join().expect("server");
    let config = std::fs::read_to_string(dir.join("providers.lua")).expect("config");
    std::fs::write(
        dir.join("providers.lua"),
        config.replace("fixture-client", "different-client"),
    )
    .expect("change");
    let output = ask(&dir, "mc");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("\"text\":\"ok\""));
    assert!(!String::from_utf8_lossy(&output.stdout).contains(GITHUB));
}
