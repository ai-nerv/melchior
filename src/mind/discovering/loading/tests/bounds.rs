use super::*;

#[tokio::test]
async fn discovery_rejects_declared_and_streamed_oversized_bodies() {
    for chunked in [false, true] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener");
        let url = format!("http://{}/models", listener.local_addr().expect("address"));
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(socket.read_u8().await.expect("request"));
            }
            if chunked {
                socket.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n")
                    .await.expect("headers");
                let mut chunk = b"100000\r\n".to_vec();
                chunk.extend(vec![b' '; 1024 * 1024]);
                chunk.extend(b"\r\n");
                for _ in 0..9 {
                    if socket.write_all(&chunk).await.is_err() {
                        break;
                    }
                }
                let _ = socket.write_all(b"0\r\n\r\n").await;
            } else {
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            cache::BYTES + 1
                        )
                        .as_bytes(),
                    )
                    .await
                    .expect("headers");
                let mut bytes = [0; 128];
                assert_eq!(socket.read(&mut bytes).await.expect("closed"), 0);
            }
        });
        let client = crate::mind::provider::http()
            .timeout(Duration::from_secs(3))
            .build()
            .expect("client");
        let response = client.get(url).send().await.expect("response");
        assert!(bounded(response).await.is_none());
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("bounded server")
            .expect("server");
    }
}
