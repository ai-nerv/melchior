use super::*;

#[test]
fn every_model_has_terminal_auth_and_truncated_tool_failures_without_retry() {
    let (dir, listener) = fixture();
    let mut requests = 0;
    for row in rows() {
        for (status,response) in [
            (401,r#"{"error":{"message":"Authentication failed"}}"#.into()),
            (200,"data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"a\",\"function\":{\"name\":\"echo\",\"arguments\":\"{\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\ndata: [DONE]\n\n".into()),
        ] {
            let server = response_server(listener.try_clone().expect("listener"),status,response);
            let said = presets::events(&ask_request(&dir,&[(KEY,"private-sentinel")],&request(&row)));
            let (headers,body) = server.join().expect("request");
            wire(&row,&headers,&body);
            requests += 1;
            assert!(!said.iter().any(|event| event["event"]=="retrying"));
            if status==401 {
                assert_eq!(said.iter().filter(|event| event["event"]=="failed").count(),1);
                assert_eq!(said.last().expect("terminal")["why"],"auth");
                assert!(!said.iter().any(|event| event["event"]=="stop"));
            } else { presets::stopped(&said,"error"); }
            presets::unasked(&listener);
        }
    }
    assert_eq!(requests, 174);
}

#[test]
fn retries_preserve_nullable_extended_and_nonreasoning_requests() {
    let requests: usize = std::thread::scope(|scope| {
        let tasks: Vec<_> = [
            ("Qwen/Qwen3.8-27B", ThinkingLevel::High),
            ("deepseek-ai/DeepSeek-V4.1-Flash", ThinkingLevel::XHigh),
            ("meta-llama/Llama-3.1-8B-Instruct", ThinkingLevel::High),
        ]
        .into_iter()
        .map(|(id, level)| {
            scope.spawn(move || {
                let (dir, listener) = fixture();
                let row = rows()
                    .into_iter()
                    .find(|row| row["id"] == id)
                    .expect("model");
                let mut ask = request(&row);
                ask.wants.thinking = Some(level);
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
                    &[(KEY, "private-sentinel")],
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
                    said.iter().filter(|event| event["event"] == "stop").count(),
                    1
                );
                assert_eq!(said.last().expect("terminal")["reason"], "end_turn");
                assert!(!said.iter().any(|event| event["event"] == "failed"));
                let (first, second) = server.join().expect("server");
                wire(&row, &first.0, &first.1);
                wire(&row, &second.0, &second.1);
                controls(&row, ask.wants.thinking, &second.1);
                assert_eq!(first.1, second.1);
                assert_eq!(serde_json::to_value(&ask).expect("unchanged"), before);
                2
            })
        })
        .collect();
        tasks
            .into_iter()
            .map(|task| task.join().expect("control family"))
            .sum()
    });
    assert_eq!(requests, 6);
}
