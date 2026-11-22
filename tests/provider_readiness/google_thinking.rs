use super::*;
use melchior::mind::{model::ThinkingLevel, wire::Ask};
use std::time::Duration;

const APIS: [&str; 2] = ["google-generative-ai", "google-vertex"];
const FINISHED: &str = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"ok\"}]},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":2,\"cachedContentTokenCount\":0,\"candidatesTokenCount\":1,\"thoughtsTokenCount\":1}}\n\n";

struct Case {
    id: &'static str,
    field: &'static str,
    menu: &'static [&'static str],
    values: [&'static str; 8],
}

const CASES: &[Case] = &[
    Case {
        id: "gemini-2.5-pro",
        field: "thinkingBudget",
        menu: &["minimal", "low", "medium", "high"],
        values: [
            "128", "128", "128", "2048", "8192", "32768", "32768", "32768",
        ],
    },
    Case {
        id: "gemini-2.5-flash-lite",
        field: "thinkingBudget",
        menu: &["off", "minimal", "low", "medium", "high"],
        values: ["0", "0", "512", "2048", "8192", "24576", "24576", "24576"],
    },
    Case {
        id: "gemini-3-pro",
        field: "thinkingLevel",
        menu: &["low", "high"],
        values: ["LOW", "LOW", "LOW", "LOW", "HIGH", "HIGH", "HIGH", "HIGH"],
    },
    Case {
        id: "gemini-3-flash-preview",
        field: "thinkingLevel",
        menu: &["minimal", "low", "medium", "high"],
        values: [
            "MINIMAL", "MINIMAL", "MINIMAL", "LOW", "MEDIUM", "HIGH", "HIGH", "HIGH",
        ],
    },
    Case {
        id: "gemini-4-hydrogen",
        field: "thinkingLevel",
        menu: &["low", "medium", "high", "xhigh"],
        values: [
            "LOW", "LOW", "LOW", "LOW", "MEDIUM", "HIGH", "XHIGH", "XHIGH",
        ],
    },
    Case {
        id: "gemma-4-31b-it",
        field: "thinkingLevel",
        menu: &["minimal", "high"],
        values: [
            "MINIMAL", "MINIMAL", "MINIMAL", "HIGH", "HIGH", "HIGH", "HIGH", "HIGH",
        ],
    },
    Case {
        id: "gemini-robotics-er-2-preview",
        field: "thinkingLevel",
        menu: &["minimal", "low", "medium", "high"],
        values: [
            "MINIMAL", "MINIMAL", "MINIMAL", "LOW", "MEDIUM", "HIGH", "HIGH", "HIGH",
        ],
    },
];

