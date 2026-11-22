use super::*;
use melchior::mind::spending::Ledger;

fn usage(output: &[Value]) -> &Value {
    &output
        .iter()
        .rev()
        .find(|event| event["event"] == "spent")
        .expect("usage")["usage"]
}

#[test]
fn cached_prompt_boundary_selects_pinned_tiers_and_reported_ticks_override_estimates() {
    for model in MODELS {
        let (dir, listener) = fixture();
        let estimates = match model {
            "grok-4.3" => [145_030, 290_063],
            "grok-4.5" => [230_072, 460_148],
            _ => [250_072, 500_148],
        };
        for (total, estimated) in [200_000, 200_001].into_iter().zip(estimates) {
            let mut reported = json!({"input_tokens":total,"output_tokens":12,"input_tokens_details":{"cached_tokens":100_000}});
            for ticks in [false, true] {
                if ticks {
                    reported["cost_in_usd_ticks"] = json!(158500);
                }
                let worker = response_server(
                    listener.try_clone().expect("clone"),
                    200,
                    completed(json!([]), reported.clone()),
                );
                let events = events(&ask_request(
                    &dir,
                    &[(KEY, "private-sentinel")],
                    &ask(model),
                ));
                let (headers, body) = worker.join().expect("priced request");
                request(model, &headers, &body);
                stopped(&events, "end_turn");
                let usage = usage(&events);
                assert_eq!(usage["input"], total - 100_000);
                assert_eq!(usage["cache_read"], 100_000);
                if ticks {
                    assert_eq!(usage["cost_micros"], 16);
                    assert_eq!(usage["estimated_micros"], 0);
                } else {
                    assert_eq!(usage["cost_micros"], 0);
                    assert_eq!(usage["estimated_micros"], estimated);
                }
            }
        }
    }
}

#[test]
fn reported_tick_charge_is_committed_once_and_blocks_the_next_ask_for_every_terminal_kind() {
    for (kind, reason) in [
        ("completed", "end_turn"),
        ("incomplete", "length"),
        ("failed", "error"),
    ] {
        let (dir, listener) = fixture();
        std::fs::create_dir_all(dir.join("plugin")).expect("plugins");
        std::fs::write(dir.join("plugin/cap.lua"), "melchior.spend_cap_usd=0.25").expect("cap");
        melchior::places::ensure(&dir.join("runtime")).expect("private runtime");
        let terminal = format!(
            "data: {}\n\n",
            json!({"type":format!("response.{kind}"),"response":{
            "output":[],"incomplete_details":{"reason":"max_output_tokens"},
            "usage":{"input_tokens":90,"output_tokens":12,"input_tokens_details":{"cached_tokens":30},
                "cost_in_usd_ticks":2500000000_u64}}})
        );
        let worker = response_server(
            listener.try_clone().expect("clone"),
            200,
            terminal.repeat(2),
        );
        let env = [
            (KEY, "private-sentinel"),
            ("MAGI_MELCHIOR_SESSION", "xai-charge-run"),
        ];
        let first = events(&ask_request(&dir, &env, &ask("grok-4.7")));
        let (headers, body) = worker.join().expect("charged request");
        request("grok-4.7", &headers, &body);
        stopped(&first, reason);
        assert_eq!(
            first
                .iter()
                .filter(|event| event["event"] == "spent")
                .count(),
            1
        );
        assert_eq!(usage(&first)["cost_micros"], 250000);
        assert_eq!(usage(&first)["estimated_micros"], 0);
        let ledger = Ledger::at(&dir.join("runtime/melchior/.spent"), "xai-charge-run");
        assert_eq!(ledger.spent(), 250000);
        let second = events(&ask_request(&dir, &env, &ask("grok-4.7")));
        assert_eq!(second.len(), 1, "{second:?}");
        assert_eq!(second[0]["event"], "failed");
        assert!(
            second[0]["message"]
                .as_str()
                .expect("refusal")
                .contains("cap")
        );
        assert_eq!(ledger.spent(), 250000);
        unasked(&listener);
    }
}

#[test]
fn a_truncated_billed_response_remains_charged_after_retry_and_enforces_the_shared_cap() {
    let (dir, listener) = fixture();
    melchior::places::ensure(&dir.join("runtime")).expect("private runtime");
    std::fs::create_dir_all(dir.join("plugin")).expect("plugins");
    std::fs::write(dir.join("plugin/cap.lua"), "melchior.spend_cap_usd=0.3").expect("cap");
    let following = listener.try_clone().expect("retry listener");
    let remaining = listener.try_clone().expect("extra request observer");
    let worker = std::thread::spawn(move || {
        let first = response_server_sized(
            listener,
            200,
            completed(
                json!([]),
                json!({"input_tokens":1,"output_tokens":1,"cost_in_usd_ticks":1000000000_u64}),
            ),
            std::time::Duration::from_secs(5),
            64,
            || {},
        )
        .join()
        .expect("truncated attempt");
        let second = response_server_with_timeout(
            following,
            200,
            completed(
                json!([]),
                json!({"input_tokens":2,"output_tokens":3,"cost_in_usd_ticks":2000000000_u64}),
            ),
            std::time::Duration::from_secs(15),
        )
        .join()
        .expect("complete attempt");
        [first, second]
    });
    let env = [
        (KEY, "private-sentinel"),
        ("MAGI_MELCHIOR_SESSION", "xai-retry-run"),
    ];
    let output = events(&ask_request_with_timeout(
        &dir,
        &env,
        &ask("grok-4.7"),
        std::time::Duration::from_secs(25),
    ));
    let requests = worker.join().expect("fixture");
    for (headers, body) in &requests {
        request("grok-4.7", headers, body);
    }
    assert_eq!(requests[0].1, requests[1].1);
    assert!(
        !output.iter().any(|event| event["event"] == "failed"),
        "{output:?}"
    );
    assert_eq!(
        output
            .iter()
            .filter(|event| event["event"] == "retrying")
            .count(),
        1
    );
    let stops: Vec<_> = output
        .iter()
        .filter(|event| event["event"] == "stop")
        .collect();
    assert_eq!(stops.len(), 1);
    assert_eq!(stops[0]["reason"], "end_turn");
    assert_eq!(usage(&output)["input"], 3);
    assert_eq!(usage(&output)["output"], 4);
    assert_eq!(usage(&output)["cost_micros"], 300000);
    assert_eq!(usage(&output)["estimated_micros"], 0);
    let ledger = Ledger::at(&dir.join("runtime/melchior/.spent"), "xai-retry-run");
    assert_eq!(ledger.spent(), 300000);
    let next = events(&ask_request(&dir, &env, &ask("grok-4.7")));
    assert_eq!(next.len(), 1, "{next:?}");
    assert_eq!(next[0]["event"], "failed");
    assert!(
        next[0]["message"]
            .as_str()
            .expect("refusal")
            .contains("cap")
    );
    assert_eq!(ledger.spent(), 300000);
    unasked(&remaining);
}
