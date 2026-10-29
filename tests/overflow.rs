use melchior::scratch::Scratch;
use serde_json::{Value, json};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Child(std::process::Child);

impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

async fn complete(mut child: Child) -> std::io::Result<std::process::ExitStatus> {
    loop {
        if let Some(status) = child.0.try_wait()? {
            return Ok(status);
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn exchange(
    listener: &tokio::net::TcpListener,
    status: u16,
    response: String,
) -> std::io::Result<Value> {
    let (mut socket, _) = listener.accept().await?;
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        if header.len() >= 65536 {
            return Err(std::io::Error::other("oversized fixture header"));
        }
        header.push(socket.read_u8().await?);
    }
    let header = String::from_utf8(header).map_err(std::io::Error::other)?;
    let length = header
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .and_then(|text| text.trim().parse::<usize>().ok())
        })
        .filter(|size| *size <= 65536)
        .ok_or_else(|| std::io::Error::other("fixture body length"))?;
    let mut body = vec![0; length];
    socket.read_exact(&mut body).await?;
    let response = format!(
        "HTTP/1.1 {status} Refused\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
        response.len()
    );
    socket.write_all(response.as_bytes()).await?;
    serde_json::from_slice(&body).map_err(std::io::Error::other)
}

#[tokio::test]
async fn real_binary_output_recovery_resets_partial_answers_and_retains_paid_usage()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = Scratch::new("mel-budget", "wire");
    std::fs::create_dir_all(dir.join("cfg"))?;
    std::fs::create_dir_all(dir.join("cache/melchior/models"))?;
    melchior::places::ensure(&dir.join("r"))?;
    for provider in ["openrouter", "ollama"] {
        std::fs::write(
            dir.join(format!("cache/melchior/models/{provider}.json")),
            "[]",
        )?;
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    std::fs::write(
        dir.join("cfg/providers.lua"),
        format!(
            r#"
melchior.provider("fixture", {{name="Fixture", api="openai-completions",
 base_url="http://{address}", auth={{kind="none"}},
 models={{{{id="one",name="One",context_window=8000,max_tokens=900}}}}}})
"#
        ),
    )?;
    let mut child = Child(
        std::process::Command::new(env!("CARGO_BIN_EXE_melchior"))
            .arg("ask")
            .env_clear()
            .env("HOME", &*dir)
            .env("TMPDIR", std::env::temp_dir())
            .env("XDG_RUNTIME_DIR", dir.join("r"))
            .env("XDG_CACHE_HOME", dir.join("cache"))
            .env("XDG_DATA_HOME", dir.join("data"))
            .env("MELCHIOR_CONFIG", dir.join("cfg"))
            .stdin(Stdio::piped())
            .stdout(std::fs::File::create(dir.join("stdout"))?)
            .stderr(std::fs::File::create(dir.join("stderr"))?)
            .spawn()?,
    );
    let ask = json!({"model":"fixture/one","context":{"messages":[
        {"role":"user","content":[{"type":"text","text":"exact input\r\n"}]}]},
        "wants":{"max_tokens":900,"max_tokens_automatic":true}});
    std::io::Write::write_all(
        &mut child.0.stdin.take().ok_or("stdin")?,
        ask.to_string().as_bytes(),
    )?;
    let partial = format!(
        "data: {}\n\n",
        json!({"choices":[{"delta":{"content":"discard me",
        "tool_calls":[{"index":0,"id":"discard-tool","function":{"name":"read","arguments":"{"}}]}}],
        "usage":{"prompt_tokens":9,"completion_tokens":3,"cost":0.000007}})
    );
    let rejection = json!({"error":{"type":"BadRequestError","code":400,"param":null,
        "message":"'max_tokens' or 'max_completion_tokens' is too large: 900. This model's maximum context length is 64 tokens and your request has 12 input tokens (900 > 64 - 12)."}}).to_string();
    let success = format!(
        "data: {}\n\ndata: [DONE]\n\n",
        json!({
        "choices":[{"delta":{"content":"recovered"},"finish_reason":"stop"}],
        "usage":{"prompt_tokens":12,"completion_tokens":2,"cost":0.000005}})
    );
    let (bodies, status) = tokio::time::timeout(Duration::from_secs(20), async {
        tokio::try_join!(
            async {
                let mut bodies = Vec::new();
                for (status, response) in [(200, partial), (400, rejection), (200, success)] {
                    bodies.push(exchange(&listener, status, response).await?);
                }
                Ok::<_, std::io::Error>(bodies)
            },
            complete(child)
        )
    })
    .await??;
    assert!(
        status.success(),
        "{}",
        std::fs::read_to_string(dir.join("stderr"))?
    );
    assert_eq!(
        bodies
            .iter()
            .map(|body| body["max_tokens"].as_u64().expect("output limit"))
            .collect::<Vec<_>>(),
        [900, 900, 52]
    );
    let mut normalized = bodies[2].clone();
    normalized["max_tokens"] = json!(900);
    assert_eq!(bodies[0], bodies[1]);
    assert_eq!(bodies[0], normalized);
    assert!(
        bodies
            .iter()
            .all(|body| body.get("__budget").is_none() && body.get("__scratch").is_none())
    );
    let events = std::fs::read_to_string(dir.join("stdout"))?
        .lines()
        .map(serde_json::from_str::<Value>)
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event"] == "retrying")
            .count(),
        2
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event"] == "stop")
            .count(),
        1
    );
    assert!(!events.iter().any(|event| event["event"] == "failed"));
    assert_eq!(events.last().ok_or("terminal")?["event"], "stop");
    let last_retry = events
        .iter()
        .rposition(|event| event["event"] == "retrying")
        .ok_or("retry")?;
    assert!(
        events[..last_retry]
            .iter()
            .any(|event| event.to_string().contains("discard me"))
    );
    assert!(
        !events[last_retry..]
            .iter()
            .any(|event| event.to_string().contains("discard"))
    );
    let usage = &events
        .iter()
        .rev()
        .find(|event| event["event"] == "spent")
        .ok_or("paid usage")?["usage"];
    assert_eq!(usage["input"], 21);
    assert_eq!(usage["output"], 5);
    assert_eq!(usage["cost_micros"], 12);
    Ok(())
}

