use super::*;
use melchior::mind::model::{Content, Message, ThinkingLevel, ToolStrictness};
use melchior::mind::wire::Schema;

#[path = "baseten/history.rs"]
mod history;

#[path = "baseten/failures.rs"]
mod failures;

const KEY: &str = "BASETEN_API_KEY";
const MODELS: &[&str] = &[
    "deepseek-ai/DeepSeek-V4-Flash-0731",
    "deepseek-ai/DeepSeek-V4-Pro-0813",
    "deepseek-ai/DeepSeek-V4.1-Flash",
    "deepseek-ai/DeepSeek-V4.1-Flash-1017",
    "deepseek-ai/DeepSeek-V4.1-Flash-1017-Fast",
    "deepseek-ai/DeepSeek-V4.1-Flash-Fast",
    "moonshotai/Kimi-K3",
    "moonshotai/Kimi-K3.1-Pro",
    "nvidia/NVIDIA-Nemotron-3-Ultra-550B-A55B",
    "openai/gpt-oss-120b",
    "zai-org/GLM-5.2",
    "zai-org/GLM-5.2-Fast",
    "zai-org/GLM-5.3",
    "zai-org/GLM-5.3-Fast",
    "zai-org/GLM-5.3-Flash",
];
const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

fn raw(model: &str) -> Value {
    let rows: Value = serde_json::from_str(include_str!(
        "../../../catalog/reference/pi-1.6.0/data/baseten.json"
    ))
    .expect("pinned source");
    rows["openai-completions"][format!("chat:{model}")].clone()
}

fn image_model(model: &str) -> bool {
    raw(model)["input"]
        .as_array()
        .expect("input")
        .contains(&json!("image"))
}

fn fixture() -> (Scratch, TcpListener) {
    let dir = Scratch::new("melchior-presets", "baseten");
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let url = format!("http://{}/v1", listener.local_addr().expect("address"));
    let mut declaration = shipped("baseten");
    assert_eq!(declaration["base_url"], "https://inference.baseten.co/v1");
    assert_eq!(declaration["auth"], json!({"kind":"api-key","vars":[KEY]}));
    assert_eq!(declaration["api"], "openai-completions");
    assert_eq!(declaration["discovery"], json!({"kind":"bundled"}));
    assert_eq!(
        declaration["catalog"],
        json!({"source":"pi-1.6.0","provider":"baseten",
        "models":MODELS.iter().map(|model| format!("chat:{model}")).collect::<Vec<_>>()})
    );
    declaration["base_url"] = json!(url);
    let overrides = declaration["models"].as_array_mut().expect("overrides");
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides[0]["id"], "moonshotai/Kimi-K3.1-Pro");
    for model in MODELS {
        if let Some(row) = overrides.iter_mut().find(|row| row["id"] == *model) {
            row["base_url"] = json!(url);
        } else {
            overrides.push(json!({"id":model,"base_url":url}));
        }
    }
    let encoded = serde_json::to_string(&declaration.to_string()).expect("Lua string");
    std::fs::write(
        dir.join("providers.lua"),
        format!(
            r#"
        for _, id in ipairs({{"openrouter", "ollama"}}) do
            melchior.provider(id, {{name=id,api="openai-completions",auth={{kind="none"}}}})
        end
        melchior.provider("baseten", melchior.json.decode_value({encoded}))
    "#
        ),
    )
    .expect("private overlay");
    (dir, listener)
}

fn ask(model: &str) -> Ask {
    let mut ask = ask_for("baseten", model);
    ask.wants.session_id = Some("private-sentinel-session".into());
    ask
}

fn finished() -> String {
    concat!(
        "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"reason α\"}}],\"usage\":null}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"answer\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":7,\"prompt_tokens_details\":{\"cached_tokens\":40}}}\n\n",
        "data: [DONE]\n\n"
    ).into()
}

