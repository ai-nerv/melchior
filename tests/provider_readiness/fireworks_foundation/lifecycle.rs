use super::*;

fn protocols() -> Vec<Value> {
    [true, false]
        .into_iter()
        .map(|is_anthropic| {
            rows()
                .into_iter()
                .find(|row| anthropic(row) == is_anthropic)
                .expect("protocol")
        })
        .collect()
}

#[test]
fn anthropic_affinity_partitions_credentials_sessions_and_endpoints_without_raw_ids() {
    let row = protocols().remove(0);
    let (dir, listener, provider) = fixture();
    let mut keys = Vec::new();
    for change in [
        "initial",
        "repeat",
        "credential",
        "session",
        "endpoint",
        "absent",
        "disabled",
    ] {
        let mut declaration = provider.clone();
        let mut ask = request(&row);
        let credential = if change == "credential" {
            "private-sentinel-other"
        } else {
            "private-sentinel"
        };
        if change == "session" {
            ask.wants.session_id = Some("private-sentinel-other-session".into());
        }
        if change == "absent" {
            ask.wants.session_id = None;
        }
        let model = declaration["models"]
            .as_array_mut()
            .expect("overrides")
            .iter_mut()
            .find(|model| model["id"] == row["id"])
            .expect("model");
        if change == "endpoint" {
            model["base_url"] = json!(format!(
                "{}/other",
                model["base_url"].as_str().expect("URL")
            ));
        }
        if change == "disabled" {
            model["compat"] = json!({"send_session_affinity_headers":false});
        }
        write(&dir, &declaration);
        let server = response_server(listener.try_clone().expect("listener"), 200, finished(&row));
        let said = events(&ask_request(&dir, &[(KEY, credential)], &ask));
        presets::stopped(&said, "end_turn");
        let (headers, body) = server.join().expect("request");
        if matches!(change, "absent" | "disabled") {
            assert!(!headers.contains("x-session-affinity:"));
            assert!(!headers.contains("x-client-request-id:"));
            assert!(!headers.contains("session_id:"));
        } else {
            keys.push(affinity(&headers, &row));
        }
        assert!(!body.to_string().contains("private-sentinel"));
        assert!(body.get("prompt_cache_key").is_none());
    }
    assert_eq!(keys.len(), 5);
    assert_eq!(keys[0], keys[1]);
    assert_eq!(keys.iter().collect::<BTreeSet<_>>().len(), 4);
    unasked(&listener);
}

#[test]
fn both_protocols_retry_identical_native_schema_and_scoped_affinity() {
    for row in protocols() {
        let (dir, listener, _) = fixture();
        let mut ask = request(&row);
        ask.wants.thinking = Some(ThinkingLevel::High);
        let response = finished(&row);
        let server = std::thread::spawn(move || {
            let first = response_server(
                listener.try_clone().expect("listener"),
                503,
                r#"{"error":{"type":"overloaded_error","message":"Overloaded"}}"#.into(),
            )
            .join()
            .expect("first");
            let second = response_server_with_timeout(
                listener,
                200,
                response,
                std::time::Duration::from_secs(15),
            )
            .join()
            .expect("second");
            (first, second)
        });
        let said = events(&ask_request_with_timeout(
            &dir,
            &[(KEY, "private-sentinel")],
            &ask,
            std::time::Duration::from_secs(30),
        ));
        assert_eq!(said.iter().filter(|e| e["event"] == "retrying").count(), 1);
        assert!(!said.iter().any(|e| e["event"] == "failed"));
        let stops: Vec<_> = said.iter().filter(|e| e["event"] == "stop").collect();
        assert_eq!(stops.len(), 1);
        assert_eq!(stops[0]["reason"], "end_turn");
        let (first, second) = server.join().expect("retry server");
        assert_eq!(first.1, second.1);
        assert_eq!(affinity(&first.0, &row), affinity(&second.0, &row));
    }
}

#[test]
fn both_protocols_preserve_terminal_auth_and_malformed_stream_failures() {
    for row in protocols() {
        let (dir, listener, _) = fixture();
        let malformed = if anthropic(&row) {
            event(
                "message_start",
                json!({"message":{"role":"assistant","content":[],"usage":{"input_tokens":0}}}),
            ) + &event(
                "content_block_start",
                json!({"index":0,"content_block":{"type":"text","text":"partial"}}),
            ) + &event("message_stop", json!({}))
        } else {
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"a\",\"function\":{\"name\":\"echo\",\"arguments\":\"{\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\ndata: [DONE]\n\n".into()
        };
        for (status, response) in [
            (
                401,
                r#"{"error":{"type":"authentication_error","message":"Authentication failed"}}"#
                    .into(),
            ),
            (200, malformed),
        ] {
            let server = response_server(listener.try_clone().expect("listener"), status, response);
            let said = events(&ask_request(
                &dir,
                &[(KEY, "private-sentinel")],
                &request(&row),
            ));
            server.join().expect("request");
            assert!(!said.iter().any(|e| e["event"] == "retrying"));
            if status == 401 {
                assert_eq!(said.iter().filter(|e| e["event"] == "failed").count(), 1);
                assert_eq!(said.last().expect("terminal")["why"], "auth");
            } else {
                presets::stopped(&said, "error");
            }
            unasked(&listener);
        }
    }
}
