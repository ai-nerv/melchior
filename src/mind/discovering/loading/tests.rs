use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

mod bounds;
mod deadlines;

#[tokio::test]
async fn concurrent_discovery_obeys_an_overall_deadline_and_cancels_sockets() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let url = Url::parse(&format!(
        "http://{}/models",
        listener.local_addr().expect("address")
    ))
    .expect("url");
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let accepted = Arc::new(AtomicUsize::new(0));
    let (a, p, c) = (active.clone(), peak.clone(), accepted.clone());
    let server = tokio::spawn(async move {
        let mut clients = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                connection = listener.accept() => {
                    let (mut socket,_) = connection.expect("accept");
                    c.fetch_add(1,Ordering::SeqCst);
                    let (a,p) = (a.clone(),p.clone());
                    clients.spawn(async move {
                        p.fetch_max(a.fetch_add(1,Ordering::SeqCst)+1,Ordering::SeqCst);
                        let mut bytes = [0;4096];
                        while socket.read(&mut bytes).await.unwrap_or(0) != 0 {}
                        a.fetch_sub(1,Ordering::SeqCst);
                    });
                },
                _ = clients.join_next(), if !clients.is_empty() => {},
            }
        }
    });
    let jobs = (0..40)
        .map(|index| Job {
            index,
            url: url.clone(),
            headers: HeaderMap::new(),
            path: None,
            popularity: false,
            ollama: None,
        })
        .collect();
    let started = std::time::Instant::now();
    let results = run(jobs, Duration::from_secs(5), Duration::from_millis(250)).await;
    assert!(results.is_empty());
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(accepted.load(Ordering::SeqCst), CONCURRENCY);
    assert_eq!(peak.load(Ordering::SeqCst), CONCURRENCY);
    tokio::time::timeout(Duration::from_secs(2), async {
        while active.load(Ordering::SeqCst) != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("cancelled sockets");
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn valid_empty_list_succeeds_and_malformed_or_oversized_list_does_not() {
    for (body, expected) in [
        (serde_json::json!({"data":[]}), true),
        (serde_json::json!({"data":[{"id":""}]}), false),
        (serde_json::json!({"data":[{"id":"  "}]}), false),
        (serde_json::json!({"data":[{"id":"bad\nid"}]}), false),
        (serde_json::json!({"data":[{"id":"one"},{}]}), false),
        (
            serde_json::json!({"data":[{"id":"one"},{"id":"one"}]}),
            false,
        ),
        (
            serde_json::json!({"data":vec![serde_json::json!({"id":"x"});MODELS+1]}),
            false,
        ),
    ] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener");
        let url = Url::parse(&format!(
            "http://{}/models",
            listener.local_addr().expect("address")
        ))
        .expect("url");
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(socket.read_u8().await.expect("request"));
            }
            let body = body.to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("response");
        });
        let result = run(
            vec![Job {
                index: 0,
                url,
                headers: HeaderMap::new(),
                path: None,
                popularity: false,
                ollama: None,
            }],
            Duration::from_secs(2),
            Duration::from_secs(3),
        )
        .await;
        assert_eq!(!result.is_empty(), expected);
        server.await.expect("server");
    }
}

#[test]
fn cached_catalogs_reject_duplicate_empty_and_control_character_identities() {
    let model = super::super::parse(&serde_json::json!({"data":[{"id":"valid"}]}));
    let mut row = serde_json::to_value(&model[0]).expect("model");
    assert!(cached_models(&serde_json::json!([row.clone()]).to_string()).is_some());
    assert!(cached_models(&serde_json::json!([row.clone(), row.clone()]).to_string()).is_none());
    for id in ["", " ", "line\nbreak", "terminal\u{1b}"] {
        row["id"] = serde_json::json!(id);
        assert!(cached_models(&serde_json::json!([row.clone()]).to_string()).is_none());
    }
}
