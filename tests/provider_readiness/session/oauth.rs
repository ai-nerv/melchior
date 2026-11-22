use super::*;

fn refresh_server(
    listener: TcpListener,
    access: &str,
    refresh: Option<&str>,
) -> std::thread::JoinHandle<String> {
    let body = json!({"access_token":access,"refresh_token":refresh,"expires_in":3600}).to_string();
    std::thread::spawn(move || {
        listener.set_nonblocking(true).expect("nonblocking");
        let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut socket = loop {
            match listener.accept() {
                Ok((socket, _)) => break socket,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(std::time::Instant::now() < until, "no refresh request");
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Err(error) => panic!("accept: {error}"),
            }
        };
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .expect("deadline");
        let mut header = Vec::new();
        while !header.ends_with(b"\r\n\r\n") {
            assert!(header.len() < 8192);
            let mut byte = [0];
            socket.read_exact(&mut byte).expect("header");
            header.push(byte[0]);
        }
        let header = String::from_utf8(header)
            .expect("header")
            .to_ascii_lowercase();
        assert!(header.starts_with("post /token "));
        let length: usize = header
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .expect("length")
            .trim()
            .parse()
            .expect("number");
        assert!(length < 8192);
        let mut request = vec![0; length];
        socket.read_exact(&mut request).expect("form");
        write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).expect("response");
        String::from_utf8(request).expect("form")
    })
}

#[test]
fn access_and_refresh_rotation_keep_affinity_but_relogin_and_logout_do_not() {
    let dir = Scratch::new("melchior-ready", "session-oauth");
    let inference = TcpListener::bind("127.0.0.1:0").expect("inference");
    let token = TcpListener::bind("127.0.0.1:0").expect("token");
    token.set_nonblocking(true).expect("nonblocking");
    fixture(
        &dir,
        &inference,
        json!({"kind":"oauth","service":"Fixture","token_url":format!("http://{}/token",token.local_addr().expect("address")),"client_id":"public"}),
        json!({"supports_prompt_cache_key":true,"send_session_affinity_headers":true}),
    );
    tokens(
        &dir,
        "private-sentinel-initial",
        Some("private-sentinel-refresh-a"),
        u64::MAX,
    );
    let ask = request();
    let initial = key(&round(&dir, &inference, &[], &ask).1);
    let path = dir.join("data/magi/credentials.json");
    let mut binding = None;
    for (access, expected, next) in [
        (
            "private-sentinel-next-a",
            "private-sentinel-refresh-a",
            Some("private-sentinel-refresh-b"),
        ),
        (
            "private-sentinel-next-b",
            "private-sentinel-refresh-b",
            None,
        ),
    ] {
        let mut store = Store::load_from(&path).expect("load");
        store
            .providers
            .get_mut("fixture")
            .expect("tokens")
            .expires_at = 1;
        store.save_to(&path).expect("expire");
        let worker = refresh_server(token.try_clone().expect("listener"), access, next);
        let (headers, body) = round(&dir, &inference, &[], &ask);
        assert_eq!(key(&body), initial);
        assert!(headers.contains(&format!("x-session-affinity: {initial}\r\n")));
        assert!(headers.contains(&format!("authorization: bearer {access}\r\n")));
        let form = worker.join().expect("refresh");
        assert!(form.contains(&format!("refresh_token={expected}")));
        let stored = Store::load_from(&path).expect("renewed");
        let tokens = stored.get("fixture").expect("tokens");
        assert_eq!(tokens.access, access);
        assert_eq!(
            tokens.refresh.as_deref(),
            Some("private-sentinel-refresh-b")
        );
        assert_eq!(tokens.binding.as_ref().expect("lineage").len(), 64);
        if let Some(binding) = &binding {
            assert_eq!(&tokens.binding, binding);
        }
        binding = Some(tokens.binding.clone());
    }
    tokens(&dir, "private-sentinel-new-account", None, u64::MAX);
    assert_ne!(key(&round(&dir, &inference, &[], &ask).1), initial);
    let mut store = Store::load_from(&path).expect("load");
    assert!(store.forget("fixture"));
    store.save_to(&path).expect("logout");
    let result = events(&ask_request(&dir, &[], &ask));
    assert_eq!(result.len(), 1);
    assert_eq!(result[0]["why"], "auth");
    unasked(&inference);
    unasked(&token);
}
