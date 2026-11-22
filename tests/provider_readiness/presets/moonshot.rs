use super::*;
use melchior::mind::model::{Content, Message, ThinkingLevel, ToolStrictness};
use melchior::mind::wire::Schema;
use std::path::Path;

#[path = "moonshot/history.rs"]
mod history;

const REGIONS: [(&str, &str, &str); 3] = [
    (
        "moonshotai",
        "MOONSHOT_API_KEY",
        "https://api.moonshot.ai/v1",
    ),
    (
        "moonshotai-cn",
        "MOONSHOT_API_KEY",
        "https://api.moonshot.cn/v1",
    ),
    (
        "kimi-coding",
        "KIMI_API_KEY",
        "https://api.kimi.com/coding/v1",
    ),
];
const MOONSHOT: &[&str] = &[
    "kimi-k2.6",
    "kimi-k2.7-code",
    "kimi-k2.7-code-highspeed",
    "kimi-k3",
    "kimi-k3.1-pro",
    "kimi-k3.1-pro-highspeed",
];
const CODING: &[&str] = &[
    "k3",
    "k3-256k",
    "k3.1-pro",
    "k3.1-pro-highspeed",
    "kimi-for-coding",
    "kimi-for-coding-highspeed",
];
const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

fn models(id: &str) -> &'static [&'static str] {
    if id == "kimi-coding" {
        CODING
    } else {
        MOONSHOT
    }
}

fn binary(model: &str) -> bool {
    model.starts_with("kimi-k2.") || model == "kimi-for-coding-highspeed"
}

fn fixture(id: &str, key: &str, url: &str) -> (Scratch, TcpListener) {
    let dir = Scratch::new("melchior-presets", id);
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let path = if id == "kimi-coding" {
        "/coding/v1"
    } else {
        "/v1"
    };
    let local = format!("http://{}{path}", listener.local_addr().expect("address"));
    let mut declaration = shipped(id);
    assert_eq!(declaration["base_url"], url);
    assert_eq!(declaration["auth"], json!({"kind":"api-key","vars":[key]}));
    assert_eq!(declaration["headers"], json!({"User-Agent":"melchior"}));
    assert_eq!(declaration["api"], "openai-completions");
    assert_eq!(declaration["discovery"], json!({"kind":"bundled"}));
    assert_eq!(
        declaration["catalog"],
        json!({"source":"pi-1.6.0","provider":id,"models":models(id).iter().map(|model| format!("chat:{model}")).collect::<Vec<_>>()})
    );
    declaration["base_url"] = json!(local);
    let overrides = declaration["models"].as_array_mut().expect("overrides");
    assert_eq!(overrides.len(), 6);
    for model in overrides {
        model["base_url"] = json!(local);
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

fn finished() -> String {
    concat!(
        "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"reason α\"}}],\"usage\":null}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"answer\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":7,\"prompt_tokens_details\":{\"cached_tokens\":40}}}\n\n",
        "data: [DONE]\n\n"
    ).into()
}

fn request(id: &str, model: &str, headers: &str, body: &Value) {
    let path = if id == "kimi-coding" {
        "/coding/v1"
    } else {
        "/v1"
    };
    assert!(headers.starts_with(&format!("post {path}/chat/completions ")));
    assert!(headers.contains("authorization: bearer private-sentinel\r\n"));
    assert_eq!(headers.matches("authorization:").count(), 1);
    assert!(headers.contains("user-agent: melchior\r\n"));
    assert_eq!(body["model"], model);
    assert_eq!(body["max_tokens"], 128);
    assert_eq!(body["stream"], true);
    assert_eq!(body["stream_options"], json!({"include_usage":true}));
    for field in [
        "max_completion_tokens",
        "store",
        "tool_stream",
        "tool_choice",
    ] {
        assert!(body.get(field).is_none(), "{field}: {body}");
    }
    if let Some(tools) = body["tools"].as_array() {
        for tool in tools {
            assert!(tool["function"].get("strict").is_none());
        }
    }
}

fn controls(model: &str, level: Option<ThinkingLevel>, body: &Value) {
    let thinking = if model == "kimi-k2.6" {
        Some(if level == Some(ThinkingLevel::Off) {
            json!({"type":"disabled"})
        } else {
            json!({"type":"enabled","keep":"all"})
        })
    } else if binary(model) && level == Some(ThinkingLevel::High) {
        Some(json!({"type":"enabled"}))
    } else {
        None
    };
    assert_eq!(
        body.get("thinking"),
        thinking.as_ref(),
        "{model}: {level:?}"
    );
    let effort = level.filter(|level| {
        !binary(model)
            && matches!(
                level,
                ThinkingLevel::Low | ThinkingLevel::High | ThinkingLevel::Max
            )
    });
    assert_eq!(
        body.get("reasoning_effort"),
        effort.map(|level| json!(level.as_str())).as_ref()
    );
}

fn usage(id: &str, model: &str, said: &[Value]) {
    let usage = &said.iter().rfind(|e| e["event"] == "spent").expect("usage")["usage"];
    assert_eq!(usage["input"], 60);
    assert_eq!(usage["cache_read"], 40);
    assert_eq!(usage["output"], 7);
    assert_eq!(usage["cost_micros"], 0);
    let raw: Value = serde_json::from_slice(
        &std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("catalog/reference/pi-1.6.0/data/{id}.json")),
        )
        .expect("source"),
    )
    .expect("JSON");
    let cost = &raw["openai-completions"][format!("chat:{model}")]["cost"];
    let estimate = cost["input"].as_f64().expect("input") * 60.0
        + cost["output"].as_f64().expect("output") * 7.0
        + cost["cacheRead"].as_f64().expect("cache") * 40.0;
    let expected: Value =
        serde_json::from_str(&format!("{estimate:.0}")).expect("rounded estimate");
    assert_eq!(usage["estimated_micros"], expected);
}

