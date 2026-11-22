use super::*;

#[test]
fn credentials_unknown_models_invalid_sessions_and_required_schemas_refuse_before_network() {
    let (dir, listener) = fixture();
    for model in MODELS {
        for vars in [
            vec![],
            vec![(KEY, " \t\n")],
            vec![("OPENAI_API_KEY", "private-sentinel")],
        ] {
            let said = events(&ask_request(&dir, &vars, &ask(model)));
            assert_eq!(said.len(), 1);
            assert_eq!(said[0]["why"], "auth");
            unasked(&listener);
        }
        let mut ask = ask(model);
        ask.wants.session_id = Some("invalid\nheader".into());
        let said = events(&ask_request(&dir, &[], &ask));
        assert_eq!(said.len(), 1);
        assert_eq!(said[0]["why"], "invalid");
        unasked(&listener);
        ask.wants.session_id = None;
        ask.context.tools[0].strict = Some(ToolStrictness::Require);
        ask.context.tools[0].parameters = json!({"type":"object","properties":{"text":{"type":"string"}},"additionalProperties":true});
        let said = events(&ask_request(&dir, &[], &ask));
        assert_eq!(said.len(), 1);
        assert_eq!(said[0]["why"], "invalid");
        unasked(&listener);
    }
    let said = run(&dir, &ask("not-a-model"));
    assert_eq!(said.len(), 1);
    assert_eq!(said[0]["event"], "failed");
    unasked(&listener);
}

#[test]
fn every_model_has_terminal_auth_and_truncated_tool_failures_without_secret_leakage() {
    let (dir, listener) = fixture();
    for model in MODELS {
        for (status, response) in [
            (401, r#"{"error":{"message":"Authentication failed"}}"#.into()),
            (200, "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"a\",\"function\":{\"name\":\"echo\",\"arguments\":\"{\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\ndata: [DONE]\n\n".into()),
        ] {
            let server = response_server(listener.try_clone().expect("listener"), status, response);
            let said = run(&dir, &ask(model));
            let (headers, body) = server.join().expect("request");
            request(model, &headers, &body);
            assert!(!said.iter().any(|e| e["event"] == "retrying"));
            if status == 200 { stopped(&said, "error"); }
            else {
                assert!(said.iter().all(|e| matches!(e["event"].as_str(), Some("request_frame" | "failed"))), "{said:?}");
                assert_eq!(said.iter().filter(|e| e["event"] == "failed").count(), 1);
                assert_eq!(said.last().expect("terminal")["event"], "failed");
                assert_eq!(said.last().expect("terminal")["why"], "auth");
            }
            unasked(&listener);
        }
    }
}

#[test]
fn retries_preserve_templates_efforts_and_scoped_affinity_for_all_control_families() {
    for model in [
        "zai-org/GLM-5.2",
        "openai/gpt-oss-120b",
        "moonshotai/Kimi-K3.1-Pro",
    ] {
        let (dir, listener) = fixture();
        let ask = ask(model);
        let before = serde_json::to_value(&ask).expect("ask");
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
        let output = ask_request_with_timeout(
            &dir,
            &[(KEY, "private-sentinel")],
            &ask,
            std::time::Duration::from_secs(30),
        );
        assert!(!String::from_utf8_lossy(&output.stdout).contains("private-sentinel"));
        let said = events(&output);
        assert_eq!(said.iter().filter(|e| e["event"] == "retrying").count(), 1);
        assert!(!said.iter().any(|e| e["event"] == "failed"));
        let stops: Vec<_> = said.iter().filter(|e| e["event"] == "stop").collect();
        assert_eq!(stops.len(), 1);
        assert_eq!(stops[0]["reason"], "end_turn");
        let (first, second) = server.join().expect("server");
        request(model, &first.0, &first.1);
        request(model, &second.0, &second.1);
        controls(model, ask.wants.thinking, &second.1);
        assert_eq!(affinity(&first.0), affinity(&second.0));
        assert_eq!(first.1, second.1);
        assert_eq!(serde_json::to_value(&ask).expect("unchanged"), before);
    }
}
