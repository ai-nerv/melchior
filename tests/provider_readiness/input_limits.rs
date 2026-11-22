use super::*;
use melchior::mind::{
    model::{Content, Context, Message},
    wire::Ask,
};

fn declaration(dir: &Scratch, url: &str, auth: &str, limits: &str) {
    config(dir, url, auth, "openai-completions");
    std::fs::OpenOptions::new().append(true).open(dir.join("providers.lua")).expect("config")
        .write_all(format!(r#"
        melchior.provider("fixture", {{name="Fixture",api="openai-completions",base_url={url:?},
            auth={auth}, discovery={{kind="bundled"}},
            models={{{{id="m",name="Model",input={{"text","image"}},context_window=8192,max_tokens=128,
                input_limits={limits}}}}}}})
        "#).as_bytes()).expect("limits declaration");
}

fn ask() -> Ask {
    let mut message = Message::user("private-sentinel-message");
    for _ in 0..2 {
        message.content.push(Content::Image {
            data: "YWJj".into(),
            media_type: "image/png".into(),
        });
    }
    Ask {
        model: "fixture/m".into(),
        context: Context {
            messages: vec![message],
            ..Default::default()
        },
        wants: Default::default(),
        about: String::new(),
    }
}

#[test]
fn model_cards_publish_all_enforced_limits_without_network_or_credentials() {
    let dir = Scratch::new("melchior-limits", "cards");
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let url = format!("http://{}", listener.local_addr().expect("address"));
    declaration(
        &dir,
        &url,
        "{kind='api-key',vars={'FIXTURE_KEY'}}",
        "{max_request_bytes=33554432,images={max_per_message=20,max_per_request=100,resize={max_width=2000,max_height=1500,max_bytes=4718592,jpeg_quality=80}}}",
    );
    let card = cards(&dir, &[]);
    assert_eq!(card["ready"], false);
    assert_eq!(
        card["capabilities"]["input_limits"],
        json!({"max_request_bytes":33554432,"images":{
        "max_per_message":20,"max_per_request":100,"resize":{"max_width":2000,"max_height":1500,"max_bytes":4718592,"jpeg_quality":80}}})
    );
    let output = command(&dir)
        .args(["models", "--cbor"])
        .output()
        .expect("CBOR listing");
    assert!(output.status.success());
    let reply: Value = ciborium::from_reader(output.stdout.as_slice()).expect("CBOR reply");
    assert_eq!(reply["ok"], true);
    assert!(reply["result"].as_array().expect("cards").contains(&card));
    assert_eq!(
        listener.accept().expect_err("listing is offline").kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[test]
fn contradictory_advertised_limits_refuse_instead_of_disagreeing_with_enforcement() {
    let dir = Scratch::new("melchior-limits", "contradictory");
    config(
        &dir,
        "http://127.0.0.1:1",
        "{kind='none'}",
        "openai-completions",
    );
    std::fs::OpenOptions::new().append(true).open(dir.join("providers.lua")).expect("config")
        .write_all(br#"
        melchior.provider("fixture", {name="Fixture",api="openai-completions",auth={kind="none"},
            models={{id="m",name="M",context_window=8192,max_tokens=128,
                input_limits={max_request_bytes=1024},capabilities={input_limits={max_request_bytes=2048}}}}})
        "#).expect("declaration");
    let output = ask_request(&dir, &[], &ask());
    let events: Vec<Value> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).expect("event"))
        .collect();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["why"], "invalid");
    assert!(
        events[0]["message"]
            .as_str()
            .expect("message")
            .contains("capabilities.input_limits")
    );
}

#[test]
fn limits_refuse_before_oauth_refresh_network_and_credential_mutation() {
    for limits in [
        "{max_request_bytes=1}",
        "{images={max_per_message=1}}",
        "{images={max_per_request=1}}",
        "{images={resize={max_bytes=3}}}",
    ] {
        let dir = Scratch::new("melchior-limits", "oauth");
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        listener.set_nonblocking(true).expect("nonblocking");
        let url = format!("http://{}", listener.local_addr().expect("address"));
        let auth = format!(
            r#"{{kind="oauth",service="Fixture",authorize_url="{url}/authorize",token_url="{url}/token",client_id="fixture"}}"#
        );
        declaration(&dir, &url, &auth, limits);
        tokens(
            &dir,
            "private-sentinel-access",
            Some("private-sentinel-refresh"),
            1,
        );
        let path = dir.join("data/magi/credentials.json");
        let before = std::fs::read(&path).expect("credentials");
        let output = ask_request(&dir, &[], &ask());
        assert!(output.status.success());
        let events: Vec<Value> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|line| serde_json::from_str(line).expect("event"))
            .collect();
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0]["event"], "failed");
        assert_eq!(events[0]["why"], "invalid");
        assert!(
            events[0]["message"]
                .as_str()
                .expect("message")
                .contains("limit")
        );
        assert!(!String::from_utf8_lossy(&output.stdout).contains("private-sentinel"));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("private-sentinel"));
        assert_eq!(
            listener
                .accept()
                .expect_err("no refresh or inference")
                .kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert_eq!(std::fs::read(path).expect("credentials"), before);
    }
}

#[test]
fn invalid_limit_declarations_fail_with_field_diagnostics_without_echoing_values() {
    for limits in [
        "{max_request_bytes=0}",
        "{images={max_per_message=-1}}",
        "{images={resize={jpeg_quality=101}}}",
        "{images={resize={max_bytes='private-sentinel'}}}",
        "{private_sentinel=1}",
    ] {
        let dir = Scratch::new("melchior-limits", "invalid");
        declaration(&dir, "http://127.0.0.1:1", "{kind='none'}", limits);
        let output = ask_request(&dir, &[], &ask());
        assert!(output.status.success());
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(!text.contains("private-sentinel"));
        assert!(!text.contains("private_sentinel"));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("private-sentinel"));
        let events: Vec<Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).expect("event"))
            .collect();
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0]["event"], "failed");
        assert_eq!(events[0]["why"], "invalid");
        assert!(
            events[0]["message"]
                .as_str()
                .expect("message")
                .contains("input_limits")
        );
    }
}
