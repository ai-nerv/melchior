use super::*;

#[test]
fn invalid_sessions_schemas_and_unknown_models_refuse_before_credentials_or_http() {
    let mut refused = 0;
    for (id, key, fallback, count) in PLANS {
        let (dir, listener) = fixture(id, key, fallback, count);
        for row in rows(id) {
            let mut ask = request(id, &row);
            ask.wants.session_id = Some("bad\nheader".into());
            let said = presets::events(&ask_request(&dir, &[], &ask));
            assert_eq!(said.len(), 1);
            assert_eq!(said[0]["why"], "invalid");
            presets::unasked(&listener);
            refused += 1;
            ask.wants.session_id = None;
            ask.context.tools[0].parameters = json!({"type":"object","properties":{"value":{"type":"string"}},"additionalProperties":true});
            let said = presets::events(&ask_request(&dir, &[], &ask));
            assert_eq!(said.len(), 1);
            assert_eq!(said[0]["why"], "invalid");
            presets::unasked(&listener);
            refused += 1;
        }
        let mut ask = request(id, &rows(id)[0]);
        ask.model = format!("{id}/not-a-model");
        let said = presets::events(&ask_request(&dir, &[], &ask));
        assert_eq!(said.len(), 1);
        assert_eq!(said[0]["event"], "failed");
        presets::unasked(&listener);
        refused += 1;
    }
    assert_eq!(refused, 136);
}

#[test]
fn every_shipped_model_has_terminal_auth_and_truncated_tool_failures_without_retry() {
    let mut requests = 0;
    for (id, key, fallback, count) in PLANS {
        let (dir, listener) = fixture(id, key, fallback, count);
        for row in rows(id) {
            for (status, body) in [
                (401, r#"{"error":{"message":"Authentication failed"}}"#.to_owned()),
                (200, "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"a\",\"function\":{\"name\":\"echo\",\"arguments\":\"{\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\ndata: [DONE]\n\n".into()),
            ] {
                let server = response_server(listener.try_clone().expect("listener"), status, body);
                let said = presets::events(&ask_request(&dir, &[(key, "private-sentinel")], &request(id, &row)));
                let (headers, body) = server.join().expect("request");
                requests += 1;
                assert!(headers.starts_with("post /compatible-mode/v1/chat/completions "));
                assert_eq!(body["model"], row["id"]);
                assert_eq!(body["max_completion_tokens"], 128);
                assert!(!said.iter().any(|event| event["event"] == "retrying"));
                if status == 401 {
                    assert_eq!(said.iter().filter(|event| event["event"] == "failed").count(), 1);
                    assert_eq!(said.last().expect("terminal")["why"], "auth");
                } else {
                    presets::stopped(&said, "error");
                }
                presets::unasked(&listener);
            }
        }
    }
    assert_eq!(requests, 132);
}

#[test]
fn retries_preserve_routes_keys_output_caps_and_each_plan_control_family() {
    let requests: usize = std::thread::scope(|scope| {
        let tasks: Vec<_> = PLANS
            .into_iter()
            .map(|(id, key, fallback, count)| {
                scope.spawn(move || {
                    let mut requests = 0;
                    for model in ["deepseek-v4-pro", "qwen3.8-max", "qwen3.7-max"] {
                        let (dir, listener) = fixture(id, key, fallback, count);
                        let row = rows(id)
                            .into_iter()
                            .find(|row| row["id"] == model)
                            .expect("model");
                        let mut ask = request(id, &row);
                        ask.wants.thinking = Some(if model == "qwen3.8-max" {
                            ThinkingLevel::XHigh
                        } else {
                            ThinkingLevel::High
                        });
                        let before = serde_json::to_value(&ask).expect("snapshot");
                        let server = std::thread::spawn(move || {
                            let first = response_server(
                                listener.try_clone().expect("listener"),
                                503,
                                r#"{"error":{"message":"Service overloaded"}}"#.into(),
                            )
                            .join()
                            .expect("first");
                            let second = response_server_with_timeout(
                                listener,
                                200,
                                finished(),
                                std::time::Duration::from_secs(15),
                            )
                            .join()
                            .expect("retry");
                            (first, second)
                        });
                        let said = presets::events(&ask_request_with_timeout(
                            &dir,
                            &[(key, "private-sentinel")],
                            &ask,
                            std::time::Duration::from_secs(30),
                        ));
                        assert_eq!(
                            said.iter()
                                .filter(|event| event["event"] == "retrying")
                                .count(),
                            1
                        );
                        assert_eq!(
                            said.iter()
                                .filter(|event| event["event"] == "stop"
                                    && event["reason"] == "end_turn")
                                .count(),
                            1
                        );
                        assert!(!said.iter().any(|event| event["event"] == "failed"));
                        let (first, second) = server.join().expect("server");
                        requests += 2;
                        assert_eq!(first.1, second.1);
                        assert!(
                            first
                                .0
                                .contains("authorization: bearer private-sentinel\r\n")
                        );
                        assert!(
                            second
                                .0
                                .contains("authorization: bearer private-sentinel\r\n")
                        );
                        assert_eq!(second.1["max_completion_tokens"], 128);
                        assert_eq!(second.1["enable_thinking"], true);
                        assert_eq!(
                            second.1.get("reasoning_effort").and_then(Value::as_str),
                            match model {
                                "deepseek-v4-pro" => Some("high"),
                                "qwen3.8-max" => Some("xhigh"),
                                _ => None,
                            }
                        );
                        assert_eq!(serde_json::to_value(&ask).expect("unchanged"), before);
                    }
                    requests
                })
            })
            .collect();
        tasks
            .into_iter()
            .map(|task| task.join().expect("plan retry fixture"))
            .sum()
    });
    assert_eq!(requests, 24);
}
