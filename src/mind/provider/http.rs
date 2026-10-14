//! The HTTP client every provider, discovery and sign-in request is sent with. It follows no
//! redirect, and it speaks plain HTTP to this machine only: anywhere else is https or nothing.

use reqwest::header::{HeaderName, HeaderValue};
use std::time::Duration;

/// Every HTTP client melchior builds starts here.
pub fn http() -> Builder {
    Builder(reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()))
}

/// A client still being set up.
pub struct Builder(reqwest::ClientBuilder);

impl Builder {
    /// How long a whole request, answer included, may take.
    #[must_use]
    pub fn timeout(self, limit: Duration) -> Self {
        Self(self.0.timeout(limit))
    }

    /// How long opening a connection may take.
    #[must_use]
    pub fn connect_timeout(self, limit: Duration) -> Self {
        Self(self.0.connect_timeout(limit))
    }

    /// The client.
    ///
    /// # Errors
    ///
    /// When the TLS backend cannot be set up.
    pub fn build(self) -> reqwest::Result<Http> {
        self.0.build().map(Http)
    }
}

/// A client whose every request is checked for where it goes before it is sent.
#[derive(Clone)]
pub struct Http(reqwest::Client);

impl Default for Http {
    fn default() -> Self {
        http()
            .build()
            .unwrap_or_else(|_| Self(reqwest::Client::default()))
    }
}

impl Http {
    /// `GET url`.
    pub fn get(&self, url: impl reqwest::IntoUrl) -> Request {
        Request(self.0.get(url))
    }

    /// `POST url`.
    pub fn post(&self, url: impl reqwest::IntoUrl) -> Request {
        Request(self.0.post(url))
    }
}

/// A request being built.
pub struct Request(reqwest::RequestBuilder);

impl Request {
    #[must_use]
    pub fn header(self, name: HeaderName, value: HeaderValue) -> Self {
        Self(self.0.header(name, value))
    }

    #[must_use]
    pub fn bearer_auth(self, token: impl std::fmt::Display) -> Self {
        Self(self.0.bearer_auth(token))
    }

    #[must_use]
    pub fn json<T: serde::Serialize + ?Sized>(self, body: &T) -> Self {
        Self(self.0.json(body))
    }

    #[must_use]
    pub fn form<T: serde::Serialize + ?Sized>(self, body: &T) -> Self {
        Self(self.0.form(body))
    }

    /// Send it, once its URL is known to be https or this machine.
    ///
    /// # Errors
    ///
    /// [`Unanswered::Refused`] before anything is sent, or whatever reqwest failed it with.
    pub async fn send(self) -> Result<reqwest::Response, Unanswered> {
        let (client, request) = self.0.build_split();
        let request = request?;
        admitted(request.url())?;
        Ok(client.execute(request).await?)
    }
}

/// Why a request has no answer.
#[derive(Debug, thiserror::Error)]
pub enum Unanswered {
    /// Never sent: plain text to a host off this machine.
    #[error("not sent: {scheme}://{host} is neither https nor this machine")]
    Refused { scheme: String, host: String },
    #[error(transparent)]
    Http(#[from] reqwest::Error),
}

/// Whether `url` may be sent to: plain http to this machine only. https goes anywhere, and reqwest
/// refuses every other scheme itself.
fn admitted(url: &reqwest::Url) -> Result<(), Unanswered> {
    let host = url.host_str().unwrap_or_default();
    if url.scheme() != "http" || local(host) {
        return Ok(());
    }
    crate::noted!("http: refused {}://{host}", url.scheme());
    Err(Unanswered::Refused {
        scheme: url.scheme().to_owned(),
        host: host.to_owned(),
    })
}

/// `localhost`, a loopback address, or the unspecified address, which reaches this machine too.
fn local(host: &str) -> bool {
    let bare = host
        .strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .unwrap_or(host);
    bare.eq_ignore_ascii_case("localhost")
        || bare
            .parse::<std::net::IpAddr>()
            .map(|ip| ip.to_canonical())
            .is_ok_and(|ip| ip.is_loopback() || ip.is_unspecified())
}

#[cfg(test)]
mod tests {
    use super::{Unanswered, admitted, http};
    use crate::mind::provider::testing::{Server, Step, reply};
    use reqwest::header::{HeaderName, HeaderValue};
    use std::sync::atomic::Ordering;

