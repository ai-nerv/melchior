use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub(super) enum Step {
    Bytes(Vec<u8>),
    Pause(Duration),
    Wait(Arc<tokio::sync::Notify>),
    Hold,
}

pub(super) struct Server {
    pub url: String,
    pub requests: Arc<AtomicUsize>,
    pub closed: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Server {
    pub async fn start(replies: Vec<Vec<Step>>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("local listener");
        let url = format!("http://{}", listener.local_addr().expect("address"));
        let requests = Arc::new(AtomicUsize::new(0));
        let closed = Arc::new(AtomicUsize::new(0));
        let (count, ended) = (Arc::clone(&requests), Arc::clone(&closed));
        let task = tokio::spawn(async move {
            let mut clients = tokio::task::JoinSet::new();
            for reply in replies {
                let (mut socket, _) = listener.accept().await.expect("accept");
                let (count, ended) = (Arc::clone(&count), Arc::clone(&ended));
                clients.spawn(async move {
                    let mut header = Vec::new();
                    while !header.ends_with(b"\r\n\r\n") {
                        if header.len() > 65536 {
                            panic!("request header too large");
                        }
                        let Ok(byte) = socket.read_u8().await else {
                            return;
                        };
                        header.push(byte);
                    }
                    let text = String::from_utf8(header).expect("request headers");
                    let size = text
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|n| n.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    let mut body = vec![0; size];
                    socket.read_exact(&mut body).await.expect("request body");
                    count.fetch_add(1, Ordering::SeqCst);
                    for step in reply {
                        match step {
                            Step::Bytes(bytes) => {
                                if socket.write_all(&bytes).await.is_err() {
                                    return;
                                }
                            }
                            Step::Pause(duration) => tokio::time::sleep(duration).await,
                            Step::Wait(ready) => ready.notified().await,
                            Step::Hold => {
                                let mut byte = [0];
                                loop {
                                    match socket.read(&mut byte).await {
                                        Ok(0) | Err(_) => {
                                            ended.fetch_add(1, Ordering::SeqCst);
                                            return;
                                        }
                                        Ok(_) => {}
                                    }
                                }
                            }
                        }
                    }
                });
            }
            while let Some(outcome) = clients.join_next().await {
                outcome.expect("server connection");
            }
        });
        Self {
            url,
            requests,
            closed,
            task,
        }
    }

    pub async fn wait_for(&self, count: &AtomicUsize, target: usize) {
        tokio::time::timeout(Duration::from_secs(3), async {
            while count.load(Ordering::SeqCst) < target {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("server barrier");
    }
}

pub(super) fn headers(status: u16) -> Step {
    Step::Bytes(format!("HTTP/1.1 {status} Fixture\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").into_bytes())
}

pub(super) fn chunk(bytes: &[u8]) -> Step {
    let mut body = format!("{:x}\r\n", bytes.len()).into_bytes();
    body.extend_from_slice(bytes);
    body.extend_from_slice(b"\r\n");
    Step::Bytes(body)
}

pub(super) fn reply(status: u16, bytes: &[u8]) -> Vec<Step> {
    vec![
        headers(status),
        chunk(bytes),
        Step::Bytes(b"0\r\n\r\n".to_vec()),
    ]
}
