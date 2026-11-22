use super::*;
use melchior::mind::{
    model::{Context, Message},
    wire::Ask,
};

fn selected(dir: &Scratch, url: &str, auth: &str, provider: &str, keys: &str) {
    config(dir, url, auth, "openai-responses");
    std::fs::OpenOptions::new()
        .append(true)
        .open(dir.join("providers.lua"))
        .expect("config")
        .write_all(
            format!(
                r#"
        melchior.provider("fixture", {{name="Selected",api="openai-responses",base_url={url:?},
            auth={auth}, discovery={{kind="bundled"}},
            catalog={{source="pi-1.6.0",provider={provider:?},models={{{keys}}}}},
            models={{
                {{id="llama-3.3-70b-versatile",base_url={url:?},headers={{["x-route"]="selected"}}}}
            }}
        }})
        "#
            )
            .as_bytes(),
        )
        .expect("import declaration");
}

fn ask() -> Ask {
    Ask {
        model: "fixture/llama-3.3-70b-versatile".into(),
        context: Context {
            messages: vec![Message::user("hello")],
            ..Default::default()
        },
        wants: Default::default(),
        about: String::new(),
    }
}

#[test]
fn imports_are_offline_during_listing_and_route_real_inference_with_the_selected_protocol() {
    let dir = Scratch::new("melchior-import", "native");
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let url = format!("http://{}/v1", listener.local_addr().expect("address"));
    selected(
        &dir,
        &url,
        r#"{kind="api-key",vars={"FIXTURE_KEY"}}"#,
        "groq",
        r#""chat:llama-3.3-70b-versatile""#,
    );
    let card = cards(&dir, &[]);
    assert_eq!(card["id"], "fixture/llama-3.3-70b-versatile");
    assert_eq!(card["api"], "openai-completions");
    assert_eq!(card["context_window"], 131072);
    assert_eq!(card["max_output"], 32768);
    assert_eq!(card["ready"], false);
    assert_eq!(
        listener
            .accept()
            .expect_err("bundled listing does not dispatch")
            .kind(),
        std::io::ErrorKind::WouldBlock
    );
    let card = cards(&dir, &[("FIXTURE_KEY", "private-sentinel")]);
    assert_eq!(card["ready"], true);
    assert_eq!(card["pricing"]["input"], "0.59");
    assert_eq!(
        listener
            .accept()
            .expect_err("ready listing is also offline")
            .kind(),
        std::io::ErrorKind::WouldBlock
    );
    let server = server(listener);
    let output = ask_request(&dir, &[("FIXTURE_KEY", "private-sentinel")], &ask());
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let (headers, body) = server.join().expect("HTTP fixture");
    assert!(headers.starts_with("post /v1/chat/completions "));
    assert!(headers.contains("authorization: bearer private-sentinel\r\n"));
    assert!(headers.contains("x-route: selected\r\n"));
    assert_eq!(body["model"], "llama-3.3-70b-versatile");
    assert!(body["messages"].is_array());
    assert!(!body.to_string().contains("catalog"));
    let events: Vec<Value> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).expect("event"))
        .collect();
    assert!(events.iter().any(|event| event["event"] == "stop"));
    assert!(!events.iter().any(|event| event["event"] == "failed"));
}

#[test]
fn imports_refuse_unknown_and_pending_records_before_refresh_or_inference() {
    for (provider, keys) in [
        (
            "groq",
            r#""chat:llama-3.3-70b-versatile","chat:private-sentinel""#,
        ),
        ("openrouter", r#""chat:anthropic/claude-fable-5.1""#),
    ] {
        let dir = Scratch::new("melchior-import", "invalid");
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        listener.set_nonblocking(true).expect("nonblocking");
        let url = format!("http://{}", listener.local_addr().expect("address"));
        let auth = format!(
            r#"{{kind="oauth",service="Fixture",authorize_url="{url}/authorize",token_url="{url}/token",client_id="fixture"}}"#
        );
        selected(&dir, &url, &auth, provider, keys);
        tokens(
            &dir,
            "private-sentinel",
            Some("private-sentinel-refresh"),
            1,
        );
        let before = std::fs::read(dir.join("data/magi/credentials.json")).expect("credentials");
        let output = ask_request(&dir, &[], &ask());
        assert!(output.status.success());
        let events: Vec<Value> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|line| serde_json::from_str(line).expect("event"))
            .collect();
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0]["event"], "failed");
        assert_eq!(events[0]["why"], "invalid");
        assert!(
            events[0]["message"]
                .as_str()
                .expect("message")
                .contains("catalog.models")
        );
        assert_eq!(
            listener.accept().expect_err("no dispatch").kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert_eq!(
            std::fs::read(dir.join("data/magi/credentials.json")).expect("credentials"),
            before
        );
    }
}

#[test]
fn after_plugin_registration_replaces_the_whole_catalog_selection() {
    let dir = Scratch::new("melchior-import", "layers");
    selected(
        &dir,
        "https://fixture.invalid",
        r#"{kind="none"}"#,
        "groq",
        r#""chat:llama-3.3-70b-versatile""#,
    );
    std::fs::create_dir_all(dir.join("after/plugin")).expect("plugin directory");
    std::fs::write(
        dir.join("after/plugin/models.lua"),
        r#"
        melchior.provider("fixture", {name="Replaced",api="openai-completions",
            base_url="https://api.cerebras.ai/v1",auth={kind="none"},discovery={kind="bundled"},
            catalog={source="pi-1.6.0",provider="cerebras",models={"chat:gpt-oss-120b"}}})
    "#,
    )
    .expect("plugin");
    let output = command(&dir)
        .args(["models", "--json"])
        .output()
        .expect("models");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let rows: Value = serde_json::from_slice(&output.stdout).expect("models");
    let ids: Vec<_> = rows["result"]
        .as_array()
        .expect("rows")
        .iter()
        .filter(|row| row["provider"] == "fixture")
        .map(|row| row["id"].as_str().expect("id"))
        .collect();
    assert_eq!(ids, ["fixture/gpt-oss-120b"]);
}
