use super::*;
use melchior::mind::wire::Ask;
use std::time::Duration;

const APIS: [&str; 2] = ["google-generative-ai", "google-vertex"];

fn fixture(api: &str) -> (Scratch, TcpListener, Ask) {
    let dir = Scratch::new("melchior-ready", "google-usage");
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let url = format!("http://{}/v1", listener.local_addr().expect("address"));
    config(&dir, &url, r#"{kind="api-key",vars={"FIXTURE_KEY"}}"#, api);
    let path = dir.join("providers.lua");
    let source = std::fs::read_to_string(&path).expect("fixture");
    std::fs::write(
        path,
        source.replace(
            "max_tokens=128",
            "max_tokens=128,cost={input=2,output=3,cache_read=0.5}",
        ),
    )
    .expect("costs");
    let ask = serde_json::from_value(json!({"model":"fixture/m","about":"usage-fixture",
        "wants":{"max_tokens":128},"context":{"messages":[{"role":"user","content":[{"type":"text","text":"hello"}]}]}})).expect("ask");
    (dir, listener, ask)
}

fn event(candidate: u64, thoughts: u64, finish: Option<&str>) -> String {
    let mut value = json!({"usageMetadata":{"promptTokenCount":100,"cachedContentTokenCount":40,
        "candidatesTokenCount":candidate,"thoughtsTokenCount":thoughts},
        "candidates":[{"content":{"parts":[{"text":"answer"}]}}]});
    if let Some(finish) = finish {
        value["candidates"][0]["finishReason"] = json!(finish);
    }
    format!("data: {value}\n\n")
}

fn spent(events: &[Value], attempts: u64, output: u64, price: u64) {
    let value = &events
        .iter()
        .rfind(|event| event["event"] == "spent")
        .expect("spent")["usage"];
    assert_eq!(value["input"], 60 * attempts, "{events:?}");
    assert_eq!(value["cache_read"], 40 * attempts);
    assert_eq!(value["output"], output);
    assert_eq!(value["cost_micros"], 0);
    assert_eq!(value["estimated_micros"], price);
}

fn wire(api: &str, headers: &str, body: &Value) {
    assert!(headers.contains(if api == "google-generative-ai" {
        "x-goog-api-key: private-sentinel\r\n"
    } else {
        "authorization: bearer private-sentinel\r\n"
    }));
    assert_eq!(body["generationConfig"]["maxOutputTokens"], 128);
    assert!(body.get("__scratch").is_none());
    assert!(body.get("__budget").is_none());
}

#[test]
fn late_thought_usage_is_priced_after_success_length_and_error() {
    let mut requests = 0;
    for api in APIS {
        for (finish, reason) in [
            ("STOP", "end_turn"),
            ("MAX_TOKENS", "length"),
            ("SAFETY", "error"),
        ] {
            let (dir, listener, ask) = fixture(api);
            let script = event(7, 5, Some(finish))
                + &format!(
                    "data: {}\n\n",
                    json!({"usageMetadata":{"candidatesTokenCount":8,"thoughtsTokenCount":11}})
                );
            let server = response_server(listener, 200, script);
            let events = presets::events(&ask_request(
                &dir,
                &[("FIXTURE_KEY", "private-sentinel")],
                &ask,
            ));
            let (headers, body) = server.join().expect("server");
            requests += 1;
            wire(api, &headers, &body);
            assert_eq!(
                events
                    .iter()
                    .filter(|event| event["event"] == "stop")
                    .count(),
                1,
                "{events:?}"
            );
            assert_eq!(
                events.last().expect("terminal")["reason"],
                reason,
                "{events:?}"
            );
            spent(&events, 1, 19, 197);
        }
    }
    assert_eq!(requests, 6);
}

#[test]
fn retry_charges_partial_thoughts_once_and_resets_component_snapshots() {
    let mut requests = 0;
    for api in APIS {
        let (dir, listener, ask) = fixture(api);
        let server = std::thread::spawn(move || {
            let first = response_server_sized(
                listener.try_clone().expect("clone"),
                200,
                event(2, 4, None),
                Duration::from_secs(10),
                11,
                || {},
            )
            .join()
            .expect("partial");
            let second = response_server_with_timeout(
                listener,
                200,
                event(7, 5, Some("STOP")),
                Duration::from_secs(15),
            )
            .join()
            .expect("retry");
            (first, second)
        });
        let before = serde_json::to_value(&ask).expect("before");
        let events = presets::events(&ask_request_with_timeout(
            &dir,
            &[("FIXTURE_KEY", "private-sentinel")],
            &ask,
            Duration::from_secs(30),
        ));
        let (first, second) = server.join().expect("server");
        requests += 2;
        wire(api, &first.0, &first.1);
        wire(api, &second.0, &second.1);
        assert_eq!(first.1, second.1);
        assert_eq!(serde_json::to_value(&ask).expect("after"), before);
        assert_eq!(
            events
                .iter()
                .filter(|event| event["event"] == "retrying")
                .count(),
            1,
            "{events:?}"
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| event["event"] == "stop")
                .count(),
            1
        );
        assert_eq!(events.last().expect("terminal")["reason"], "end_turn");
        assert!(!events.iter().any(|event| event["event"] == "failed"));
        spent(&events, 2, 18, 334);
    }
    assert_eq!(requests, 4);
}
