use super::*;

#[test]
fn saved_mistral_api_key_reaches_native_model_request_without_environment_key() {
    let dir = Scratch::new("login-request", "mistral");
    let listener = TcpListener::bind("127.0.0.1:0").expect("local provider");
    config(
        &dir,
        &format!("http://{}/v1", listener.local_addr().expect("address")),
        r#"{kind="api-key",vars={"LOGIN_MISTRAL_FIXTURE_KEY"}}"#,
        "mistral-conversations",
    );
    let mut save = command(&dir)
        .args(["auth", "--stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("save process");
    writeln!(save.stdin.take().expect("stdin"), "{}", json!({"version":1,"event":"set_key","provider":"fixture","key":"private-login-mistral-key"})).expect("save request");
    let saved = save.wait_with_output().expect("saved");
    let reply: Value = serde_json::from_slice(&saved.stdout).expect("reply");
    assert!(reply["error"].is_null(), "{reply}");
    let serving = server(listener);
    let output = ask(&dir, &[]);
    let (headers, _) = serving.join().expect("provider request");
    assert!(
        headers
            .to_lowercase()
            .contains("authorization: bearer private-login-mistral-key")
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("private-login-mistral-key"));
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"text\":\"ok\""));
}
