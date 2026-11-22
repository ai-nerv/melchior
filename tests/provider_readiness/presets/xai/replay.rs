use super::*;
use melchior::mind::model::{Content, ThinkingLevel, ToolStrictness};

#[test]
fn opaque_reasoning_streamed_calls_and_tool_result_images_survive_each_model_followup() {
    for model in MODELS {
        let (dir, listener) = fixture();
        let output = json!([
            {"type":"reasoning","id":"rs","summary":[],"encrypted_content":"opaque α\u{1}+/="},
            {"type":"message","id":"msg","role":"assistant","content":[{"type":"output_text","text":"Checking","annotations":[]}]},
            {"type":"function_call","id":"fc","call_id":"call","name":"echo","arguments":"{\"text\":\"Zürich\"}"}
        ]);
        let mut script: String = [
            json!({"type":"response.output_item.done","output_index":0,"item":output[0]}),
            json!({"type":"response.output_item.done","output_index":1,"item":output[1]}),
            json!({"type":"response.output_item.added","output_index":2,"item":{
                "type":"function_call","id":"fc","call_id":"call","name":"echo","arguments":""}}),
            json!({"type":"response.function_call_arguments.delta","output_index":2,"item_id":"fc","delta":"{\"text\":"}),
            json!({"type":"response.function_call_arguments.delta","output_index":2,"item_id":"fc","delta":"\"Zürich\"}"}),
        ].iter().map(|event| format!("data: {event}\n\n")).collect();
        script.push_str(&completed(
            output.clone(),
            json!({"cost_in_usd_ticks":158500}),
        ));
        let worker = response_server(listener.try_clone().expect("clone"), 200, script);
        let mut ask = ask(model);
        let first = events(&ask_request(&dir, &[(KEY, "private-sentinel")], &ask));
        let (headers, sent) = worker.join().expect("tool request");
        request(model, &headers, &sent);
        stopped(&first, "tool_use");
        assert!(!first.iter().any(|event| event["event"] == "thinking"));
        assert_eq!(
            first
                .iter()
                .filter(|event| event["event"] == "text")
                .map(|event| event["text"].as_str().expect("text"))
                .collect::<String>(),
            "Checking"
        );
        let calls: Vec<_> = first
            .iter()
            .filter(|event| event["event"] == "tool_call_start")
            .collect();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["id"], "call");
        assert_eq!(calls[0]["name"], "echo");
        let args: String = first
            .iter()
            .filter(|event| event["event"] == "tool_call_args")
            .map(|event| event["args"].as_str().expect("arguments"))
            .collect();
        let arguments: Value = serde_json::from_str(&args).expect("arguments");
        assert_eq!(arguments, json!({"text":"Zürich"}));
        let signatures: Vec<_> = first
            .iter()
            .filter(|event| event["event"] == "signature")
            .collect();
        assert_eq!(signatures.len(), 1);
        let signature = signatures[0]["signature"].as_str().expect("signature");
        assert!(signature.starts_with("melchior.responses.v1:"));
        assert!(!signature.contains("private-sentinel"));
        ask.context.messages.push(
            serde_json::from_value(json!({"role":"assistant","content":[
            {"type":"thinking","thinking":"","signature":signature},
            {"type":"text","text":"Checking"},
            {"type":"tool_call","id":"call","name":"echo","arguments":arguments}]}))
            .expect("assistant"),
        );
        ask.context.messages.push(
            serde_json::from_value(json!({"role":"tool","content":[
            {"type":"tool_result","id":"call","name":"echo","content":"Zürich","is_error":false,
                "images":[{"media_type":"image/png","data":PNG}]}]}))
            .expect("tool result"),
        );
        let unchanged = ask.context.clone();
        let worker = response_server(listener, 200, completed(json!([]), json!({})));
        let next = events(&ask_request(&dir, &[(KEY, "private-sentinel")], &ask));
        let (headers, sent) = worker.join().expect("followup");
        request(model, &headers, &sent);
        stopped(&next, "end_turn");
        assert_eq!(sent["input"].as_array().expect("input").len(), 5);
        for index in 0..3 {
            assert_eq!(sent["input"][index + 1], output[index]);
        }
        assert_eq!(
            sent["input"][4],
            json!({"type":"function_call_output","call_id":"call","output":[
            {"type":"input_text","text":"Zürich"},{"type":"input_image","detail":"auto","image_url":format!("data:image/png;base64,{PNG}")}]})
        );
        assert_eq!(ask.context, unchanged);
    }
}

