use super::*;
use melchior::mind::model::{Content, Context, Message, Role, Tool, ToolStrictness};
use melchior::mind::wire::Ask;

fn tool(name: &str) -> Tool {
    Tool {
        name: name.into(),
        description: format!("Use {name}"),
        parameters: json!({"type":"object","properties":{"value":{"type":"string"}},"required":["value"],"additionalProperties":false}),
        strict: None,
    }
}

fn configure(dir: &Scratch, url: &str, compat: Value, model: Value, oauth: bool) {
    let auth = if oauth {
        json!({"kind":"oauth","service":"Fixture","authorize_url":format!("{url}/authorize"),"token_url":format!("{url}/token"),"client_id":"public"})
    } else {
        json!({"kind":"api-key","vars":["FIXTURE_KEY"]})
    };
    let declaration = json!({"name":"Fixture","api":"openai-completions","base_url":url,"auth":auth,"compat":compat,
        "models":[{"id":"m","name":"M","context_window":8192,"max_tokens":128,"compat":model}]});
    let encoded = serde_json::to_string(&declaration.to_string()).expect("encode");
    std::fs::write(
        dir.join("providers.lua"),
        format!(
            r#"for _, id in ipairs({{"openrouter", "ollama"}}) do
            melchior.provider(id, {{name=id,api="openai-completions",auth={{kind="none"}}}})
        end
        melchior.provider("fixture", melchior.json.decode_value({encoded}))"#
        ),
    )
    .expect("private config");
}

fn flags() -> Value {
    json!({"supports_mid_convo_system_messages":true,"supports_mid_convo_tool_additions":true})
}

fn fixture(compat: Value, model: Value, oauth: bool) -> (Scratch, TcpListener, Ask) {
    let dir = Scratch::new("melchior-ready", "tool-history");
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let url = format!("http://{}/v1", listener.local_addr().expect("address"));
    configure(&dir, &url, compat, model, oauth);
    let mut change = Message::tool_changes(vec![tool("b")], vec![]);
    change.content.push(Content::Text {
        text: "update α".into(),
        signature: None,
    });
    let ask = Ask {
        model: "fixture/m".into(),
        about: String::new(),
        wants: Default::default(),
        context: Context {
            system: Some("base".into()),
            tools: vec![tool("a")],
            messages: vec![Message::user("first"), change, Message::user("next")],
        },
    };
    (dir, listener, ask)
}

fn names(value: &Value) -> Vec<&str> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .map(|tool| tool["function"]["name"].as_str().expect("name"))
        .collect()
}

