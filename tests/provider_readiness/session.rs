use super::presets::{events, stopped, unasked};
use super::*;
use melchior::mind::{
    model::Message,
    wire::{Ask, Wants},
};

#[path = "session/oauth.rs"]
mod oauth;

const DONE: &str = "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";

fn fixture(dir: &Scratch, listener: &TcpListener, auth: Value, compat: Value) {
    listener.set_nonblocking(true).expect("nonblocking");
    let url = format!("http://{}/v1", listener.local_addr().expect("address"));
    let declaration = json!({"name":"Fixture","api":"openai-completions","base_url":url,"auth":auth,"compat":compat,"discovery":{"kind":"bundled"},"models":[
        {"id":"m","name":"M","context_window":8192,"max_tokens":128},
        {"id":"n","name":"N","context_window":8192,"max_tokens":128}]});
    let encoded = serde_json::to_string(&declaration.to_string()).expect("Lua string");
    std::fs::write(
        dir.join("providers.lua"),
        format!(
            r#"
        for _, id in ipairs({{"openrouter","ollama"}}) do
            melchior.provider(id,{{name=id,api="openai-completions",auth={{kind="none"}}}})
        end
        melchior.provider("fixture",melchior.json.decode_value({encoded}))
    "#
        ),
    )
    .expect("config");
}

fn request() -> Ask {
    Ask {
        model: "fixture/m".into(),
        about: "first-request".into(),
        context: melchior::mind::model::Context {
            messages: vec![Message::user("hello")],
            ..Default::default()
        },
        wants: Wants {
            session_id: Some("private-sentinel-session".into()),
            max_tokens: Some(128),
            ..Wants::default()
        },
    }
}

fn round(
    dir: &Scratch,
    listener: &TcpListener,
    vars: &[(&str, &str)],
    ask: &Ask,
) -> (String, Value) {
    let before = serde_json::to_value(ask).expect("ask");
    let server = response_server(listener.try_clone().expect("listener"), 200, DONE.into());
    let output = ask_request(dir, vars, ask);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("private-sentinel"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("private-sentinel"));
    stopped(&events(&output), "end_turn");
    assert_eq!(serde_json::to_value(ask).expect("unchanged"), before);
    server.join().expect("request")
}

fn key(body: &Value) -> String {
    let key = body["prompt_cache_key"].as_str().expect("key");
    assert_eq!(key.len(), 64);
    assert!(key.bytes().all(|byte| byte.is_ascii_hexdigit()));
    key.into()
}

#[test]
fn imported_router_baseten_and_fireworks_records_use_the_pinned_detected_header_dialect() {
    for (provider, model) in [
        ("openrouter", "qwen/qwen3.6-27b"),
        ("baseten", "openai/gpt-oss-120b"),
        ("fireworks", "accounts/fireworks/models/kimi-k3"),
    ] {
        let dir = Scratch::new("melchior-ready", "session-imports");
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        listener.set_nonblocking(true).expect("nonblocking");
        let url = format!("http://{}/v1", listener.local_addr().expect("address"));
        let declaration = json!({"name":"Fixture","api":"openai-completions","base_url":url,"auth":{"kind":"none"},"discovery":{"kind":"bundled"},
            "catalog":{"source":"pi-1.6.0","provider":provider,"models":[format!("chat:{model}")]},"models":[{"id":model,"base_url":url}]});
        let encoded = serde_json::to_string(&declaration.to_string()).expect("Lua string");
        std::fs::write(
            dir.join("providers.lua"),
            format!(
                r#"
            for _, id in ipairs({{"openrouter","ollama"}}) do
                melchior.provider(id,{{name=id,api="openai-completions",auth={{kind="none"}}}})
            end
            melchior.provider("fixture",melchior.json.decode_value({encoded}))
        "#
            ),
        )
        .expect("config");
        let mut ask = request();
        ask.model = format!("fixture/{model}");
        let (headers, body) = round(&dir, &listener, &[], &ask);
        assert_eq!(body["model"], model);
        assert!(body.get("prompt_cache_key").is_none());
        for header in [
            "x-session-id",
            "session_id",
            "x-client-request-id",
            "x-session-affinity",
        ] {
            let expected = (provider == "openrouter") == (header == "x-session-id");
            let prefix = format!("{header}: ");
            let value = headers.lines().find_map(|line| line.strip_prefix(&prefix));
            assert_eq!(value.is_some(), expected, "{provider}/{header}");
            if let Some(value) = value {
                assert_eq!(value.len(), 64);
                assert!(value.bytes().all(|b| b.is_ascii_hexdigit()));
            }
        }
    }
}