#[test]
fn model_specific_efforts_preserve_null_levels_and_the_grok_45_xhigh_alias() {
    for model in MODELS {
        let (dir, listener) = fixture();
        for level in [
            None,
            Some(ThinkingLevel::Off),
            Some(ThinkingLevel::Minimal),
            Some(ThinkingLevel::Low),
            Some(ThinkingLevel::Medium),
            Some(ThinkingLevel::High),
            Some(ThinkingLevel::XHigh),
            Some(ThinkingLevel::Max),
        ] {
            let mut ask = ask(model);
            ask.wants.thinking = level;
            let worker = response_server(
                listener.try_clone().expect("clone"),
                200,
                completed(json!([]), json!({})),
            );
            let output = events(&ask_request(&dir, &[(KEY, "private-sentinel")], &ask));
            let (headers, body) = worker.join().expect("effort request");
            request(model, &headers, &body);
            stopped(&output, "end_turn");
            let expected = match level {
                Some(ThinkingLevel::Off) if model == "grok-4.3" => Some("none"),
                Some(ThinkingLevel::Low) => Some("low"),
                Some(ThinkingLevel::Medium) => Some("medium"),
                Some(ThinkingLevel::High) => Some("high"),
                Some(ThinkingLevel::XHigh) => {
                    Some(if model == "grok-4.5" { "high" } else { "xhigh" })
                }
                _ => None,
            };
            if let Some(effort) = expected {
                assert_eq!(body["reasoning"], json!({"effort":effort,"summary":"auto"}));
            } else {
                assert!(body.get("reasoning").is_none(), "{model} {level:?}: {body}");
            }
        }
    }
}

#[test]
fn oversized_images_and_required_strict_tools_refuse_before_credentials_or_http() {
    for model in MODELS {
        let (dir, listener) = fixture();
        let mut strict = ask(model);
        strict.context.tools[0].strict = Some(ToolStrictness::Require);
        let mut large = ask(model);
        large.context.messages[0].content[1] = Content::Image {
            media_type: "image/png".into(),
            data: "A".repeat(6_291_460),
        };
        for ask in [strict, large] {
            let output = events(&ask_request(&dir, &[], &ask));
            assert_eq!(output.len(), 1, "{output:?}");
            assert_eq!(output[0]["event"], "failed");
            assert_eq!(output[0]["why"], "invalid");
            unasked(&listener);
        }
    }
}

#[test]
fn nonretryable_provider_errors_keep_their_classification_for_all_four_models() {
    for model in MODELS {
        for (status, why) in [(400, "invalid"), (401, "auth"), (422, "invalid")] {
            let (dir, listener) = fixture();
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
fn retry_preserves_shipped_headers_images_and_effective_reasoning_options() {
    let (dir, listener) = fixture();
    let following = listener.try_clone().expect("retry listener");
    let remaining = listener.try_clone().expect("extra request observer");
    let worker = std::thread::spawn(move || {
        let first = response_server(
            listener,
            503,
            json!({"error":{"message":"temporary"}}).to_string(),
        )
        .join()
        .expect("first attempt");
        let next = response_server_with_timeout(
            following,
            200,
            completed(json!([]), json!({"cost_in_usd_ticks":158500})),
            std::time::Duration::from_secs(15),
        )
        .join()
        .expect("retry");
        [first, next]
    });
    let output = events(&ask_request_with_timeout(
        &dir,
        &[(KEY, "private-sentinel")],
        &ask("grok-4.7"),
        std::time::Duration::from_secs(25),
    ));
    let requests = worker.join().expect("fixture");
    for (headers, body) in &requests {
        request("grok-4.7", headers, body);
    }
    assert_eq!(requests[0].1, requests[1].1);
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
    unasked(&remaining);
}