fn affinity(headers: &str) -> String {
    let mut keys = ["session_id", "x-client-request-id", "x-session-affinity"].map(|name| {
        let prefix = format!("{name}: ");
        let values: Vec<_> = headers
            .lines()
            .filter_map(|line| line.strip_prefix(&prefix))
            .collect();
        assert_eq!(values.len(), 1, "{name}");
        assert_eq!(values[0].len(), 64);
        assert!(values[0].bytes().all(|byte| byte.is_ascii_hexdigit()));
        values[0].to_owned()
    });
    assert_eq!(keys[0], keys[1]);
    assert_eq!(keys[1], keys[2]);
    assert!(!headers.contains("x-session-id:"));
    std::mem::take(&mut keys[0])
}

fn request(model: &str, headers: &str, body: &Value) {
    assert!(headers.starts_with("post /v1/chat/completions "));
    assert!(headers.contains("authorization: bearer private-sentinel\r\n"));
    assert_eq!(headers.matches("authorization:").count(), 1);
    affinity(headers);
    assert!(!headers.contains("private-sentinel-session"));
    assert_eq!(body["model"], model);
    assert_eq!(body["max_tokens"], 128);
    assert_eq!(body["stream"], true);
    assert_eq!(body["stream_options"], json!({"include_usage":true}));
    for field in [
        "max_completion_tokens",
        "store",
        "tool_stream",
        "thinking",
        "prompt_cache_key",
        "prompt_cache_retention",
        "__thinking_enabled",
        "session_id",
    ] {
        assert!(body.get(field).is_none(), "{field}: {body}");
    }
}

fn controls(model: &str, level: Option<ThinkingLevel>, body: &Value) {
    let source = raw(model);
    let template = source["compat"]["thinkingFormat"] == "baseten";
    let effective = level.and_then(|level| source["thinkingLevelMap"][level.as_str()].as_str());
    let effort = effective.or_else(|| (template && level.is_none()).then_some("none"));
    assert_eq!(
        body.get("reasoning_effort").and_then(Value::as_str),
        effort,
        "{model}/{level:?}"
    );
    let args = (template && (level.is_none() || effective.is_some())).then(|| {
        json!({"enable_thinking":matches!(level, Some(ThinkingLevel::High | ThinkingLevel::Max))})
    });
    assert_eq!(
        body.get("chat_template_args"),
        args.as_ref(),
        "{model}/{level:?}"
    );
}

fn run(dir: &Scratch, request: &Ask) -> Vec<Value> {
    let output = ask_request(dir, &[(KEY, "private-sentinel")], request);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("private-sentinel"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("private-sentinel"));
    events(&output)
}

fn usage(model: &str, said: &[Value]) {
    let usage = &said.iter().rfind(|e| e["event"] == "spent").expect("usage")["usage"];
    assert_eq!(usage["input"], 60);
    assert_eq!(usage["cache_read"], 40);
    assert_eq!(usage["output"], 7);
    assert_eq!(usage["cost_micros"], 0);
    let source = raw(model);
    let cost = &source["cost"];
    let estimate = cost["input"].as_f64().expect("input") * 60.0
        + cost["output"].as_f64().expect("output") * 7.0
        + cost["cacheRead"].as_f64().expect("cache") * 40.0;
    let expected: Value =
        serde_json::from_str(&format!("{estimate:.0}")).expect("rounded estimate");
    assert_eq!(usage["estimated_micros"], expected);
}

