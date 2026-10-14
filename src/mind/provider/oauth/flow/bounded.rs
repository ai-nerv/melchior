use super::*;
use crate::mind::provider::{
    Http, control,
    testing::{Server, Step, chunk, headers, reply},
};
use std::time::Duration;

fn limits() -> control::Limits {
    control::Limits {
        bytes: 64 * 1024,
        timeout: Duration::from_millis(75),
    }
}

#[tokio::test]
async fn oauth_control_reads_bound_headers_bodies_and_total_drip_time() {
    let mut drip = vec![headers(200)];
    for _ in 0..20 {
        drip.push(chunk(b" "));
        drip.push(Step::Pause(Duration::from_millis(15)));
    }
    let cumulative = vec![
        Step::Pause(Duration::from_millis(50)),
        headers(200),
        Step::Pause(Duration::from_millis(50)),
        chunk(b"{\"access_token\":\"token\"}"),
        Step::Bytes(b"0\r\n\r\n".to_vec()),
    ];
    for steps in [
        vec![Step::Hold],
        vec![headers(200), Step::Hold],
        drip,
        cumulative,
    ] {
        let server = Server::start(vec![steps]).await;
        let began = std::time::Instant::now();
        let why = tokio::time::timeout(
            Duration::from_secs(3),
            exchange_with_limits(
                &Http::default(),
                &server.url,
                &[("refresh_token", "SECRET-refresh")],
                limits(),
            ),
        )
        .await
        .expect("bounded exchange")
        .expect_err("stall");
        assert!(why.to_string().contains("deadline"), "{why}");
        assert!(began.elapsed() < Duration::from_secs(3));
    }
}

#[tokio::test]
async fn oauth_oversized_invalid_and_error_responses_never_echo_secrets() {
    let too_large = vec![b'x'; limits().bytes + 1];
    for (status, body, reason) in [
        (200, too_large.as_slice(), "byte limit"),
        (401, b"SECRET-access SECRET-refresh".as_slice(), "401"),
        (
            200,
            b"{\"access_token\":\"SECRET-access\", \"expires_in\":\"SECRET-refresh\"}".as_slice(),
            "unreadable",
        ),
        (200, b"\xff".as_slice(), "UTF-8"),
    ] {
        let server = Server::start(vec![reply(status, body)]).await;
        let why = exchange_with_limits(
            &Http::default(),
            &format!("{}?key=SECRET-url", server.url),
            &[],
            control::LIMITS,
        )
        .await
        .expect_err("invalid control response");
        assert!(why.to_string().contains(reason), "{why}");
        assert!(!why.to_string().contains("SECRET"), "{why}");
    }
}

#[tokio::test]
async fn oauth_valid_unicode_tokens_survive_byte_chunks_and_expiry_cannot_wrap() {
    let body = serde_json::json!({"access_token":"clé-🦀", "refresh_token":"refresh-é", "expires_in":u64::MAX}).to_string();
    let mut steps = vec![headers(200)];
    steps.extend(body.as_bytes().chunks(1).map(chunk));
    steps.push(Step::Bytes(b"0\r\n\r\n".to_vec()));
    let server = Server::start(vec![steps]).await;
    let tokens = exchange(&Http::default(), &server.url, &[])
        .await
        .expect("tokens");
    assert_eq!(tokens.access, "clé-🦀");
    assert_eq!(tokens.refresh.as_deref(), Some("refresh-é"));
    assert_eq!(tokens.expires_at, u64::MAX);
}

#[tokio::test]
async fn cancelling_an_oauth_exchange_closes_its_connection() {
    let server = Server::start(vec![vec![headers(200), Step::Hold]]).await;
    let client = Http::default();
    {
        let exchange = exchange(&client, &server.url, &[]);
        tokio::pin!(exchange);
        tokio::select! {
            _ = &mut exchange => panic!("held exchange ended"),
            () = server.wait_for(&server.requests, 1) => {}
        }
    }
    server.wait_for(&server.closed, 1).await;
}

#[tokio::test]
async fn a_token_endpoint_off_this_machine_over_plain_http_is_refused() {
    let why = exchange(
        &Http::default(),
        "http://auth.invalid/token",
        &[("refresh_token", "SECRET-refresh")],
    )
    .await
    .expect_err("refused");
    assert!(why.to_string().contains("http://auth.invalid "), "{why}");
    assert!(!why.to_string().contains("SECRET"), "{why}");
}
