#![allow(clippy::expect_used, clippy::indexing_slicing)]

use melchior::{
    events::*,
    framing,
    mind::{
        running,
        spending::Ledger,
        wire::{Ask, Said},
    },
};
use serde_json::{Value, json};
use std::{
    process::{Command, Stdio},
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

#[allow(dead_code)]
#[path = "../src/mind/provider/testing.rs"]
mod fixture;
use fixture::{Server, Step, chunk, headers, reply};

fn measured(input: u64, output: u64, cost: u64) -> String {
    format!(
        "data: {}\n\n",
        json!({"usage":{"prompt_tokens":input,"completion_tokens":output,"prompt_cache_hit_tokens":0,"cost":cost as f64 / 1_000_000.0}})
    )
}

fn success() -> Vec<Step> {
    reply(
        200,
        format!(
            "{}{}{}data: [DONE]\n\n",
            measured(10, 1, 11),
            measured(10, 2, 17),
            measured(10, 2, 17)
        )
        .as_bytes(),
    )
}

fn ask() -> Ask {
    serde_json::from_value(
        json!({"model":"private-provider-canary/private-model-canary","context":{"messages":[]}}),
    )
    .expect("ask")
}

fn drain(receiver: &Receiver) -> Vec<Value> {
    let mut events = Vec::new();
    while let Ok(bytes) = receiver.try_recv() {
        let text = std::str::from_utf8(&bytes).expect("UTF8");
        for canary in [
            "private-provider-canary",
            "private-model-canary",
            "private-input-canary",
            "private-error-canary",
            "private-run-canary",
            "127.0.0.1",
        ] {
            assert!(!text.contains(canary), "event leaked {canary}");
        }
        decode(&bytes).expect("strict schema");
        events.push(serde_json::from_slice(&bytes).expect("JSON"));
    }
    events
}

async fn run() -> Vec<Said> {
    let mut said = Vec::new();
    tokio::time::timeout(
        Duration::from_secs(40),
        running::run(&ask(), |item| said.push(item)),
    )
    .await
    .expect("bounded native ask");
    assert!(matches!(said.last(), Some(Said::Stop { .. })), "{said:?}");
    said
}

#[tokio::test]
async fn worker() {
    let Ok(scenario) = std::env::var("EVENT_USAGE_WORKER") else {
        return;
    };
    let paid = measured(10, 1, 11);
    let broken = [paid.as_bytes(), b"data: \xff\n\n"].concat();
    let second = format!("{}data: [DONE]\n\n", measured(5, 3, 7));
    let replies = match scenario.as_str() {
        "retry" | "wait_cancel" => vec![reply(200, &broken), reply(200, second.as_bytes())],
        "missing_retry" => vec![
            reply(503, b"private-error-canary"),
            reply(200, second.as_bytes()),
        ],
        "cancel" => vec![vec![headers(200), chunk(paid.as_bytes()), Step::Hold]],
        "missing_cancel" => vec![vec![
            headers(200),
            chunk(b"data: {\"choices\":[{\"delta\":{\"content\":\"private-input-canary\"}}]}\n\n"),
            Step::Hold,
        ]],
        "estimate" | "zero" => {
            let input = if scenario == "zero" { 0 } else { 10 };
            let output = if scenario == "zero" { 0 } else { 2 };
            vec![reply(200, format!("event: message_start\ndata: {}\n\nevent: message_delta\ndata: {}\n\n", json!({"message":{"usage":{"input_tokens":input,"output_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}), json!({"usage":{"output_tokens":output},"delta":{"stop_reason":"end_turn"}})).as_bytes())]
        }
        "full" => vec![success(), success()],
        _ => vec![success()],
    };
    let server = Server::start(replies).await;
    let config = std::path::PathBuf::from(std::env::var_os("MELCHIOR_CONFIG").expect("config"));
    let api = if matches!(scenario.as_str(), "estimate" | "zero") {
        "anthropic-messages"
    } else {
        "openai-completions"
    };
    std::fs::write(config.join("providers.lua"), format!(r#"melchior.provider("private-provider-canary", {{ name="Synthetic", api="{api}", base_url="{}", auth={{kind="none"}}, models={{{{id="private-model-canary",name="Synthetic",context_window=8192,max_tokens=128,cost={{input=3,output=5}}}}}} }})"#, server.url)).expect("owned provider");
    let ledger = Ledger::at(&Ledger::kept(), "private-run-canary");
    let receiver = enable_local().expect("explicit isolated activation");
    if scenario == "full" {
        for _ in 0..QUEUED_EVENTS {
            assert_eq!(
                emit(|| (
                    Correlation::default(),
                    Fact::Retry {
                        outcome: RetryOutcome::Scheduled
                    }
                )),
                Emitted::Queued
            );
        }
        run().await;
        assert_eq!(ledger.spent(), 17);
        assert!(loss().full > 0);
        for _ in 0..QUEUED_EVENTS {
            receiver.try_recv().expect("filler");
        }
    }
    if matches!(
        scenario.as_str(),
        "cancel" | "missing_cancel" | "wait_cancel"
    ) {
        let asked = ask();
        let ready = tokio::sync::Notify::new();
        let mut future = Box::pin(running::run(&asked, |said| {
            if matches!(
                (&*scenario, said),
                ("cancel", Said::Spent { .. })
                    | ("missing_cancel", Said::Text { .. })
                    | ("wait_cancel", Said::Retrying { .. })
            ) {
                ready.notify_one();
            }
        }));
        tokio::select! {
            () = &mut future => panic!("unexpected completion"),
            () = async { tokio::time::timeout(Duration::from_secs(10), ready.notified()).await.expect("native cancellation barrier"); } => {}
        }
        drop(future);
        if scenario != "wait_cancel" {
            server.wait_for(&server.closed, 1).await;
        }
        assert_eq!(server.requests.load(Ordering::SeqCst), 1);
    } else if scenario == "recover" {
        let mut blocked = false;
        let mut removed = false;
        let mut stopped = false;
        tokio::time::timeout(
            Duration::from_secs(10),
            running::run(&ask(), |said| {
                if matches!(said, Said::RequestFrame { .. }) && !blocked {
                    std::fs::create_dir_all(Ledger::kept().parent().expect("parent"))
                        .expect("runtime parent");
                    std::fs::write(Ledger::kept(), b"owned failure fixture").expect("block ledger");
                    blocked = true;
                }
                if matches!(said, Said::Spent { .. }) && !removed {
                    std::fs::remove_file(Ledger::kept()).expect("remove owned blocker");
                    removed = true;
                }
                if matches!(said, Said::Stop { .. }) {
                    stopped = true;
                }
            }),
        )
        .await
        .expect("recovery ask");
        assert!(blocked && removed && stopped);
    } else {
        run().await;
    }
    let events = drain(&receiver);
    let snapshots: Vec<_> = events
        .iter()
        .filter(|event| event["fact"]["kind"] == "usage_snapshot")
        .collect();
    assert!(snapshots.len() >= 2, "{events:?}");
    let id = &snapshots[0]["correlation"]["ask_id"];
    assert!(id.is_string());
    assert!(
        events
            .iter()
            .all(|event| &event["correlation"]["ask_id"] == id)
    );
    for (index, event) in snapshots.iter().enumerate() {
        assert_eq!(event["fact"]["revision"], (index + 1) as u64);
        assert!(event["correlation"]["attempt_id"].is_null());
    }
    let last = &snapshots.last().expect("last")["fact"];
    assert_eq!(
        last["ledger"],
        if scenario == "wait_cancel" {
            "unchanged"
        } else {
            "unknown"
        }
    );
    match scenario.as_str() {
        "retry" => {
            assert_eq!(last["input_tokens"], 15);
            assert_eq!(last["output_tokens"], 4);
            assert_eq!(last["reported_cost_micros"], 18);
            assert_eq!(last["estimated_cost_micros"], 0);
            assert_eq!(ledger.spent(), 18);
            let first_known = snapshots
                .iter()
                .position(|s| s["fact"]["input_tokens"] == 10)
                .expect("first attempt");
            assert!(
                snapshots
                    .iter()
                    .skip(first_known + 1)
                    .any(|s| s["fact"]["input_tokens"].is_null())
            );
        }
        "missing_retry" | "missing_cancel" => {
            for key in [
                "input_tokens",
                "output_tokens",
                "reported_cost_micros",
                "estimated_cost_micros",
            ] {
                assert!(last[key].is_null(), "{key}: {last}");
            }
            assert_eq!(
                ledger.spent(),
                if scenario == "missing_retry" { 7 } else { 0 }
            );
        }
        "cancel" | "wait_cancel" => {
            assert_eq!(last["input_tokens"], 10);
            assert_eq!(last["reported_cost_micros"], 11);
            assert_eq!(ledger.spent(), 11);
        }
        "estimate" | "zero" => {
            assert!(last["reported_cost_micros"].is_null());
            assert_eq!(
                last["estimated_cost_micros"],
                if scenario == "estimate" { 40 } else { 0 }
            );
            assert_eq!(ledger.spent(), if scenario == "estimate" { 40 } else { 0 });
            if scenario == "zero" {
                assert!(!Ledger::kept().exists());
            }
        }
        _ => {
            assert_eq!(last["input_tokens"], 10);
            assert_eq!(last["output_tokens"], 2);
            assert_eq!(last["reported_cost_micros"], 17);
            assert!(last["cache_write_tokens"].is_null());
            assert_eq!(
                ledger.spent(),
                if scenario == "untracked" {
                    0
                } else if scenario == "full" {
                    34
                } else {
                    17
                }
            );
        }
    }
    let statuses: Vec<_> = snapshots
        .iter()
        .map(|s| s["fact"]["ledger"].as_str().expect("ledger"))
        .collect();
    if scenario == "recover" {
        assert!(statuses.contains(&"failed"));
        assert!(statuses.contains(&"persisted"));
    }
    if scenario == "untracked" {
        assert!(statuses.contains(&"untracked"));
        assert!(!statuses.contains(&"persisted"));
    }
    if scenario == "normal" {
        assert!(statuses.contains(&"persisted"));
        assert!(statuses.contains(&"unchanged"));
    }
}

#[test]
fn native_usage_snapshots_follow_accounting_and_cancellation() {
    for scenario in [
        "normal",
        "retry",
        "missing_retry",
        "cancel",
        "missing_cancel",
        "wait_cancel",
        "recover",
        "untracked",
        "estimate",
        "zero",
        "full",
    ] {
        let root = melchior::scratch::Scratch::new("event-usage", scenario);
        std::fs::create_dir_all(root.join("cfg")).expect("config");
        std::fs::create_dir_all(root.join("cache/melchior/models")).expect("cache");
        melchior::places::ensure(&root.join("r")).expect("runtime");
        for provider in ["openrouter", "ollama"] {
            std::fs::write(
                root.join(format!("cache/melchior/models/{provider}.json")),
                "[]",
            )
            .expect("offline cache");
        }
        let mut command = Command::new(std::env::current_exe().expect("test executable"));
        command
            .args(["--exact", "worker", "--nocapture"])
            .env_clear()
            .env("HOME", &*root)
            .env("TMPDIR", std::env::temp_dir())
            .env("XDG_RUNTIME_DIR", root.join("r"))
            .env("XDG_CACHE_HOME", root.join("cache"))
            .env("XDG_DATA_HOME", root.join("data"))
            .env("MELCHIOR_CONFIG", root.join("cfg"))
            .env("EVENT_USAGE_WORKER", scenario)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        if scenario != "untracked" {
            command.env("MAGI_MELCHIOR_SESSION", "private-run-canary");
        }
        let mut child = command.spawn().expect("isolated worker");
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if let Some(status) = child.try_wait().expect("worker status") {
                assert!(status.success(), "scenario {scenario}: {status}");
                break;
            }
            if Instant::now() >= deadline {
                child.kill().expect("kill timed out worker");
                child.wait().expect("reap worker");
                panic!("scenario {scenario} exceeded deadline");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
