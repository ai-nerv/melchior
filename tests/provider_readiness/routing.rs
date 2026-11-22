use super::*;

fn append(dir: &Scratch, declaration: &str) {
    std::fs::OpenOptions::new()
        .append(true)
        .open(dir.join("providers.lua"))
        .expect("config")
        .write_all(declaration.as_bytes())
        .expect("declaration");
}

fn listener() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    listener.set_nonblocking(true).expect("nonblocking");
    let url = format!("http://{}", listener.local_addr().expect("address"));
    (listener, url)
}

fn unasked(listener: &TcpListener) {
    assert_eq!(
        listener.accept().expect_err("no request expected").kind(),
        std::io::ErrorKind::WouldBlock
    );
}

fn stopped(out: std::process::Output) {
    assert!(out.status.success());
    assert!(
        String::from_utf8(out.stdout)
            .expect("UTF-8")
            .lines()
            .any(|line| {
                serde_json::from_str::<Value>(line).is_ok_and(|value| value["event"] == "stop")
            })
    );
}

fn refused(out: std::process::Output) {
    assert!(
        String::from_utf8(out.stdout)
            .expect("UTF-8")
            .lines()
            .any(|line| {
                serde_json::from_str::<Value>(line).is_ok_and(|value| {
                    value["event"] == "failed"
                        && value["message"]
                            .as_str()
                            .is_some_and(|text| text.contains("credential_origins"))
                })
            })
    );
}

#[test]
fn model_endpoint_and_header_precedence_are_used_by_real_inference() {
    let dir = Scratch::new("melchior-route", "override");
    let (listener, url) = listener();
    config(&dir, &url, r#"{kind="none"}"#, "openai-completions");
    append(
        &dir,
        &format!(
            r#"
        melchior.provider("fixture", {{name="Fixture",api="openai-completions",base_url="{url}/provider",
            auth={{kind="api-key",vars={{"FIXTURE_KEY"}}}},
            headers={{["X-Order"]="provider",["X-Provider"]="private-sentinel-metadata"}},
            models={{{{id="m",name="M",context_window=8192,max_tokens=128,base_url="{url}/model",
                headers={{["x-ORDER"]="model"}}}}}}}})
    "#
        ),
    );
    std::fs::write(dir.join("apis.lua"), r#"
        local api = melchior.apis["openai-completions"]
        local headers = api.headers
        melchior.api("openai-completions", {endpoint=api.endpoint,request=api.request,on_event=api.on_event,
            headers=function(key)
                local h = headers(key)
                h["x-order"] = "protocol"
                h["x-protocol"] = "fixture"
                return h
            end})
    "#).expect("protocol");
    assert_eq!(cards(&dir, &[])["setup"]["state"], "credential_missing");
    assert_eq!(
        cards(&dir, &[("FIXTURE_KEY", "private-sentinel-listing")])["ready"],
        true
    );
    let server = server(listener);
    stopped(ask(&dir, &[("FIXTURE_KEY", "private-sentinel-inference")]));
    let (headers, _) = server.join().expect("server");
    for expected in [
        "post /model/chat/completions ",
        "x-order: model\r\n",
        "x-provider: private-sentinel-metadata\r\n",
        "x-protocol: fixture\r\n",
        "authorization: bearer private-sentinel-inference\r\n",
    ] {
        assert!(headers.contains(expected), "missing {expected}");
    }
    assert_eq!(headers.matches("x-order:").count(), 1);
    assert!(!headers.contains("private-sentinel-listing"));
}

#[test]
fn changed_origins_are_rejected_before_refresh_and_require_explicit_binding() {
    let dir = Scratch::new("melchior-route", "binding");
    let (original, base) = listener();
    let (target, destination) = listener();
    let (refresh, token) = listener();
    config(&dir, &base, r#"{kind="none"}"#, "openai-completions");
    let declaration = |binding: &str| {
        format!(
            r#"
        melchior.provider("fixture", {{name="Fixture",api="openai-completions",base_url={base:?},
            credential_origins=melchior.json.array({{{binding}}}),
            auth={{kind="oauth",service="Fixture",token_url="{token}/token",client_id="public"}},
            models={{{{id="m",name="M",context_window=8192,max_tokens=128,base_url="{destination}/bound"}}}}}})
    "#
        )
    };
    append(&dir, &declaration(""));
    tokens(
        &dir,
        "private-sentinel-stale",
        Some("private-sentinel-refresh"),
        1,
    );
    let card = cards(&dir, &[]);
    assert_eq!(card["ready"], false);
    assert_eq!(card["setup"]["fields"], json!(["credential_origins"]));
    refused(ask(&dir, &[]));
    for listener in [&original, &target, &refresh] {
        unasked(listener);
    }
    append(&dir, &declaration(&format!("{destination:?}")));
    tokens(&dir, "private-sentinel-bound", None, u64::MAX);
    assert_eq!(cards(&dir, &[])["ready"], true);
    let server = server(target);
    stopped(ask(&dir, &[]));
    let (header, _) = server.join().expect("server");
    assert!(header.starts_with("post /bound/chat/completions "));
    assert!(header.contains("authorization: bearer private-sentinel-bound\r\n"));
    for listener in [&original, &refresh] {
        unasked(listener);
    }
    std::fs::write(
        dir.join("apis.lua"),
        format!(
            r#"
        local api = melchior.apis["openai-completions"]
        melchior.api("openai-completions", {{endpoint=function() return "{token}/escaped" end,
            headers=api.headers,request=api.request,on_event=api.on_event}})
    "#
        ),
    )
    .expect("changed protocol endpoint");
    refused(ask(&dir, &[]));
    unasked(&refresh);
}
