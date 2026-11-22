use super::*;
use melchior::mind::model::ThinkingLevel;

#[test]
fn declared_controls_survive_model_roles_and_reach_the_http_request() {
    let dir = Scratch::new("melchior-thinking", "roles");
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let url = format!("http://{}", listener.local_addr().expect("address"));
    config(&dir, &url, r#"{kind="none"}"#, "openai-completions");
    std::fs::OpenOptions::new()
        .append(true)
        .open(dir.join("providers.lua"))
        .expect("config")
        .write_all(format!(r#"
        melchior.provider("fixture", {{name="Fixture",api="openai-completions",base_url={url:?},
            auth={{kind="none"}},compat={{supports_reasoning_effort=true}},
            models={{{{id="m",name="M",context_window=8192,max_tokens=128,reasoning=true,
                capabilities={{thinking={{"native-xhigh"}}}},
                thinking={{off=melchior.json.null,minimal=melchior.json.null,low="low",
                    medium=melchior.json.null,high=melchior.json.null,xhigh="xhigh",max=melchior.json.null}}
            }}}}}})
        "#).as_bytes()).expect("thinking config");
    let card = cards(&dir, &[]);
    assert_eq!(card["ready"], true);
    assert_eq!(card["capabilities"]["thinking"], json!(["native-xhigh"]));
    assert_eq!(
        card["capabilities"]["thinking_levels"],
        json!(["low", "xhigh"])
    );
    let reply = command(&dir)
        .args(["models", "--cbor"])
        .output()
        .expect("CBOR models");
    assert!(reply.status.success());
    let reply: Value = ciborium::from_reader(reply.stdout.as_slice()).expect("CBOR reply");
    assert_eq!(reply["ok"], true);
    assert!(reply["result"].as_array().expect("cards").contains(&card));
    let server = server(listener);
    let output = ask_with(
        &dir,
        &[],
        melchior::mind::wire::Wants {
            thinking: Some(ThinkingLevel::XHigh),
            ..Default::default()
        },
    );
    let (header, request) = server.join().expect("server");
    assert!(header.starts_with("post /chat/completions "));
    assert_eq!(request["reasoning_effort"], "xhigh");
    assert!(output.status.success());
    let events: Vec<Value> = String::from_utf8(output.stdout)
        .expect("events")
        .lines()
        .map(|line| serde_json::from_str(line).expect("event"))
        .collect();
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event"] == "stop")
            .count(),
        1
    );
    assert!(!events.iter().any(|event| event["event"] == "failed"));
}
