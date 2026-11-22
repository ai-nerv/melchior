use super::*;

fn config(dir: &Scratch, url: &str, strategy: &str, tenant: &str) {
    std::fs::write(dir.join("providers.lua"),format!(r#"
        for _, id in ipairs({{"openrouter","ollama"}}) do
            melchior.provider(id,{{name=id,api="openai-completions",auth={{kind="none"}}}})
        end
        melchior.provider("fixture",{{name="Fixture",api="openai-completions",base_url={url:?},
            auth={{kind="api-key",vars={{"KEY","ALIAS"}}}},headers={{["x-tenant"]={tenant:?}}},
            discovery={strategy}, models={{{{id="bundled",name="Bundled",context_window=8192,max_tokens=128}}}}}})
    "#)).expect("config");
}

fn fixture(reply: &Value) -> Vec<String> {
    reply["result"]
        .as_array()
        .expect("models")
        .iter()
        .filter(|row| row["provider"] == "fixture")
        .map(|row| row["name"].as_str().expect("name").to_owned())
        .collect()
}

#[test]
fn cache_partitions_credentials_headers_and_endpoints_without_storing_them() {
    let dir = Scratch::new("melchior-discovery", "identity");
    let server = Provider::new();
    let strategy = r#"{kind="compatible",auth="x-api-key"}"#;
    config(&dir, &server.url, strategy, "private-sentinel-tenant");
    let missing = list_with(&dir, true, &[]);
    assert_eq!(fixture(&missing), ["bundled"]);
    assert_eq!(missing["failed"], json!(["fixture"]));
    assert_eq!(server.calls.load(Ordering::SeqCst), 0);
    for (key, id) in [("private-sentinel-a", "a"), ("private-sentinel-b", "b")] {
        *server.response.lock().expect("response") = (200, json!({"data":[{"id":id}]}));
        assert_eq!(
            fixture(&list_with(&dir, false, &[("KEY", " "), ("ALIAS", key)])),
            [id]
        );
    }
    assert_eq!(
        fixture(&list_with(&dir, false, &[("ALIAS", "private-sentinel-a")])),
        ["a"]
    );
    assert_eq!(server.calls.load(Ordering::SeqCst), 2);
    {
        let requests = server.requests.lock().expect("requests");
        assert!(requests[0].contains("x-api-key: private-sentinel-a\r\n"));
        assert!(requests[0].contains("x-tenant: private-sentinel-tenant\r\n"));
        assert!(!requests[0].to_ascii_lowercase().contains("authorization:"));
    }
    config(
        &dir,
        &server.url,
        strategy,
        "private-sentinel-changed-tenant",
    );
    assert_eq!(
        fixture(&list_with(&dir, false, &[("ALIAS", "private-sentinel-a")])),
        ["b"]
    );
    assert_eq!(server.calls.load(Ordering::SeqCst), 3);
    let second = Provider::new();
    config(&dir, &second.url, strategy, "private-sentinel-tenant");
    assert_eq!(
        fixture(&list_with(&dir, false, &[("ALIAS", "private-sentinel-a")])),
        ["old"]
    );
    assert_eq!(second.calls.load(Ordering::SeqCst), 1);
    let entries: Vec<_> = std::fs::read_dir(dir.join("cache/melchior/models/v2"))
        .expect("cache")
        .map(|entry| entry.expect("entry").path())
        .collect();
    assert_eq!(entries.len(), 4);
    for path in entries {
        assert!(!path.to_string_lossy().contains("private-sentinel"));
        assert!(
            !std::fs::read_to_string(path)
                .expect("cache")
                .contains("private-sentinel")
        );
    }
}

#[test]
fn openrouter_uses_a_separate_authenticated_listing_and_preserves_last_good_data() {
    let dir = Scratch::new("melchior-discovery", "openrouter");
    let server = Provider::new();
    config(&dir, &server.url, r#"{kind="openrouter"}"#, "fixture");
    assert_eq!(fixture(&list_with(&dir, false, &[])), ["old"]);
    *server.response.lock().expect("response") = (200, json!({"data":[{"id":"allowed"}]}));
    let vars = [("KEY", "private-sentinel-key")];
    assert_eq!(fixture(&list_with(&dir, false, &vars)), ["allowed"]);
    {
        let requests = server.requests.lock().expect("requests");
        assert!(requests[0].starts_with("GET /v1/models?sort=most-popular "));
        assert!(!requests[0].to_ascii_lowercase().contains("authorization:"));
        assert!(requests[1].starts_with("GET /v1/models/user "));
        assert!(requests[1].contains("authorization: Bearer private-sentinel-key\r\n"));
    }
    for entry in std::fs::read_dir(dir.join("cache/melchior/models/v2")).expect("cache") {
        std::fs::File::open(entry.expect("entry").path())
            .expect("cache file")
            .set_times(std::fs::FileTimes::new().set_modified(std::time::SystemTime::UNIX_EPOCH))
            .expect("stale");
    }
    *server.response.lock().expect("response") = (401, json!({"error":"private-sentinel-refused"}));
    let stale = list_with(&dir, false, &vars);
    assert_eq!(fixture(&stale), ["allowed"]);
    assert_eq!(list_with(&dir, true, &vars)["failed"], json!(["fixture"]));
    *server.response.lock().expect("response") = (200, json!({"data":[]}));
    assert!(fixture(&list_with(&dir, true, &vars)).is_empty());
    assert!(fixture(&list_with(&dir, false, &vars)).is_empty());
}

#[test]
fn explicit_bundled_strategy_performs_no_discovery() {
    let dir = Scratch::new("melchior-discovery", "bundled");
    let server = Provider::new();
    config(&dir, &server.url, r#"{kind="bundled"}"#, "fixture");
    assert_eq!(
        fixture(&list_with(&dir, true, &[("KEY", "private-sentinel-key")])),
        ["bundled"]
    );
    assert_eq!(server.calls.load(Ordering::SeqCst), 0);
}
