use super::*;
use melchior::mind::model::{Content, Message};
use melchior::mind::wire::Ask;

fn fixture(
    api: &str,
    provider: Option<bool>,
    model: Option<bool>,
    developer: bool,
    oauth: bool,
) -> (Scratch, TcpListener, Ask) {
    let dir = Scratch::new("melchior-ready", "instructions");
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let url = format!("http://{}", listener.local_addr().expect("address"));
    let auth = if oauth {
        format!(
            r#"{{kind="oauth",service="Fixture",authorize_url="{url}/authorize",token_url="{url}/token",client_id="public"}}"#
        )
    } else {
        r#"{kind="api-key",vars={"FIXTURE_KEY"}}"#.into()
    };
    config(&dir, &url, &auth, api);
    let flag = |value: Option<bool>| {
        value.map_or(String::new(), |value| {
            format!("supports_mid_convo_system_messages={value},")
        })
    };
    std::fs::OpenOptions::new()
        .append(true)
        .open(dir.join("providers.lua"))
        .expect("config")
        .write_all(
            format!(
                r#"
        melchior.provider("fixture", {{name="Fixture",api={api:?},base_url={url:?},auth={auth},
            compat={{{}supports_developer_role={developer}}},
            models={{{{id="m",name="Model",context_window=8192,max_tokens=128,compat={{{}}}}}}}}})
    "#,
                flag(provider),
                flag(model)
            )
            .as_bytes(),
        )
        .expect("overlay");
    let ask = serde_json::from_value(json!({"model":"fixture/m","context":{
        "system":"base", "messages":[
            {"role":"system","content":[{"type":"text","text":"leading"}]},
            {"role":"user","content":[{"type":"text","text":"first"}]},
            {"role":"assistant","content":[{"type":"text","text":"answer"},
                {"type":"tool_call","id":"call","name":"echo","arguments":{"v":[null,{},[],18446744073709551615u64]}}]},
            {"role":"tool","content":[{"type":"tool_result","id":"call","name":"echo","content":"result","is_error":false}]},
            {"role":"system","content":[{"type":"text","text":"late α"},{"type":"text","text":"\nsecond"}]},
            {"role":"user","content":[{"type":"text","text":"later"}]}
        ],"tools":[{"name":"echo","description":"echo","parameters":{"type":"object"}}]
    }})).expect("ask");
    (dir, listener, ask)
}

fn check(body: &Value, native: bool, developer: bool) {
    let messages = body["messages"].as_array().expect("messages");
    let role = if developer { "developer" } else { "system" };
    let (leading, assistant) = if native {
        assert_eq!(
            messages
                .iter()
                .map(|m| m["role"].as_str().expect("role"))
                .collect::<Vec<_>>(),
            vec![role, role, "user", "assistant", "tool", role, "user"]
        );
        assert_eq!(messages[0]["content"], "base");
        assert_eq!(messages[1]["content"], "leading");
        assert_eq!(messages[5]["content"], "late α\nsecond");
        (2, 3)
    } else {
        assert_eq!(
            messages
                .iter()
                .map(|m| m["role"].as_str().expect("role"))
                .collect::<Vec<_>>(),
            vec![role, "user", "assistant", "tool", "user"]
        );
        assert_eq!(messages[0]["content"], "base\n\nleading\n\nlate α\nsecond");
        (1, 2)
    };
    assert_eq!(messages[leading]["content"], "first");
    assert_eq!(messages.last().expect("last")["content"], "later");
    assert_eq!(
        messages[assistant]["tool_calls"][0]["function"]["arguments"],
        r#"{"v":[null,{},[],18446744073709551615]}"#
    );
    assert_eq!(
        messages[assistant + 1],
        json!({"role":"tool","tool_call_id":"call","content":"result"})
    );
}

#[test]
fn native_chat_instruction_positions_and_false_model_overrides_preserve_tool_history() {
    for (provider, model, native) in [
        (None, None, false),
        (Some(true), None, true),
        (Some(true), Some(false), false),
        (Some(false), Some(true), true),
    ] {
        for developer in [false, true] {
            let (dir, listener, ask) =
                fixture("openai-completions", provider, model, developer, false);
            let saved = ask.context.clone();
            let worker = server(listener);
            let events = presets::events(&ask_request(
                &dir,
                &[("FIXTURE_KEY", "private-sentinel")],
                &ask,
            ));
            presets::stopped(&events, "end_turn");
            let (headers, body) = worker.join().expect("fixture");
            assert!(headers.starts_with("post /chat/completions "));
            assert!(headers.contains("authorization: bearer private-sentinel\r\n"));
            check(&body, native, developer);
            assert_eq!(ask.context, saved);
        }
    }
}