#[tokio::test]
async fn real_binary_preserves_typed_input_unknown_and_auth_terminal_failures()
-> Result<(), Box<dyn std::error::Error>> {
    for (api, status, message, detail, refusal) in [
        (
            "anthropic-messages",
            400,
            "prompt is too long: 250000 tokens > 200000 maximum",
            Some(json!({"kind":"input"})),
            "overflow",
        ),
        (
            "anthropic-messages",
            400,
            "context length exceeded",
            Some(json!({"kind":"unknown"})),
            "overflow",
        ),
        (
            "anthropic-messages",
            401,
            "prompt is too long: 250000 tokens > 200000 maximum",
            None,
            "auth",
        ),
        (
            "openai-completions",
            400,
            "'max_tokens' or 'max_completion_tokens' is too large: 77. This model's maximum context length is 64 tokens and your request has 12 input tokens (77 > 64 - 12).",
            Some(
                json!({"kind":"output_budget","prompt_tokens":12,"limit_tokens":64,"requested_output_tokens":77}),
            ),
            "overflow",
        ),
        (
            "openai-completions",
            400,
            "'max_tokens' or 'max_completion_tokens' is too large: 77. This model's maximum context length is 64 tokens and your request has 64 input tokens (77 > 64 - 64).",
            Some(json!({"kind":"unknown"})),
            "overflow",
        ),
    ] {
        let dir = Scratch::new("mel-overflow", "wire");
        std::fs::create_dir_all(dir.join("cfg/plugin"))?;
        std::fs::create_dir_all(dir.join("cache/melchior/models"))?;
        melchior::places::ensure(&dir.join("r"))?;
        for provider in ["openrouter", "ollama"] {
            std::fs::write(
                dir.join(format!("cache/melchior/models/{provider}.json")),
                "[]",
            )?;
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        std::fs::write(
            dir.join("cfg/providers.lua"),
            format!(
                r#"
melchior.provider("fixture", {{name="Fixture", api="{api}",
 base_url="http://{address}", auth={{kind="none"}},
 models={{{{id="one",name="One",context_window=8000,max_tokens=900}}}}}})
"#
            ),
        )?;
        let mut child = Child(
            std::process::Command::new(env!("CARGO_BIN_EXE_melchior"))
                .arg("ask")
                .env_clear()
                .env("HOME", &*dir)
                .env("TMPDIR", std::env::temp_dir())
                .env("XDG_RUNTIME_DIR", dir.join("r"))
                .env("XDG_CACHE_HOME", dir.join("cache"))
                .env("XDG_DATA_HOME", dir.join("data"))
                .env("MELCHIOR_CONFIG", dir.join("cfg"))
                .stdin(Stdio::piped())
                .stdout(std::fs::File::create(dir.join("stdout"))?)
                .stderr(std::fs::File::create(dir.join("stderr"))?)
                .spawn()?,
        );
        let ask = json!({"model":"fixture/one","context":{"messages":[
            {"role":"user","content":[{"type":"text","text":"exact input\r\n"}]}]},
            "wants":{"max_tokens":77}});
        std::io::Write::write_all(
            &mut child.0.stdin.take().ok_or("stdin")?,
            ask.to_string().as_bytes(),
        )?;
        let response = if api == "openai-completions" {
            json!({"error":{"type":"BadRequestError","code":400,"param":null,"message":message}})
        } else {
            json!({"type":"error","error":{"type":"invalid_request_error","message":message}})
        }
        .to_string();
        let (body, output) = tokio::time::timeout(Duration::from_secs(15), async {
            tokio::try_join!(exchange(&listener, status, response), complete(child))
        })
        .await??;
        assert!(
            output.success(),
            "{}",
            std::fs::read_to_string(dir.join("stderr"))?
        );
        assert_eq!(body["max_tokens"], 77);
        let content = &body["messages"][0]["content"];
        if api == "openai-completions" {
            assert_eq!(content, "exact input\r\n");
        } else {
            assert_eq!(content[0]["text"], "exact input\r\n");
        }
        let lines: Vec<Value> = std::fs::read_to_string(dir.join("stdout"))?
            .lines()
            .map(serde_json::from_str)
            .collect::<Result<_, _>>()?;
        assert_eq!(
            lines
                .iter()
                .filter(|line| line["event"] == "failed")
                .count(),
            1
        );
        assert!(
            !lines
                .iter()
                .any(|line| line["event"] == "retrying" || line["event"] == "stop")
        );
        let terminal = lines.last().ok_or("terminal failure")?;
        assert_eq!(terminal["event"], "failed");
        assert_eq!(terminal["why"], refusal);
        match detail {
            Some(detail) => assert_eq!(terminal["overflow"], detail),
            None => assert!(terminal.get("overflow").is_none()),
        }
    }
    Ok(())
}