#[test]
fn eighteen_models_list_offline_with_payg_subscription_key_separation_and_binary_menus() {
    let mut total = 0;
    for (id, key, url) in REGIONS {
        let (dir, listener) = fixture(id, key, url);
        for vars in [
            vec![],
            vec![("MOONSHOT_API_KEY", "private-sentinel")],
            vec![("KIMI_API_KEY", "private-sentinel")],
            vec![(key, " \t\n")],
            vec![("OPENAI_API_KEY", "private-sentinel")],
        ] {
            let output = command(&dir)
                .envs(vars.clone())
                .args(["models", "--refresh", "--json"])
                .output()
                .expect("models");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(!String::from_utf8_lossy(&output.stdout).contains("private-sentinel"));
            let reply: Value = serde_json::from_slice(&output.stdout).expect("reply");
            assert_eq!(reply["ok"], true);
            let cards: Vec<_> = reply["result"]
                .as_array()
                .expect("cards")
                .iter()
                .filter(|card| card["provider"] == id)
                .collect();
            assert_eq!(cards.len(), 6);
            for model in models(id) {
                let card = cards
                    .iter()
                    .find(|card| card["id"] == format!("{id}/{model}"))
                    .expect("card");
                assert_eq!(card["ready"], vars.contains(&(key, "private-sentinel")));
                assert_eq!(card["setup"]["variables"], json!([[key]]));
                assert_eq!(card["capabilities"]["strict_tools"], false);
                assert_eq!(
                    card["capabilities"]["thinking_levels"],
                    if *model == "kimi-k2.6" {
                        json!(["off", "high"])
                    } else if binary(model) {
                        json!(["high"])
                    } else {
                        json!(["low", "high", "max"])
                    }
                );
                assert_eq!(card["capabilities"]["input"], json!(["text", "image"]));
                assert_eq!(
                    card["capabilities"]["input_limits"],
                    json!({"images":{"resize":{"max_width":2000,"max_height":2000,"max_bytes":4718592,"jpeg_quality":80}}})
                );
            }
            unasked(&listener);
        }
        for model in models(id) {
            let said = events(&ask_request(&dir, &[], &ask_for(id, model)));
            assert_eq!(said.len(), 1);
            assert_eq!(said[0]["why"], "auth");
            unasked(&listener);
            total += 1;
        }
    }
    assert_eq!(total, 18);
}