fn additions(body: &Value) -> Vec<&Value> {
    body["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .filter(|m| m.get("tools").is_some())
        .collect()
}

fn check(body: &Value, native: bool, positional: bool) {
    assert_eq!(
        names(&body["tools"]),
        if native { vec!["a"] } else { vec!["a", "b"] }
    );
    let inline = additions(body);
    assert_eq!(inline.len(), usize::from(native));
    if native {
        assert_eq!(inline[0]["role"], "system");
        assert!(inline[0].get("content").is_none());
        assert_eq!(names(&inline[0]["tools"]), ["b"]);
        assert_eq!(body["messages"][1]["content"], "first");
        assert_eq!(body["messages"][3]["content"], "update α");
    }
    assert_eq!(
        body["messages"][0]["content"],
        if positional {
            "base"
        } else {
            "base\n\nupdate α"
        }
    );
    assert!(!body.to_string().contains("tool_changes"));
}

#[test]
fn native_positions_require_both_flags_and_respect_explicit_model_overrides() {
    for (provider, model, native, positional) in [
        (json!({}), json!({}), false, false),
        (flags(), json!({}), true, true),
        (
            flags(),
            json!({"supports_mid_convo_tool_additions":false}),
            false,
            true,
        ),
        (
            flags(),
            json!({"supports_mid_convo_system_messages":false}),
            false,
            false,
        ),
        (json!({}), flags(), true, true),
    ] {
        let (dir, listener, ask) = fixture(provider, model, false);
        let saved = ask.context.clone();
        let worker = server(listener);
        presets::stopped(
            &presets::events(&ask_request(
                &dir,
                &[("FIXTURE_KEY", "private-sentinel")],
                &ask,
            )),
            "end_turn",
        );
        let (headers, body) = worker.join().expect("fixture");
        assert!(headers.starts_with("post /v1/chat/completions "));
        assert!(headers.contains("authorization: bearer private-sentinel\r\n"));
        check(&body, native, positional);
        assert_eq!(ask.context, saved);
    }
}

#[test]
fn removals_and_redefinitions_fold_only_the_prefix_and_keep_past_tool_calls_lossless() {
    for removed in [false, true] {
        let (dir, listener, mut ask) = fixture(flags(), json!({}), false);
        let mut changed = tool("a");
        changed.description = "replacement".into();
        let mut later = tool("c");
        later.parameters = json!({"type":"object","properties":{"state":{"const":{"array":[],"object":{},"null":null,"integer":u64::MAX}}}});
        ask.context.messages.extend([
            serde_json::from_value(json!({"role":"assistant","content":[{"type":"tool_call","id":"b1","name":"b","arguments":{"values":[null,[],{},18446744073709551615u64]}}]})).expect("assistant"),
            serde_json::from_value(json!({"role":"tool","content":[{"type":"tool_result","id":"b1","name":"b","content":"result","is_error":false}]})).expect("tool result"),
            Message::tool_changes(vec![changed], if removed { vec!["b".into()] } else { vec![] }),
            Message::user("after change"),
            Message::tool_changes(vec![later.clone()], vec![]),
        ]);
        let saved = ask.context.clone();
        let worker = server(listener);
        presets::stopped(
            &presets::events(&ask_request(
                &dir,
                &[("FIXTURE_KEY", "private-sentinel")],
                &ask,
            )),
            "end_turn",
        );
        let body = worker.join().expect("fixture").1;
        assert_eq!(
            names(&body["tools"]),
            if removed { vec!["a"] } else { vec!["a", "b"] }
        );
        assert_eq!(body["tools"][0]["function"]["description"], "replacement");
        let inline = additions(&body);
        assert_eq!(inline.len(), 1);
        assert_eq!(names(&inline[0]["tools"]), ["c"]);
        assert_eq!(
            inline[0]["tools"][0]["function"]["parameters"],
            later.parameters
        );
        let messages = body["messages"].as_array().expect("messages");
        let call = messages
            .iter()
            .find(|m| m["role"] == "assistant")
            .expect("call");
        assert_eq!(call["tool_calls"][0]["function"]["name"], "b");
        assert_eq!(
            call["tool_calls"][0]["function"]["arguments"],
            r#"{"values":[null,[],{},18446744073709551615]}"#
        );
        assert!(messages.contains(&json!({"role":"tool","tool_call_id":"b1","content":"result"})));
        assert_eq!(ask.context, saved);
    }
}

#[test]
fn retries_and_later_processes_resend_anchored_declarations_without_mutating_history() {
    let (dir, listener, mut ask) = fixture(flags(), json!({}), false);
    let retry = listener.try_clone().expect("retry");
    let later = listener.try_clone().expect("later");
    let worker = std::thread::spawn(move || {
        let first = response_server(listener, 503, r#"{"error":{"message":"retry"}}"#.into())
            .join()
            .expect("first");
        let second = response_server_with_timeout(retry,200,"data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n".into(),std::time::Duration::from_secs(20)).join().expect("second");
        (first, second)
    });
    let saved = ask.context.clone();
    let events = presets::events(&ask_request_with_timeout(
        &dir,
        &[("FIXTURE_KEY", "private-sentinel")],
        &ask,
        std::time::Duration::from_secs(30),
    ));
    assert_eq!(
        events.iter().filter(|e| e["event"] == "retrying").count(),
        1
    );
    assert!(!events.iter().any(|e| e["event"] == "failed"));
    assert_eq!(
        events
            .iter()
            .filter(|e| e["event"] == "stop" && e["reason"] == "end_turn")
            .count(),
        1
    );
    let (first, second) = worker.join().expect("fixture");
    assert_eq!(first.1, second.1);
    check(&second.1, true, true);
    assert_eq!(ask.context, saved);
    ask.context
        .messages
        .extend([Message::assistant("ok"), Message::user("later")]);
    let worker = server(later);
    presets::stopped(
        &presets::events(&ask_request(
            &dir,
            &[("FIXTURE_KEY", "private-sentinel")],
            &ask,
        )),
        "end_turn",
    );
    let body = worker.join().expect("later request").1;
    check(&body, true, true);
    assert_eq!(
        body["messages"]
            .as_array()
            .expect("messages")
            .last()
            .expect("last")["content"],
        "later"
    );
}

#[test]
fn malformed_and_oversized_histories_fail_before_oauth_refresh_or_inference() {
    for case in 0..7 {
        let (dir, listener, mut ask) = fixture(flags(), json!({}), true);
        tokens(&dir, "old-access", Some("private-sentinel-refresh"), 1);
        match case {
            0 => ask.context.messages[1].role = Role::User,
            1 => ask.context.messages[1].content.push(Content::ToolChanges {
                added: vec![],
                removed: vec![],
            }),
            2 => ask
                .context
                .messages
                .push(Message::tool_changes(vec![tool(&"x".repeat(257))], vec![])),
            3 => ask
                .context
                .messages
                .push(Message::tool_changes(vec![], vec!["bad\nname".into()])),
            4 => ask.context.tools = (0..4096).map(|i| tool(&format!("t{i}"))).collect(),
            5 => ask
                .context
                .messages
                .push(Message::tool_changes(vec![], vec!["absent".into(); 16384])),
            _ => {
                let mut huge = tool("huge");
                huge.description = "x".repeat(8 * 1024 * 1024);
                ask.context
                    .messages
                    .push(Message::tool_changes(vec![huge], vec![]));
            }
        }
        let events = presets::events(&ask_request(&dir, &[], &ask));
        assert_eq!(events.len(), 1, "case {case}: {events:?}");
        assert_eq!(events[0]["event"], "failed", "case {case}: {events:?}");
        assert_eq!(events[0]["why"], "invalid", "case {case}: {events:?}");
        presets::unasked(&listener);
        let store = Store::load_from(&dir.join("data/magi/credentials.json")).expect("store");
        assert_eq!(
            store.get("fixture").expect("credentials").access,
            "old-access"
        );
    }
}

#[test]
fn strict_inline_tools_are_negotiated_before_credentials_and_participate_in_routing() {
    for supported in [false, true] {
        for deepseek in [false, true] {
            let mut compat = flags();
            compat["supports_strict_mode"] = json!(supported);
            if deepseek {
                compat["chat_strict_dialect"] = json!("deepseek");
            }
            let (dir, listener, mut ask) = fixture(compat, json!({}), !supported);
            ask.context.tools.clear();
            if let Content::ToolChanges { added, .. } = &mut ask.context.messages[1].content[0] {
                added[0].strict = Some(ToolStrictness::Require);
            }
            if supported {
                let worker = server(listener);
                presets::stopped(
                    &presets::events(&ask_request(
                        &dir,
                        &[("FIXTURE_KEY", "private-sentinel")],
                        &ask,
                    )),
                    "end_turn",
                );
                let (headers, body) = worker.join().expect("fixture");
                assert!(headers.starts_with(if deepseek {
                    "post /beta/chat/completions "
                } else {
                    "post /v1/chat/completions "
                }));
                assert!(body.get("tools").is_none());
                assert_eq!(additions(&body)[0]["tools"][0]["function"]["strict"], true);
            } else {
                tokens(&dir, "old-access", Some("private-sentinel-refresh"), 1);
                let events = presets::events(&ask_request(&dir, &[], &ask));
                assert_eq!(events.len(), 1);
                assert_eq!(events[0]["event"], "failed");
                assert_eq!(events[0]["why"], "invalid");
                presets::unasked(&listener);
                let store =
                    Store::load_from(&dir.join("data/magi/credentials.json")).expect("store");
                assert_eq!(
                    store.get("fixture").expect("credentials").access,
                    "old-access"
                );
            }
        }
    }
}