#[test]
fn scopes_survive_new_processes_and_history_growth_but_not_session_model_or_key_changes() {
    let dir = Scratch::new("melchior-ready", "session-scopes");
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    fixture(
        &dir,
        &listener,
        json!({"kind":"api-key","vars":["FIXTURE_KEY"]}),
        json!({"supports_prompt_cache_key":true,"send_session_affinity_headers":true}),
    );
    assert_eq!(cards(&dir, &[])["capabilities"]["session_identity"], true);
    let mut ask = request();
    let first = round(
        &dir,
        &listener,
        &[("FIXTURE_KEY", "private-sentinel-a")],
        &ask,
    );
    let initial = key(&first.1);
    for header in ["x-client-request-id", "x-session-affinity"] {
        assert!(first.0.contains(&format!("{header}: {initial}\r\n")));
    }
    ask.about = "second-request".into();
    ask.context
        .messages
        .extend([Message::assistant("one"), Message::user("two")]);
    assert_eq!(
        key(&round(
            &dir,
            &listener,
            &[("FIXTURE_KEY", "private-sentinel-a")],
            &ask
        )
        .1),
        initial
    );
    assert_ne!(
        key(&round(
            &dir,
            &listener,
            &[("FIXTURE_KEY", "private-sentinel-b")],
            &ask
        )
        .1),
        initial
    );
    ask.model = "fixture/n".into();
    assert_ne!(
        key(&round(
            &dir,
            &listener,
            &[("FIXTURE_KEY", "private-sentinel-a")],
            &ask
        )
        .1),
        initial
    );
    ask.model = "fixture/m".into();
    ask.wants.session_id = Some("other-session".into());
    assert_ne!(
        key(&round(
            &dir,
            &listener,
            &[("FIXTURE_KEY", "private-sentinel-a")],
            &ask
        )
        .1),
        initial
    );
    ask.wants.session_id = None;
    let absent = round(
        &dir,
        &listener,
        &[("FIXTURE_KEY", "private-sentinel-a")],
        &ask,
    );
    assert!(absent.1.get("prompt_cache_key").is_none());
    assert!(
        !absent.0.contains("x-session-affinity:") && !absent.0.contains("x-client-request-id:")
    );
}

#[test]
fn explicit_header_dialects_and_disabled_presets_preserve_default_body_bytes() {
    for format in ["generic", "openai", "openrouter"] {
        let dir = Scratch::new("melchior-ready", "session-dialect");
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let mut baseline = None;
        for enabled in [false, true] {
            fixture(
                &dir,
                &listener,
                json!({"kind":"none"}),
                json!({"send_session_affinity_headers":enabled,"session_affinity_format":format}),
            );
            let (headers, body) = round(&dir, &listener, &[], &request());
            if let Some(baseline) = &baseline {
                assert_eq!(&body, baseline);
            } else {
                baseline = Some(body.clone());
            }
            assert!(body.get("prompt_cache_key").is_none());
            for header in [
                "session_id",
                "x-client-request-id",
                "x-session-affinity",
                "x-session-id",
            ] {
                let wanted = enabled
                    && match format {
                        "generic" => matches!(header, "x-client-request-id" | "x-session-affinity"),
                        "openai" => header != "x-session-id",
                        _ => header == "x-session-id",
                    };
                assert_eq!(
                    headers.contains(&format!("\r\n{header}:")),
                    wanted,
                    "{format}/{header}"
                );
            }
        }
    }
}

#[test]
fn invalid_identities_refuse_before_missing_keys_oauth_refresh_or_inference() {
    let dir = Scratch::new("melchior-ready", "session-invalid");
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let token = TcpListener::bind("127.0.0.1:0").expect("token");
    token.set_nonblocking(true).expect("nonblocking");
    for auth in [
        json!({"kind":"api-key","vars":["FIXTURE_KEY"]}),
        json!({"kind":"oauth","service":"Fixture","token_url":format!("http://{}/token",token.local_addr().expect("token")),"client_id":"public"}),
    ] {
        fixture(
            &dir,
            &listener,
            auth,
            json!({"supports_prompt_cache_key":true,"send_session_affinity_headers":true}),
        );
        tokens(
            &dir,
            "private-sentinel-access",
            Some("private-sentinel-refresh"),
            1,
        );
        for id in [
            String::new(),
            "x".repeat(257),
            "é".repeat(129),
            "private-sentinel\r\nheader".into(),
            "\u{7f}".into(),
        ] {
            let mut ask = request();
            ask.wants.session_id = Some(id);
            let output = ask_request(&dir, &[], &ask);
            assert!(!String::from_utf8_lossy(&output.stdout).contains("private-sentinel"));
            let said = events(&output);
            assert_eq!(said.len(), 1);
            assert_eq!(said[0]["why"], "invalid");
            unasked(&listener);
            unasked(&token);
        }
    }
}

#[test]
fn retry_keeps_the_same_scoped_key_and_headers_without_mutating_the_ask() {
    let dir = Scratch::new("melchior-ready", "session-retry");
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    fixture(
        &dir,
        &listener,
        json!({"kind":"none"}),
        json!({"supports_prompt_cache_key":true,"send_session_affinity_headers":true}),
    );
    let worker = std::thread::spawn(move || {
        let first = response_server(listener.try_clone().expect("listener"), 503, "{}".into())
            .join()
            .expect("first");
        let second = response_server_with_timeout(
            listener,
            200,
            DONE.into(),
            std::time::Duration::from_secs(15),
        )
        .join()
        .expect("second");
        (first, second)
    });
    let ask = request();
    let saved = serde_json::to_value(&ask).expect("ask");
    let said = events(&ask_request_with_timeout(
        &dir,
        &[],
        &ask,
        std::time::Duration::from_secs(30),
    ));
    assert_eq!(said.iter().filter(|e| e["event"] == "retrying").count(), 1);
    assert!(!said.iter().any(|e| e["event"] == "failed"));
    assert_eq!(
        said.iter()
            .filter(|e| e["event"] == "stop" && e["reason"] == "end_turn")
            .count(),
        1
    );
    let (first, second) = worker.join().expect("server");
    assert_eq!(first, second);
    key(&second.1);
    assert_eq!(serde_json::to_value(&ask).expect("unchanged"), saved);
}
