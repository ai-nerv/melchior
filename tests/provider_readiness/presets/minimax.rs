use super::super::anthropic;
use super::*;
use melchior::mind::model::{Content, Image, Message, Role, ThinkingLevel, ToolStrictness};
use melchior::mind::wire::Schema;

#[path = "minimax/accounting.rs"]
mod accounting;

const REGIONS: [(&str, &str, &str); 2] = [
    (
        "minimax",
        "MINIMAX_API_KEY",
        "https://api.minimax.io/anthropic",
    ),
    (
        "minimax-cn",
        "MINIMAX_CN_API_KEY",
        "https://api.minimax.cn/anthropic",
    ),
];
const MODELS: [&str; 2] = ["MiniMax-M3", "MiniMax-M3.1-Flash"];
const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

fn fixture(id: &str, key: &str, endpoint: &str) -> (Scratch, TcpListener) {
    let dir = Scratch::new("melchior-presets", id);
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let url = format!(
        "http://{}/anthropic",
        listener.local_addr().expect("address")
    );
    let mut declaration = shipped(id);
    assert_eq!(declaration["base_url"], endpoint);
    assert_eq!(declaration["api"], "anthropic-messages");
    assert_eq!(declaration["auth"], json!({"kind":"api-key","vars":[key]}));
    assert_eq!(declaration["discovery"], json!({"kind":"bundled"}));
    assert_eq!(
        declaration["catalog"],
        json!({"source":"pi-1.6.0","provider":id,
        "models":MODELS.map(|model| format!("chat:{model}"))})
    );
    declaration["base_url"] = json!(url);
    for model in declaration["models"].as_array_mut().expect("overrides") {
        assert_eq!(model["base_url"], endpoint);
        model["base_url"] = json!(url);
    }
    let encoded = serde_json::to_string(&declaration.to_string()).expect("Lua string");
    std::fs::write(
        dir.join("providers.lua"),
        format!(
            r#"
        for _, id in ipairs({{"openrouter", "ollama"}}) do
            melchior.provider(id, {{name=id,api="openai-completions",auth={{kind="none"}}}})
        end
        melchior.provider({id:?}, melchior.json.decode_value({encoded}))
    "#
        ),
    )
    .expect("private overlay");
    (dir, listener)
}

fn ask(id: &str, model: &str) -> Ask {
    let mut ask = ask_for(id, model);
    ask.wants.max_tokens = Some(4096);
    ask.wants.thinking = Some(if model == MODELS[0] {
        ThinkingLevel::Minimal
    } else {
        ThinkingLevel::XHigh
    });
    ask.context.messages[0].content.push(Content::Image {
        data: PNG.into(),
        media_type: "image/png".into(),
    });
    ask
}

fn request(model: &str, headers: &str, body: &Value) {
    assert!(headers.starts_with("post /anthropic/v1/messages "));
    assert!(headers.contains("x-api-key: private-sentinel\r\n"));
    assert!(headers.contains("anthropic-version: 2023-06-01\r\n"));
    assert_eq!(headers.matches("x-api-key:").count(), 1);
    assert_eq!(body["model"], model);
    assert_eq!(body["stream"], true);
    assert_eq!(body["system"], "Use the echo tool.");
    assert_eq!(body["max_tokens"], 4096);
    assert_eq!(body["tools"][0]["name"], "echo");
    assert!(body["tools"][0].get("strict").is_none());
    assert_eq!(
        body["messages"][0]["content"][1],
        json!({"type":"image",
        "source":{"type":"base64","media_type":"image/png","data":PNG}})
    );
    for field in [
        "__scratch",
        "__budget",
        "tool_choice",
        "response_format",
        "stream_options",
        "temperature",
        "top_p",
    ] {
        assert!(body.get(field).is_none(), "{field}: {body}");
    }
    assert!(!body.to_string().contains("private-sentinel"));
    assert!(!body.to_string().contains("melchior.anthropic.v1:"));
}

