use super::*;

struct Server {
    root: String,
    active: Arc<AtomicUsize>,
    requests: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl Server {
    async fn new() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener");
        let root = format!("http://{}", listener.local_addr().expect("address"));
        let active = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(AtomicUsize::new(0));
        let (a, r) = (active.clone(), requests.clone());
        let task = tokio::spawn(async move {
            let mut tasks = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    connection = listener.accept() => {
                        let (mut socket, _) = connection.expect("accept");
                        let (a,r) = (a.clone(),r.clone());
                        tasks.spawn(async move {
                            a.fetch_add(1,Ordering::SeqCst);
                            let mut request = Vec::new();
                            while !request.ends_with(b"\r\n\r\n") {
                                match socket.read_u8().await {
                                    Ok(byte) => request.push(byte),
                                    Err(_) => { a.fetch_sub(1,Ordering::SeqCst); return; }
                                }
                            }
                            r.fetch_add(1,Ordering::SeqCst);
                            if request.starts_with(b"GET /ready ") || request.starts_with(b"GET /ollama ") {
                                let body = r#"{"data":[{"id":"available"},{"id":"second"}]}"#;
                                let _ = socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await;
                            } else {
                                let mut bytes = [0;4096];
                                while socket.read(&mut bytes).await.unwrap_or(0) != 0 {}
                            }
                            a.fetch_sub(1,Ordering::SeqCst);
                        });
                    },
                    _ = tasks.join_next(), if !tasks.is_empty() => {},
                }
            }
        });
        Self {
            root,
            active,
            requests,
            task,
        }
    }

    fn job(&self, index: usize, path: &str) -> Job {
        Job {
            index,
            url: Url::parse(&format!("{}{path}", self.root)).expect("url"),
            headers: HeaderMap::new(),
            path: None,
            popularity: false,
            ollama: (path == "/ollama").then(|| self.root.clone()),
        }
    }

    async fn closed(&self) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while self.active.load(Ordering::SeqCst) != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("sockets closed");
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[tokio::test]
async fn per_provider_timeout_releases_slots_and_preserves_completed_results() {
    let server = Server::new().await;
    let mut jobs: Vec<_> = (0..4).map(|index| server.job(index, "/stalled")).collect();
    jobs.push(server.job(4, "/ready"));
    let started = std::time::Instant::now();
    let result = run(jobs, Duration::from_millis(100), Duration::from_secs(2)).await;
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(server.requests.load(Ordering::SeqCst), 5);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, 4);
    server.closed().await;
}

#[tokio::test]
async fn overall_timeout_keeps_completed_catalogs_and_cancels_pending_ones() {
    let server = Server::new().await;
    let jobs = (0..40)
        .map(|index| server.job(index, if index == 0 { "/ready" } else { "/stalled" }))
        .collect();
    let result = run(jobs, Duration::from_secs(5), Duration::from_millis(250)).await;
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, 0);
    assert_eq!(server.requests.load(Ordering::SeqCst), CONCURRENCY + 1);
    server.closed().await;
}

#[tokio::test]
async fn cancelled_discovery_drops_active_http_requests() {
    let server = Server::new().await;
    let jobs = (0..40).map(|index| server.job(index, "/stalled")).collect();
    let worker = tokio::spawn(run(jobs, Duration::from_secs(5), Duration::from_secs(20)));
    tokio::time::timeout(Duration::from_secs(2), async {
        while server.requests.load(Ordering::SeqCst) != CONCURRENCY {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("active requests");
    worker.abort();
    assert!(worker.await.expect_err("cancelled").is_cancelled());
    server.closed().await;
}

#[tokio::test]
async fn timed_out_ollama_metadata_preserves_the_successful_model_listing() {
    let server = Server::new().await;
    let result = run(
        vec![server.job(0, "/ollama")],
        Duration::from_millis(200),
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(result.len(), 1);
    assert_eq!(
        result[0]
            .1
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>(),
        ["available", "second"]
    );
    assert_eq!(server.requests.load(Ordering::SeqCst), 2);
    server.closed().await;
}