fn fixture(api: &str, model: Value, compat: Value) -> (Scratch, TcpListener, Ask) {
    let dir = Scratch::new("melchior-ready", "google-thinking");
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let url = format!("http://{}/v1", listener.local_addr().expect("address"));
    config(&dir, &url, r#"{kind="api-key",vars={"FIXTURE_KEY"}}"#, api);
    let id = model["id"].as_str().expect("id");
    let declaration = json!({"name":"Fixture","api":api,"base_url":url,
        "auth":{"kind":"api-key","vars":["FIXTURE_KEY"]},"compat":compat,"models":[model]});
    let quoted = serde_json::to_string(&declaration.to_string()).expect("Lua JSON string");
    std::fs::OpenOptions::new()
        .append(true)
        .open(dir.join("providers.lua"))
        .expect("config")
        .write_all(
            format!("\nmelchior.provider(\"fixture\", melchior.json.decode({quoted}))\n")
                .as_bytes(),
        )
        .expect("append");
    let ask = serde_json::from_value(json!({"model":format!("fixture/{id}"),"about":"thinking-fixture",
        "wants":{"max_tokens":128},"context":{"messages":[{"role":"user","content":[{"type":"text","text":"hello"}]}]}})).expect("ask");
    (dir, listener, ask)
}

fn model(id: &str) -> Value {
    let mut model =
        json!({"id":id,"name":"Model","reasoning":true,"context_window":100000,"max_tokens":65536});
    if id.starts_with("gemma-") {
        model["thinking"] =
            json!({"off":null,"minimal":"MINIMAL","low":null,"medium":null,"high":"HIGH"});
    }
    model
}

fn wire(api: &str, headers: &str, body: &Value, config: &Value) {
    assert!(headers.contains(if api == "google-generative-ai" {
        "x-goog-api-key: private-sentinel\r\n"
    } else {
        "authorization: bearer private-sentinel\r\n"
    }));
    assert_eq!(&body["generationConfig"]["thinkingConfig"], config);
    assert_eq!(body["generationConfig"]["maxOutputTokens"], 128);
    for field in ["__google_control", "__budget", "__scratch"] {
        assert!(!body.to_string().contains(field));
    }
}

#[test]
fn published_json_cbor_menus_match_all_native_effort_and_default_requests() {
    let mut requests = 0;
    for api in APIS {
        for case in CASES {
            let (dir, listener, mut ask) = fixture(api, model(case.id), json!({}));
            let card = cards(&dir, &[("FIXTURE_KEY", "private-sentinel")]);
            assert_eq!(card["capabilities"]["thinking_levels"], json!(case.menu));
            let cbor = command(&dir)
                .env("FIXTURE_KEY", "private-sentinel")
                .args(["models", "--cbor"])
                .output()
                .expect("CBOR");
            assert!(cbor.status.success());
            let decoded: Value = ciborium::from_reader(cbor.stdout.as_slice()).expect("decode");
            assert!(decoded["result"].as_array().expect("cards").contains(&card));
            for (index, level) in std::iter::once(None)
                .chain(ThinkingLevel::ALL.map(Some))
                .enumerate()
            {
                ask.wants.thinking = level;
                let before = serde_json::to_value(&ask).expect("before");
                let mut expected = json!({});
                expected[case.field] = if case.field == "thinkingBudget" {
                    json!(case.values[index].parse::<i32>().expect("budget"))
                } else {
                    json!(case.values[index])
                };
                if level.is_some_and(|level| level != ThinkingLevel::Off) {
                    expected["includeThoughts"] = json!(true);
                }
                let server =
                    response_server(listener.try_clone().expect("clone"), 200, FINISHED.into());
                let said = presets::events(&ask_request(
                    &dir,
                    &[("FIXTURE_KEY", "private-sentinel")],
                    &ask,
                ));
                let (headers, body) = server.join().expect("server");
                wire(api, &headers, &body, &expected);
                presets::stopped(&said, "end_turn");
                assert_eq!(serde_json::to_value(&ask).expect("after"), before);
                requests += 1;
            }
            presets::unasked(&listener);
        }
    }
    assert_eq!(requests, 112);
}

#[test]
fn inherited_modes_explicit_auto_and_model_overrides_survive_native_retries() {
    let mut requests = 0;
    for api in APIS {
        for override_mode in [None, Some("auto"), Some("budget"), Some("level")] {
            let mut model = model("private-model");
            model["thinking"] = json!({"xhigh":"XHIGH"});
            if let Some(mode) = override_mode {
                model["compat"] = json!({"google_thinking":mode});
            }
            let (dir, listener, mut ask) = fixture(api, model, json!({"google_thinking":"level"}));
            ask.wants.thinking = Some(ThinkingLevel::XHigh);
            let levels = cards(&dir, &[("FIXTURE_KEY", "private-sentinel")])["capabilities"]["thinking_levels"].clone();
            let discrete = matches!(override_mode, None | Some("level"));
            assert_eq!(
                levels.as_array().expect("levels").contains(&json!("xhigh")),
                discrete
            );
            let expected = if discrete {
                json!({"thinkingLevel":"XHIGH","includeThoughts":true})
            } else {
                json!({"thinkingBudget":-1,"includeThoughts":true})
            };
            let server = std::thread::spawn(move || {
                let first = response_server(
                    listener.try_clone().expect("clone"),
                    503,
                    "{\"error\":{\"message\":\"retry\"}}".into(),
                )
                .join()
                .expect("503");
                let second = response_server_with_timeout(
                    listener,
                    200,
                    FINISHED.into(),
                    Duration::from_secs(15),
                )
                .join()
                .expect("retry");
                (first, second)
            });
            let said = presets::events(&ask_request_with_timeout(
                &dir,
                &[("FIXTURE_KEY", "private-sentinel")],
                &ask,
                Duration::from_secs(30),
            ));
            let (first, second) = server.join().expect("server");
            wire(api, &first.0, &first.1, &expected);
            wire(api, &second.0, &second.1, &expected);
            assert_eq!(first.1, second.1);
            assert_eq!(
                said.iter()
                    .filter(|event| event["event"] == "retrying")
                    .count(),
                1
            );
            assert_eq!(
                said.iter().filter(|event| event["event"] == "stop").count(),
                1
            );
            assert!(!said.iter().any(|event| event["event"] == "failed"));
            requests += 2;
        }
    }
    assert_eq!(requests, 16);
}
