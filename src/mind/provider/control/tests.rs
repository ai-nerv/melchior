use super::*;
use crate::mind::provider::testing::{Server, Step, reply};

#[tokio::test]
async fn control_byte_limit_accepts_exact_size_and_rejects_declared_or_streamed_excess() {
    let limits = Limits {
        bytes: 32,
        timeout: Duration::from_secs(2),
    };
    let server = Server::start(vec![
        reply(400, &[b'a'; 32]),
        reply(400, &[b'a'; 33]),
        vec![
            Step::Bytes(b"HTTP/1.1 400 Error\r\nContent-Length: 999999999\r\n\r\n".to_vec()),
            Step::Hold,
        ],
    ])
    .await;
    let client = reqwest::Client::new();
    for expected in [true, false, false] {
        let response = client.get(&server.url).send().await.expect("headers");
        let result = read(
            response,
            limits,
            tokio::time::Instant::now() + limits.timeout,
        )
        .await;
        if expected {
            assert_eq!(result.expect("exact cap"), "a".repeat(32));
        } else {
            assert!(matches!(result, Err(Error::TooLarge)));
        }
    }
}

#[tokio::test]
async fn compressed_control_bodies_are_limited_after_decompression() {
    use base64::Engine;
    let compressed = base64::engine::general_purpose::STANDARD.decode("H4sIAAAAAAAAA+3BgQAAAADDILb5S/0gVQEAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAMANrYbBPgEAAQA=").expect("gzip fixture");
    let mut response = format!("HTTP/1.1 400 Error\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", compressed.len()).into_bytes();
    response.extend_from_slice(&compressed);
    let server = Server::start(vec![vec![Step::Bytes(response)]]).await;
    let response = reqwest::Client::new()
        .get(&server.url)
        .send()
        .await
        .expect("compressed response");
    assert!(matches!(
        read(
            response,
            LIMITS,
            tokio::time::Instant::now() + LIMITS.timeout
        )
        .await,
        Err(Error::TooLarge)
    ));
}