#[test]
fn all_fifteen_models_list_offline_with_exact_metadata_and_no_invented_kimi_controls() {
    let (dir, listener) = fixture();
    for vars in [
        vec![],
        vec![(KEY, "private-sentinel")],
        vec![(KEY, " \t\n")],
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
        let reply: Value = serde_json::from_slice(&output.stdout).expect("JSON");
        assert_eq!(reply["ok"], true);
        let cards: Vec<_> = reply["result"]
            .as_array()
            .expect("cards")
            .iter()
            .filter(|card| card["provider"] == "baseten")
            .collect();
        assert_eq!(cards.len(), MODELS.len());
        for model in MODELS {
            let source = raw(model);
            let card = cards
                .iter()
                .find(|card| card["id"] == format!("baseten/{model}"))
                .expect("card");
            assert_eq!(card["ready"], vars.contains(&(KEY, "private-sentinel")));
            assert_eq!(card["setup"]["variables"], json!([[KEY]]));
            assert_eq!(card["context_window"], source["contextWindow"]);
            assert_eq!(card["max_output"], source["maxTokens"]);
            assert_eq!(card["capabilities"]["input"], source["input"]);
            assert_eq!(card["capabilities"]["strict_tools"], true);
            assert_eq!(card["capabilities"]["session_identity"], true);
            let levels: Vec<_> = ThinkingLevel::ALL
                .into_iter()
                .filter(|level| source["thinkingLevelMap"][level.as_str()].is_string())
                .map(ThinkingLevel::as_str)
                .collect();
            assert_eq!(
                card["capabilities"]["thinking_levels"],
                json!(levels),
                "{model}"
            );
            if image_model(model) {
                assert_eq!(
                    card["capabilities"]["input_limits"],
                    json!({"images":{"resize":{"max_width":2000,"max_height":2000,"max_bytes":4718592,"jpeg_quality":80}}})
                );
            } else {
                assert!(card["capabilities"].get("input_limits").is_none());
            }
        }
        unasked(&listener);
    }
}

#[test]
fn every_default_and_effort_preserves_templates_images_tools_cache_accounting_and_affinity() {
    let (dir, listener) = fixture();
    let mut keys = std::collections::BTreeSet::new();
    let mut total = 0;
    for model in MODELS {
        let mut identity = None;
        for level in ThinkingLevel::ALL.into_iter().map(Some).chain([None]) {
            let mut ask = ask(model);
            ask.wants.thinking = level;
            if level.is_none() {
                ask.context.tools.clear();
            }
            if image_model(model) {
                ask.context.messages[0].content.push(Content::Image {
                    media_type: "image/png".into(),
                    data: PNG.into(),
                });
            }
            let before = ask.context.clone();
            let server = response_server(listener.try_clone().expect("listener"), 200, finished());
            let said = run(&dir, &ask);
            stopped(&said, "end_turn");
            let (headers, body) = server.join().expect("request");
            request(model, &headers, &body);
            controls(model, level, &body);
            let key = affinity(&headers);
            if let Some(previous) = &identity {
                assert_eq!(&key, previous);
            }
            identity = Some(key);
            if image_model(model) {
                assert_eq!(
                    body["messages"][1]["content"][1]["image_url"]["url"],
                    format!("data:image/png;base64,{PNG}")
                );
            } else {
                assert_eq!(body["messages"][1]["content"], "hello");
            }
            if level.is_some() {
                assert_eq!(
                    body["tools"][0]["function"]["parameters"],
                    ask.context.tools[0].parameters
                );
                assert_eq!(body["tools"][0]["function"]["strict"], true);
            } else {
                assert!(body.get("tools").is_none());
            }
            usage(model, &said);
            assert_eq!(ask.context, before);
            total += 1;
        }
        assert!(keys.insert(identity.expect("identity")));
    }
    assert_eq!(total, 120);
}

#[test]
fn strict_policies_and_json_schema_answers_retain_exact_json_values() {
    let (dir, listener) = fixture();
    for model in MODELS {
        for policy in [
            None,
            Some(ToolStrictness::Prefer),
            Some(ToolStrictness::Require),
        ] {
            let mut ask = ask(model);
            ask.context.tools[0].strict = policy;
            let schema = json!({"type":"object","properties":{"value":{"enum":[null,[],{},u64::MAX]}},"required":[],"additionalProperties":false});
            ask.wants.schema = Some(Schema {
                name: "answer".into(),
                schema: schema.clone(),
            });
            let before = ask.context.clone();
            let server = response_server(listener.try_clone().expect("listener"), 200, finished());
            stopped(&run(&dir, &ask), "end_turn");
            let (headers, body) = server.join().expect("request");
            request(model, &headers, &body);
            assert_eq!(body["tools"][0]["function"]["strict"], policy.is_some());
            assert_eq!(
                body["response_format"],
                json!({"type":"json_schema","json_schema":{"name":"answer","schema":schema,"strict":true}})
            );
            assert_eq!(ask.context, before);
        }
    }
}
