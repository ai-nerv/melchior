use super::*;

fn spent(events: &[Value]) -> &Value {
    &events
        .iter()
        .rev()
        .find(|e| e["event"] == "spent")
        .expect("spent")["usage"]
}

#[test]
fn m3_prices_select_long_context_tiers_using_cached_and_uncached_tokens() {
    for (id, key, endpoint) in REGIONS {
        let (dir, listener) = fixture(id, key, endpoint);
        let estimates = if id == "minimax" {
            [129614, 259229]
        } else {
            [135529, 271111]
        };
        for ((input, total), estimate) in [(412000, 512000), (412001, 512001)]
            .into_iter()
            .zip(estimates)
        {
            let mut body = anthropic::stream(&json!([{"type":"text","text":"ok"}]), "end_turn");
            body = body
                .replace("\"input_tokens\":60", &format!("\"input_tokens\":{input}"))
                .replace(
                    "\"cache_read_input_tokens\":30",
                    "\"cache_read_input_tokens\":100000",
                );
            let worker = response_server(listener.try_clone().expect("listener"), 200, body);
            let said = events(&ask_request(
                &dir,
                &[(key, "private-sentinel")],
                &ask(id, MODELS[0]),
            ));
            let (headers, body) = worker.join().expect("request");
            request(MODELS[0], &headers, &body);
            stopped(&said, "end_turn");
            assert_eq!(spent(&said)["input"], input);
            assert_eq!(spent(&said)["cache_read"], 100000);
            assert_eq!(spent(&said)["output"], 12);
            assert_eq!(spent(&said)["estimated_micros"], estimate, "{id} {total}");
        }
    }
}

#[test]
fn output_defaults_and_explicit_caps_never_create_legacy_thinking_budgets() {
    for (id, key, endpoint) in REGIONS {
        let (dir, listener) = fixture(id, key, endpoint);
        for model in MODELS {
            for (maximum, expected) in [(None, 32000), (Some(1), 1), (Some(999999), 524288)] {
                let mut ask = ask(id, model);
                ask.wants.max_tokens = maximum;
                let worker = response_server(
                    listener.try_clone().expect("listener"),
                    200,
                    anthropic::stream(&json!([{"type":"text","text":"ok"}]), "end_turn"),
                );
                let said = events(&ask_request(&dir, &[(key, "private-sentinel")], &ask));
                let (_, body) = worker.join().expect("request");
                stopped(&said, "end_turn");
                assert_eq!(body["max_tokens"], expected);
                assert_eq!(body["thinking"]["type"], "adaptive");
                assert!(body["thinking"].get("budget_tokens").is_none());
            }
        }
    }
}

#[test]
fn transient_retry_preserves_each_regions_model_images_and_effort_settings() {
    for (id, key, endpoint) in REGIONS {
        for model in MODELS {
            let (dir, listener) = fixture(id, key, endpoint);
            let next = listener.try_clone().expect("retry listener");
            let remaining = listener.try_clone().expect("observer");
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
                    anthropic::stream(&json!([{"type":"text","text":"ok"}]), "end_turn"),
                    std::time::Duration::from_secs(15),
                )
                .join()
                .expect("second");
                [first, second]
            });
            let said = events(&ask_request_with_timeout(
                &dir,
                &[(key, "private-sentinel")],
                &ask(id, model),
                std::time::Duration::from_secs(25),
            ));
            let captured = worker.join().expect("worker");
            for (headers, body) in &captured {
                request(model, headers, body);
            }
            assert_eq!(captured[0].1, captured[1].1);
            assert_eq!(said.iter().filter(|e| e["event"] == "retrying").count(), 1);
            assert_eq!(
                said.iter()
                    .filter(|e| e["event"] == "stop" && e["reason"] == "end_turn")
                    .count(),
                1
            );
            assert!(!said.iter().any(|e| e["event"] == "failed"), "{said:?}");
            unasked(&remaining);
        }
    }
}
