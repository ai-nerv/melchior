use super::*;

struct Stalled {
    url: String,
    accepted: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Stalled {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        listener.set_nonblocking(true).expect("nonblocking");
        let url = format!("http://{}", listener.local_addr().expect("address"));
        let stop = Arc::new(AtomicBool::new(false));
        let accepted = Arc::new(AtomicUsize::new(0));
        let (end, count) = (stop.clone(), accepted.clone());
        let thread = std::thread::spawn(move || {
            let mut clients = Vec::new();
            while !end.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((socket, _)) => {
                        count.fetch_add(1, Ordering::SeqCst);
                        clients.push(socket);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2))
                    }
                    Err(error) => panic!("accept: {error}"),
                }
            }
        });
        Self {
            url,
            accepted,
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for Stalled {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.take().expect("thread").join().expect("server");
    }
}

#[test]
fn forty_unavailable_providers_finish_inside_the_model_roles_deadline() {
    let server = Stalled::new();
    let dir = Scratch::new("melchior-discovery", "overall-deadline");
    std::fs::write(
        dir.join("providers.lua"),
        format!(
            r#"
        for _,id in ipairs({{"openrouter","ollama"}}) do
            melchior.provider(id,{{name=id,api="openai-completions",auth={{kind="none"}}}})
        end
        for i=1,40 do
            local id = "fixture-" .. i
            melchior.provider(id,{{name=id,api="openai-completions",base_url={:?},
                auth={{kind="none"}},discovery={{kind="compatible"}},
                models={{{{id="offline",name="Offline",context_window=8192,max_tokens=128}}}}}})
        end
    "#,
            server.url
        ),
    )
    .expect("config");
    let stdout = std::fs::File::create(dir.join("stdout.json")).expect("stdout");
    let stderr = std::fs::File::create(dir.join("stderr.txt")).expect("stderr");
    let mut child = Command::new(env!("CARGO_BIN_EXE_melchior"))
        .env_clear()
        .env("HOME", dir.as_os_str())
        .env("MELCHIOR_CONFIG", dir.as_os_str())
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env("XDG_CACHE_HOME", dir.join("cache"))
        .env("XDG_DATA_HOME", dir.join("data"))
        .env("XDG_RUNTIME_DIR", dir.join("runtime"))
        .args(["models", "--refresh", "--json"])
        .stdout(stdout)
        .stderr(stderr)
        .spawn()
        .expect("binary");
    let started = std::time::Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait") {
            break status;
        }
        if started.elapsed() > Duration::from_secs(25) {
            let _ = child.kill();
            let _ = child.wait();
            panic!("model discovery exceeded its overall deadline");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(status.success());
    assert!(started.elapsed() < Duration::from_secs(24));
    let reply: Value =
        serde_json::from_slice(&std::fs::read(dir.join("stdout.json")).expect("output"))
            .expect("reply");
    assert_eq!(reply["ok"], true);
    assert_eq!(reply["refreshed"], true);
    assert_eq!(reply["failed"].as_array().expect("failed").len(), 40);
    assert_eq!(
        reply["result"]
            .as_array()
            .expect("cards")
            .iter()
            .filter(|card| card["id"]
                .as_str()
                .is_some_and(|id| id.starts_with("fixture-") && id.ends_with("/offline")))
            .count(),
        40
    );
    let calls = server.accepted.load(Ordering::SeqCst);
    assert!((4..=16).contains(&calls), "calls={calls}");
}