#[test]
fn every_effort_and_default_retains_exact_model_image_tool_schema_and_regional_accounting() {
    let mut total = 0;
    for (id, key, url) in REGIONS {
        let (dir, listener) = fixture(id, key, url);
        for model in models(id) {
            for level in ThinkingLevel::ALL.into_iter().map(Some).chain([None]) {
                let mut ask = ask_for(id, model);
                ask.wants.thinking = level;
                if level.is_none() {
                    ask.context.tools.clear();
                }
                ask.context.messages[0].content.push(Content::Image {
                    media_type: "image/png".into(),
                    data: PNG.into(),
                });
                let before = ask.context.clone();
                let server =
                    response_server(listener.try_clone().expect("listener"), 200, finished());
                let said = events(&ask_request(&dir, &[(key, "private-sentinel")], &ask));
                stopped(&said, "end_turn");
                let (headers, body) = server.join().expect("request");
                request(id, model, &headers, &body);
                controls(model, level, &body);
                assert_eq!(
                    body["messages"][1]["content"][1]["image_url"]["url"],
                    format!("data:image/png;base64,{PNG}")
                );
                if level.is_some() {
                    assert_eq!(
                        body["tools"][0]["function"]["parameters"],
                        ask.context.tools[0].parameters
                    );
                } else {
                    assert!(body.get("tools").is_none());
                }
                usage(id, model, &said);
                assert_eq!(ask.context, before);
                total += 1;
            }
        }
    }
    assert_eq!(total, 144);
}

#[test]
fn required_strict_tools_and_unknown_models_refuse_without_credentials_or_network() {
    for (id, _, url) in REGIONS {
        let key = if id == "kimi-coding" {
            "KIMI_API_KEY"
        } else {
            "MOONSHOT_API_KEY"
        };
        let (dir, listener) = fixture(id, key, url);
        for model in models(id) {
            let mut ask = ask_for(id, model);
            ask.context.tools[0].strict = Some(ToolStrictness::Require);
            let said = events(&ask_request(&dir, &[], &ask));
            assert_eq!(said.len(), 1);
            assert_eq!(said[0]["why"], "invalid");
            unasked(&listener);
        }
        let said = events(&ask_request(
            &dir,
            &[(key, "private-sentinel")],
            &ask_for(id, "not-a-model"),
        ));
        assert_eq!(said.len(), 1);
        assert_eq!(said[0]["event"], "failed");
        unasked(&listener);
    }
}

#[test]
fn json_schema_answers_preserve_empty_containers_nulls_and_large_integers() {
    for (id, key, url) in REGIONS {
        let (dir, listener) = fixture(id, key, url);
        for model in models(id) {
            let mut ask = ask_for(id, model);
            let schema = json!({"type":"object","properties":{"value":{"enum":[null,[],{},u64::MAX]}},"required":[],"additionalProperties":false});
            ask.wants.schema = Some(Schema {
                name: "answer".into(),
                schema: schema.clone(),
            });
            let server = response_server(listener.try_clone().expect("listener"), 200, finished());
            let said = events(&ask_request(&dir, &[(key, "private-sentinel")], &ask));
            stopped(&said, "end_turn");
            let (headers, body) = server.join().expect("request");
            request(id, model, &headers, &body);
            assert_eq!(
                body["response_format"],
                json!({"type":"json_schema","json_schema":{"name":"answer","schema":schema,"strict":true}})
            );
        }
    }
}
