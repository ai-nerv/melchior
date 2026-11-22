use super::*;
use melchior::mind::model::ToolStrictness;
use melchior::mind::wire::Ask;

fn fixture(prefix: &str, oauth: bool, model_base: Option<&str>) -> (Scratch, TcpListener, Ask) {
    let dir = Scratch::new("melchior-deepseek", "strict-routing");
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let origin = format!("http://{}", listener.local_addr().expect("address"));
    let url = format!("{origin}{prefix}");
    let auth = if oauth {
        format!(
            r#"{{kind="oauth",service="Fixture",authorize_url="{origin}/authorize",token_url="{origin}/token",client_id="public"}}"#
        )
    } else {
        r#"{kind="api-key",vars={"FIXTURE_KEY"}}"#.into()
    };
    config(&dir, &url, &auth, "openai-completions");
    let model_base = model_base.map_or(String::new(), |base| format!("base_url={base:?},"));
    std::fs::OpenOptions::new().append(true).open(dir.join("providers.lua")).expect("config")
        .write_all(format!(r#"
        melchior.provider("fixture", {{name="Fixture",api="openai-completions",base_url={url:?},auth={auth},
            compat={{supports_strict_mode=true,chat_strict_dialect="deepseek",supports_json_schema=false}},
            models={{{{id="m",name="M",{model_base}context_window=8192,max_tokens=128}}}}}})
        "#).as_bytes()).expect("overlay");
    let ask = serde_json::from_value(json!({"model":"fixture/m","context":{
        "messages":[{"role":"user","content":[{"type":"text","text":"hello"}]}],
        "tools":[{"name":"echo","description":"echo","strict":"require",
            "parameters":{"type":"object","properties":{"value":{"type":"string","pattern":"^a"}},"required":["value"]}}]
    }})).expect("ask");
    (dir, listener, ask)
}

fn finished() -> String {
    concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":null}],\"usage\":null}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":7,\"prompt_cache_hit_tokens\":40,\"prompt_cache_miss_tokens\":60}}\n\n",
        "data: [DONE]\n\n"
    ).into()
}

fn requested(headers: &str, body: &Value, path: &str, strict: bool) {
    assert!(
        headers.starts_with(&format!("post {path}/chat/completions ")),
        "{headers}"
    );
    assert_eq!(headers.matches("authorization:").count(), 1);
    assert!(headers.contains("authorization: bearer private-sentinel\r\n"));
    assert_eq!(body["model"], "m");
    assert_eq!(body["stream_options"]["include_usage"], true);
    for tool in body["tools"].as_array().expect("tools") {
        assert_eq!(tool["function"]["strict"], strict);
        assert!(tool["function"]["parameters"].is_object());
    }
    assert!(!body.to_string().contains("__strict"));
    assert!(!body.to_string().contains("chat_strict_dialect"));
}

#[test]
fn actual_process_routes_effective_strict_sets_only_and_counts_terminal_usage() {
    for prefix in ["/v1", "/gateway/v1"] {
        for mode in ["require", "prefer", "ordinary", "fallback"] {
            let (dir, listener, mut ask) = fixture(prefix, false, None);
            let effective = matches!(mode, "require" | "prefer");
            if mode == "ordinary" {
                ask.context.tools[0].strict = None;
            } else if mode != "require" {
                ask.context.tools[0].strict = Some(ToolStrictness::Prefer);
            }
            if mode == "fallback" {
                let mut other = ask.context.tools[0].clone();
                other.name = "other".into();
                other.parameters["properties"]["value"]["minLength"] = json!(1);
                ask.context.tools.push(other);
            }
            let saved = ask.clone();
            let server = response_server(listener, 200, finished());
            let said = presets::events(&ask_request(
                &dir,
                &[("FIXTURE_KEY", "private-sentinel")],
                &ask,
            ));
            presets::stopped(&said, "end_turn");
            let spent = said.iter().rfind(|e| e["event"] == "spent").expect("usage");
            assert_eq!(spent["usage"]["input"], 60);
            assert_eq!(spent["usage"]["output"], 7);
            assert_eq!(spent["usage"]["cache_read"], 40);
            let (headers, body) = server.join().expect("fixture");
            let path = if effective {
                format!("{}/beta", prefix.trim_end_matches("/v1"))
            } else {
                prefix.into()
            };
            requested(&headers, &body, &path, effective);
            for (tool, original) in body["tools"]
                .as_array()
                .expect("tools")
                .iter()
                .zip(&ask.context.tools)
            {
                let parameters = &tool["function"]["parameters"];
                if effective {
                    assert_eq!(parameters["additionalProperties"], false);
                    assert_eq!(parameters["properties"], original.parameters["properties"]);
                } else {
                    assert_eq!(parameters, &original.parameters);
                }
            }
            assert_eq!(ask.context, saved.context);
        }
    }
}

#[test]
fn unsupported_required_sets_and_unbound_routes_refuse_before_refresh_or_dispatch() {
    let foreign = TcpListener::bind("127.0.0.1:0").expect("foreign listener");
    foreign.set_nonblocking(true).expect("nonblocking");
    let foreign_url = format!("http://{}/v1", foreign.local_addr().expect("address"));
    for case in ["schema", "mixed", "origin", "query"] {
        let prefix = if case == "query" {
            "/v1?secret=private-sentinel"
        } else {
            "/v1"
        };
        let (dir, listener, mut ask) = fixture(
            prefix,
            true,
            (case == "origin").then_some(foreign_url.as_str()),
        );
        tokens(&dir, "old-access", Some("private-sentinel-refresh"), 1);
        match case {
            "schema" => {
                ask.context.tools[0].parameters["properties"]["value"]["minLength"] = json!(1)
            }
            "mixed" => {
                let mut tool = ask.context.tools[0].clone();
                tool.name = "legacy".into();
                tool.strict = None;
                ask.context.tools.push(tool);
            }
            _ => {}
        }
        let said = presets::events(&ask_request(&dir, &[], &ask));
        assert_eq!(said.len(), 1, "{said:?}");
        assert_eq!(said[0]["event"], "failed");
        assert_eq!(said[0]["why"], "invalid");
        assert!(!said[0].to_string().contains("private-sentinel"));
        for socket in [&listener, &foreign] {
            assert_eq!(
                socket.accept().expect_err("no refresh or inference").kind(),
                std::io::ErrorKind::WouldBlock
            );
        }
        let store = Store::load_from(&dir.join("data/magi/credentials.json")).expect("store");
        assert_eq!(
            store.get("fixture").expect("unchanged").access,
            "old-access"
        );
    }
}

#[test]
fn retry_preserves_beta_destination_and_required_schema_without_loosening_policy() {
    let (dir, listener, ask) = fixture("/v1", false, None);
    let next = listener.try_clone().expect("listener");
    let worker = std::thread::spawn(move || {
        let first = response_server(listener, 503, r#"{"error":{"message":"retry"}}"#.into())
            .join()
            .expect("first");
        let second =
            response_server_with_timeout(next, 200, finished(), std::time::Duration::from_secs(20))
                .join()
                .expect("second");
        (first, second)
    });
    let saved = ask.context.clone();
    let said = presets::events(&ask_request_with_timeout(
        &dir,
        &[("FIXTURE_KEY", "private-sentinel")],
        &ask,
        std::time::Duration::from_secs(30),
    ));
    assert_eq!(said.iter().filter(|e| e["event"] == "retrying").count(), 1);
    assert!(!said.iter().any(|e| e["event"] == "failed"));
    let stops: Vec<_> = said.iter().filter(|e| e["event"] == "stop").collect();
    assert_eq!(stops.len(), 1);
    assert_eq!(stops[0]["reason"], "end_turn");
    let (first, second) = worker.join().expect("fixture");
    for (headers, body) in [&first, &second] {
        requested(headers, body, "/beta", true);
    }
    assert_eq!(first.1, second.1);
    assert_eq!(ask.context, saved);
}