    fn verdict(url: &str) -> Result<(), Unanswered> {
        admitted(&reqwest::Url::parse(url).expect("a url"))
    }

    fn moved_to(location: &str) -> Vec<Step> {
        let moved = format!(
            "HTTP/1.1 307 Moved\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        vec![Step::Bytes(moved.into_bytes())]
    }

    #[tokio::test]
    async fn plain_http_to_a_remote_host_is_refused_before_it_is_sent() {
        let client = http().build().expect("a client");
        let refused = client
            .get("http://models.invalid/v1/models?key=the-key")
            .bearer_auth("the-key")
            .send()
            .await
            .expect_err("refused");
        let said = refused.to_string();
        assert!(matches!(refused, Unanswered::Refused { .. }), "{said}");
        assert!(said.contains("http://models.invalid "), "{said}");
        assert!(!said.contains("the-key"), "{said}");
    }

    #[test]
    fn plain_http_off_this_machine_is_refused_whatever_names_it() {
        for url in [
            "http://10.0.0.1:11434/v1",
            "http://localhost.example.com/v1",
            "http://127.0.0.1.example.com/v1",
            "http://[2001:db8::1]:11434/v1",
        ] {
            assert!(
                matches!(verdict(url), Err(Unanswered::Refused { .. })),
                "{url}"
            );
        }
    }

    #[test]
    fn plain_http_to_this_machine_is_allowed() {
        for url in [
            "http://localhost:11434/v1",
            "http://LOCALHOST/v1",
            "http://127.0.0.1:11434/v1",
            "http://127.8.9.10/v1",
            "http://[::1]:11434/v1",
            "http://[::ffff:127.0.0.1]/v1",
            "http://0.0.0.0:11434/v1",
        ] {
            assert!(verdict(url).is_ok(), "{url}");
        }
    }

    #[test]
    fn https_is_allowed_anywhere() {
        for url in ["https://api.example.com/v1", "https://10.0.0.1:8443/v1"] {
            assert!(verdict(url).is_ok(), "{url}");
        }
    }

    #[tokio::test]
    async fn plain_http_to_loopback_is_sent() {
        let server = Server::start(vec![reply(200, b"{}")]).await;
        let client = http().build().expect("a client");
        let answered = client
            .get(format!("{}/v1/models", server.url))
            .send()
            .await
            .expect("an answer");
        assert_eq!(answered.status().as_u16(), 200);
        assert_eq!(server.requests.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_redirect_is_not_followed_so_a_key_goes_nowhere_else() {
        let elsewhere = Server::start(vec![reply(200, b"{}")]).await;
        let asked = Server::start(vec![moved_to(&format!("{}/stolen", elsewhere.url))]).await;
        let client = http().build().expect("a client");
        let answered = client
            .get(format!("{}/v1/models", asked.url))
            .header(
                HeaderName::from_static("x-api-key"),
                HeaderValue::from_static("the-key"),
            )
            .send()
            .await
            .expect("an answer");
        assert_eq!(answered.status().as_u16(), 307);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(elsewhere.requests.load(Ordering::SeqCst), 0, "it followed");
    }

    #[tokio::test]
    async fn a_redirect_down_to_plain_http_elsewhere_is_handed_back_unfollowed() {
        let asked = Server::start(vec![moved_to("http://downgraded.invalid/stolen")]).await;
        let client = http().build().expect("a client");
        let answered = client
            .get(format!("{}/v1/models", asked.url))
            .bearer_auth("the-key")
            .send()
            .await
            .expect("the redirect itself, unfollowed");
        assert_eq!(answered.status().as_u16(), 307);
        assert_eq!(
            answered
                .headers()
                .get("location")
                .map(HeaderValue::as_bytes),
            Some(&b"http://downgraded.invalid/stolen"[..])
        );
    }
}