#[test]
fn region_specific_keys_and_reviewed_binary_or_effort_controls_are_listed_offline() {
    for (id, key, endpoint) in REGIONS {
        let (dir, listener) = fixture(id, key, endpoint);
        let other = if id == "minimax" {
            "MINIMAX_CN_API_KEY"
        } else {
            "MINIMAX_API_KEY"
        };
        for (key, value, ready) in [
            (other, "private-sentinel", false),
            ("ANTHROPIC_API_KEY", "private-sentinel", false),
            (key, " \t\n", false),
            (key, "private-sentinel", true),
        ] {
            let output = command(&dir)
                .env(key, value)
                .args(["models", "--refresh", "--json"])
                .output()
                .expect("cards");
            assert!(output.status.success());
            assert!(!String::from_utf8_lossy(&output.stdout).contains("private-sentinel"));
            let reply: Value = serde_json::from_slice(&output.stdout).expect("reply");
            assert_eq!(reply["ok"], true, "{reply}");
            let cards: Vec<_> = reply["result"]
                .as_array()
                .expect("cards")
                .iter()
                .filter(|c| c["provider"] == id)
                .collect();
            assert_eq!(cards.len(), 2);
            for model in MODELS {
                let card = cards
                    .iter()
                    .find(|c| c["id"] == format!("{id}/{model}"))
                    .expect("card");
                assert_eq!(card["ready"], ready);
                assert_eq!(card["api"], "anthropic-messages");
                assert_eq!(card["context_window"], 1000000);
                assert_eq!(card["max_output"], 524288);
                assert_eq!(card["capabilities"]["strict_tools"], false);
                assert_eq!(card["capabilities"]["input"], json!(["text", "image"]));
                assert_eq!(
                    card["capabilities"]["thinking_levels"],
                    if model == MODELS[0] {
                        json!(["off", "minimal"])
                    } else {
                        json!(["low", "medium", "high", "xhigh", "max"])
                    }
                );
                assert_eq!(
                    card["capabilities"]["input_limits"]["images"]["resize"],
                    json!({"max_width":2000,"max_height":2000,"max_bytes":4718592,"jpeg_quality":80})
                );
            }
            unasked(&listener);
        }
        for model in MODELS {
            let said = events(&ask_request(&dir, &[], &ask(id, model)));
            assert_eq!(said.len(), 1, "{said:?}");
            assert_eq!(said[0]["why"], "auth");
            unasked(&listener);
        }
    }
}

#[test]
fn every_neutral_level_produces_only_supported_native_controls() {
    for (id, key, endpoint) in REGIONS {
        let (dir, listener) = fixture(id, key, endpoint);
        for model in MODELS {
            for level in std::iter::once(None).chain(ThinkingLevel::ALL.into_iter().map(Some)) {
                let mut ask = ask(id, model);
                ask.wants.thinking = level;
                let worker = response_server(
                    listener.try_clone().expect("listener"),
                    200,
                    anthropic::stream(&json!([{"type":"text","text":"hi"}]), "end_turn"),
                );
                let said = events(&ask_request(&dir, &[(key, "private-sentinel")], &ask));
                let (headers, body) = worker.join().expect("request");
                request(model, &headers, &body);
                stopped(&said, "end_turn");
                if model == MODELS[0] {
                    match level {
                        Some(ThinkingLevel::Off) => {
                            assert_eq!(body["thinking"], json!({"type":"disabled"}))
                        }
                        Some(ThinkingLevel::Minimal) => {
                            assert_eq!(body["thinking"], json!({"type":"adaptive"}))
                        }
                        _ => assert!(body.get("thinking").is_none()),
                    }
                    assert!(body.get("output_config").is_none());
                } else if let Some(level) =
                    level.filter(|l| !matches!(l, ThinkingLevel::Off | ThinkingLevel::Minimal))
                {
                    assert_eq!(body["thinking"], json!({"type":"adaptive"}));
                    assert_eq!(body["output_config"], json!({"effort":level.as_str()}));
                } else {
                    assert!(body.get("thinking").is_none());
                    assert!(body.get("output_config").is_none());
                }
            }
        }
    }
}

