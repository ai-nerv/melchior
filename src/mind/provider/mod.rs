pub mod api;
pub mod client;
pub mod compat;
mod control;
pub mod endpoint;
pub mod model;
pub mod oauth;
pub mod retry;
pub mod sse;

#[cfg(test)]
mod testing;

/// Every HTTP client melchior builds starts here: a redirect is not followed, so a credential
/// header never reaches a host it was not sent to, and never over plain HTTP after HTTPS.
pub fn http() -> reqwest::ClientBuilder {
    reqwest::Client::builder().redirect(reqwest::redirect::Policy::none())
}

#[cfg(test)]
mod redirect_tests {
    use super::testing::{Server, Step};
    use std::sync::atomic::Ordering;

    #[tokio::test]
    async fn a_redirect_is_not_followed_so_a_key_goes_nowhere_else() {
        let elsewhere = Server::start(vec![super::testing::reply(200, b"{}")]).await;
        let moved = format!(
            "HTTP/1.1 307 Moved\r\nLocation: {}/stolen\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            elsewhere.url
        );
        let asked = Server::start(vec![vec![Step::Bytes(moved.into_bytes())]]).await;
        let client = super::http().build().expect("a client");
        let answered = client
            .get(format!("{}/v1/models", asked.url))
            .header("x-api-key", "the-key")
            .send()
            .await
            .expect("an answer");
        assert_eq!(answered.status().as_u16(), 307);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(elsewhere.requests.load(Ordering::SeqCst), 0, "it followed");
    }
}
