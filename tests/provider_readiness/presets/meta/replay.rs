use super::*;

#[test]
fn every_model_replays_encrypted_reasoning_and_tools_but_never_replays_it_to_another_model() {
    let (dir, listener) = fixture();
    for (index, (model, ..)) in MODELS.iter().enumerate() {
        let output = json!([
            {"type":"reasoning","id":"rs_meta","summary":[],"encrypted_content":"opaque Meta α+/="},
            {"type":"function_call","id":"fc","call_id":"call","name":"echo","arguments":"{\"text\":\"Zürich\"}"}
        ]);
        let mut script: String = [
            json!({"type":"response.output_item.done","output_index":0,"item":output[0]}),
            json!({"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","id":"fc","call_id":"call","name":"echo","arguments":""}}),
            json!({"type":"response.function_call_arguments.delta","output_index":1,"item_id":"fc","delta":"{\"text\":"}),
            json!({"type":"response.function_call_arguments.delta","output_index":1,"item_id":"fc","delta":"\"Zürich\"}"}),
        ].iter().map(|event|format!("data: {event}\n\n")).collect();
        script.push_str(&completed(output.clone(), usage()));
        let worker = response_server(listener.try_clone().expect("clone"), 200, script);
        let mut ask = ask(model);
        let first = events(&ask_request(&dir, &[(KEY, "private-sentinel")], &ask));
        let (headers, body) = worker.join().expect("tool request");
        request(model, &headers, &body);
        stopped(&first, "tool_use");
        assert!(
            !first
                .iter()
                .any(|event| matches!(event["event"].as_str(), Some("thinking" | "text")))
        );
        let signatures: Vec<_> = first
            .iter()
            .filter(|event| event["event"] == "signature")
            .collect();
        assert_eq!(signatures.len(), 1);
        let signature = signatures[0]["signature"].as_str().expect("signature");
        assert!(!signature.contains("private-sentinel"));
        let calls: Vec<_> = first
            .iter()
            .filter(|event| event["event"] == "tool_call_start")
            .collect();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["id"], "call");
        assert_eq!(calls[0]["name"], "echo");
        let arguments: String = first
            .iter()
            .filter(|event| event["event"] == "tool_call_args")
            .map(|event| event["args"].as_str().expect("args"))
            .collect();
        let arguments: Value = serde_json::from_str(&arguments).expect("arguments");
        assert_eq!(arguments, json!({"text":"Zürich"}));
        ask.context.messages.push(
            serde_json::from_value(json!({"role":"assistant","content":[
            {"type":"thinking","thinking":"","signature":signature},
            {"type":"tool_call","id":"call","name":"echo","arguments":arguments}]}))
            .expect("assistant"),
        );
        ask.context.messages.push(
            serde_json::from_value(json!({"role":"tool","content":[
            {"type":"tool_result","id":"call","name":"echo","content":"Zürich","is_error":false,
                "images":[{"media_type":"image/png","data":PNG}]}]}))
            .expect("tool result"),
        );
        let saved = ask.context.clone();
        let result = json!({"type":"function_call_output","call_id":"call","output":[
            {"type":"input_text","text":"Zürich"},{"type":"input_image","detail":"auto","image_url":format!("data:image/png;base64,{PNG}")} ]});
        for same_model in [true, false] {
            let target = if same_model {
                *model
            } else {
                MODELS[(index + 1) % MODELS.len()].0
            };
            ask.model = format!("meta/{target}");
            let worker = response_server(
                listener.try_clone().expect("clone"),
                200,
                completed(
                    json!([{
                "type":"message","id":"msg","role":"assistant","content":[{"type":"output_text","text":"ok","annotations":[]}]}]),
                    json!({}),
                ),
            );
            let next = events(&ask_request(&dir, &[(KEY, "private-sentinel")], &ask));
            let (headers, body) = worker.join().expect("followup");
            request(target, &headers, &body);
            stopped(&next, "end_turn");
            assert_eq!(
                next.iter()
                    .filter(|event| event["event"] == "text")
                    .map(|event| event["text"].as_str().expect("text"))
                    .collect::<String>(),
                "ok"
            );
            let input = body["input"].as_array().expect("input");
            if same_model {
                assert_eq!(input.len(), 4);
                assert_eq!(input[1], output[0]);
                assert_eq!(input[2], output[1]);
            } else {
                assert_eq!(input.len(), 3);
                assert!(!body.to_string().contains("encrypted_content\":\"opaque"));
                assert_eq!(
                    input[1],
                    json!({"type":"function_call","call_id":"call","name":"echo","arguments":"{\"text\":\"Zürich\"}"})
                );
            }
            assert_eq!(input.last().expect("tool result"), &result);
            assert_eq!(ask.context, saved);
        }
    }
}

#[test]
fn authentication_and_invalid_provider_failures_keep_their_type_and_never_retry() {
    let (dir, listener) = fixture();
    for (model, ..) in MODELS {
        for (status, why) in [(401, "auth"), (400, "invalid"), (422, "invalid")] {
            let worker = response_server(
                listener.try_clone().expect("clone"),
                status,
                json!({"error":{"message":"fixture refusal"}}).to_string(),
            );
            let output = events(&ask_request(
                &dir,
                &[(KEY, "private-sentinel")],
                &ask(model),
            ));
            let (headers, body) = worker.join().expect("refused request");
            request(model, &headers, &body);
            let failed: Vec<_> = output
                .iter()
                .filter(|event| event["event"] == "failed")
                .collect();
            assert_eq!(failed.len(), 1, "{output:?}");
            assert_eq!(failed[0]["why"], why);
            assert!(
                !output
                    .iter()
                    .any(|event| matches!(event["event"].as_str(), Some("stop" | "retrying")))
            );
            assert!(
                !serde_json::to_string(&output)
                    .expect("wire")
                    .contains("private-sentinel")
            );
            unasked(&listener);
        }
    }
}

#[test]
fn transient_retry_preserves_the_requested_standard_model_and_does_not_switch_to_contributor() {
    let (dir, listener) = fixture();
    let next = listener.try_clone().expect("retry listener");
    let remaining = listener.try_clone().expect("extra request observer");
    let worker = std::thread::spawn(move || {
        let first = response_server(
            listener,
            503,
            json!({"error":{"message":"temporary"}}).to_string(),
        )
        .join()
        .expect("first");
        let second = response_server_with_timeout(
            next,
            200,
            completed(json!([]), usage()),
            std::time::Duration::from_secs(15),
        )
        .join()
        .expect("second");
        [first, second]
    });
    let output = events(&ask_request_with_timeout(
        &dir,
        &[(KEY, "private-sentinel")],
        &ask("muse-spark-1.3"),
        std::time::Duration::from_secs(25),
    ));
    let captured = worker.join().expect("fixture");
    for (headers, body) in &captured {
        request("muse-spark-1.3", headers, body);
    }
    assert_eq!(captured[0].1, captured[1].1);
    assert_eq!(
        output
            .iter()
            .filter(|event| event["event"] == "retrying")
            .count(),
        1
    );
    assert!(
        !output.iter().any(|event| event["event"] == "failed"),
        "{output:?}"
    );
    let stops: Vec<_> = output
        .iter()
        .filter(|event| event["event"] == "stop")
        .collect();
    assert_eq!(stops.len(), 1);
    assert_eq!(stops[0]["reason"], "end_turn");
    assert_eq!(spent(&output)["estimated_micros"], 131);
    unasked(&remaining);
}