#[test]
fn interleaved_unsigned_thinking_tool_results_images_and_usage_survive_native_followup() {
    let blocks = json!([
        {"type":"thinking","thinking":"first "}, {"type":"text","text":"before "},
        {"type":"thinking","thinking":"second","signature":""},
        {"type":"tool_use","id":"echo-1","name":"echo","input":{"text":"hi","n":18446744073709551615u64}},
        {"type":"text","text":"after"}
    ]);
    for (id, key, endpoint) in REGIONS {
        let (dir, listener) = fixture(id, key, endpoint);
        for model in MODELS {
            let mut ask = ask(id, model);
            let worker = response_server(
                listener.try_clone().expect("listener"),
                200,
                anthropic::stream(&blocks, "tool_use"),
            );
            let said = events(&ask_request(&dir, &[(key, "private-sentinel")], &ask));
            let (headers, body) = worker.join().expect("request");
            request(model, &headers, &body);
            stopped(&said, "tool_use");
            let spent = &said
                .iter()
                .rev()
                .find(|e| e["event"] == "spent")
                .expect("spent")["usage"];
            assert_eq!(spent["input"], 60);
            assert_eq!(spent["cache_read"], 30);
            assert_eq!(spent["output"], 12);
            assert_eq!(
                spent["estimated_micros"],
                if model == MODELS[1] {
                    17
                } else if id == "minimax" {
                    34
                } else {
                    36
                }
            );
            ask.context.messages.push(anthropic::assistant(&said));
            ask.context.messages.push(Message {
                role: Role::Tool,
                content: vec![Content::ToolResult {
                    id: "echo-1".into(),
                    name: "echo".into(),
                    content: "ok".into(),
                    is_error: false,
                    images: vec![Image {
                        data: PNG.into(),
                        media_type: "image/png".into(),
                    }],
                }],
                stop_reason: None,
                usage: None,
                error: None,
            });
            let before = ask.context.clone();
            let worker = response_server(
                listener.try_clone().expect("listener"),
                200,
                anthropic::stream(&json!([{"type":"text","text":"done"}]), "end_turn"),
            );
            let said = events(&ask_request(&dir, &[(key, "private-sentinel")], &ask));
            let (headers, body) = worker.join().expect("followup");
            request(model, &headers, &body);
            stopped(&said, "end_turn");
            assert_eq!(body["messages"][1]["content"], blocks);
            let result = &body["messages"][2]["content"][0];
            assert_eq!(result["type"], "tool_result");
            assert_eq!(result["tool_use_id"], "echo-1");
            assert_eq!(result["content"][1]["source"]["data"], PNG);
            assert_eq!(ask.context, before);
        }
    }
}

#[test]
fn unsupported_forced_or_strict_tools_and_oversized_images_refuse_before_authentication() {
    for (id, key, endpoint) in REGIONS {
        let (dir, listener) = fixture(id, key, endpoint);
        for model in MODELS {
            for case in ["schema", "strict", "image"] {
                let mut ask = ask(id, model);
                match case {
                    "schema" => {
                        ask.wants.schema = Some(Schema {
                            name: "answer".into(),
                            schema: json!({"type":"object"}),
                        })
                    }
                    "strict" => ask.context.tools[0].strict = Some(ToolStrictness::Require),
                    _ => ask.context.messages[0].content.push(Content::Image {
                        data: "a".repeat(4718593),
                        media_type: "image/png".into(),
                    }),
                }
                let said = events(&ask_request(&dir, &[], &ask));
                assert_eq!(said.len(), 1, "{case}: {said:?}");
                assert_eq!(said[0]["why"], "invalid", "{case}: {said:?}");
                unasked(&listener);
            }
        }
    }
}

#[test]
fn authentication_and_invalid_requests_do_not_retry_or_publish_success() {
    for (id, key, endpoint) in REGIONS {
        let (dir, listener) = fixture(id, key, endpoint);
        for model in MODELS {
            for (status, why) in [(401, "auth"), (400, "invalid")] {
                let worker = response_server(listener.try_clone().expect("listener"),status,
                    json!({"type":"error","error":{"type":"invalid_request_error","message":"fixture refusal"}}).to_string());
                let said = events(&ask_request(
                    &dir,
                    &[(key, "private-sentinel")],
                    &ask(id, model),
                ));
                worker.join().expect("request");
                assert_eq!(
                    said.iter()
                        .filter(|e| e["event"] == "failed" && e["why"] == why)
                        .count(),
                    1,
                    "{said:?}"
                );
                assert!(
                    !said
                        .iter()
                        .any(|e| matches!(e["event"].as_str(), Some("stop" | "retrying")))
                );
                unasked(&listener);
            }
        }
    }
}