#[test]
fn invalid_instruction_content_and_unimplemented_positional_apis_refuse_before_refresh() {
    for api in [
        "openai-completions",
        "openai-responses",
        "anthropic-messages",
    ] {
        let (dir, listener, mut ask) = fixture(api, Some(true), None, false, true);
        tokens(&dir, "old-access", Some("private-sentinel-refresh"), 1);
        if api == "openai-completions" {
            ask.context.messages[0].content.push(Content::Image {
                media_type: "image/png".into(),
                data: "aaaa".into(),
            });
        }
        let said = presets::events(&ask_request(&dir, &[], &ask));
        assert_eq!(said.len(), 1, "{said:?}");
        assert_eq!(said[0]["event"], "failed");
        assert_eq!(said[0]["why"], "invalid");
        assert!(!said[0].to_string().contains("private-sentinel"));
        assert_eq!(
            listener
                .accept()
                .expect_err("no token or inference request")
                .kind(),
            std::io::ErrorKind::WouldBlock
        );
        let store =
            Store::load_from(&dir.join("data/magi/credentials.json")).expect("private store");
        assert_eq!(
            store.get("fixture").expect("unchanged credentials").access,
            "old-access"
        );
    }
}

#[test]
fn retry_and_later_user_turn_keep_instruction_placement_without_mutating_the_ask() {
    for native in [false, true] {
        let (dir, listener, mut ask) =
            fixture("openai-completions", Some(native), None, false, false);
        let next = listener.try_clone().expect("listener");
        let worker = std::thread::spawn(move || {
            let first = response_server(listener, 503, r#"{"error":{"message":"retry"}}"#.into())
                .join()
                .expect("first");
            let second = response_server_with_timeout(next,200,
                "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n".into(),
                std::time::Duration::from_secs(20)).join().expect("second");
            (first, second)
        });
        let saved = ask.context.clone();
        let said = presets::events(&ask_request_with_timeout(
            &dir,
            &[("FIXTURE_KEY", "private-sentinel")],
            &ask,
            std::time::Duration::from_secs(30),
        ));
        assert_eq!(said.iter().filter(|e| e["event"] == "retrying").count(), 1);
        assert!(!said.iter().any(|e| e["event"] == "failed"));
        let stops: Vec<_> = said.iter().filter(|e| e["event"] == "stop").collect();
        assert_eq!(stops.len(), 1);
        assert_eq!(stops[0]["reason"], "end_turn");
        let (first, second) = worker.join().expect("fixture");
        assert_eq!(first.1, second.1);
        check(&second.1, native, false);
        assert_eq!(ask.context, saved);
        let later = TcpListener::bind("127.0.0.1:0").expect("later listener");
        let url = format!("http://{}", later.local_addr().expect("address"));
        std::fs::OpenOptions::new().append(true).open(dir.join("providers.lua")).expect("config")
            .write_all(format!(r#"melchior.provider("fixture", {{name="Fixture",api="openai-completions",base_url={url:?},auth={{kind="none"}},compat={{supports_mid_convo_system_messages={native}}},models={{{{id="m",name="M",context_window=8192,max_tokens=128}}}}}})"#).as_bytes()).expect("local config");
        ask.context.messages.extend([
            Message::assistant("ok"),
            Message::system("additional"),
            Message::user("next"),
        ]);
        let saved = ask.context.clone();
        let worker = server(later);
        presets::stopped(&presets::events(&ask_request(&dir, &[], &ask)), "end_turn");
        let body = worker.join().expect("later request").1;
        let messages = body["messages"].as_array().expect("messages");
        assert_eq!(messages.last().expect("last")["content"], "next");
        if native {
            assert_eq!(
                messages[messages.len() - 2],
                json!({"role":"system","content":"additional"})
            );
        } else {
            assert_eq!(
                messages[0]["content"],
                "base\n\nleading\n\nlate α\nsecond\n\nadditional"
            );
        }
        assert_eq!(ask.context, saved);
    }
}
