use super::{Auth, Provider, Store, oauth};
use anyhow::{Context, Result, bail};
use std::io::{Read, Write};
use std::time::{Duration, Instant};

fn safe_url(value: &str) -> bool {
    reqwest::Url::parse(value).is_ok_and(|url| {
        url.username().is_empty()
            && url.password().is_none()
            && (url.scheme() == "https"
                || (url.scheme() == "http"
                    && matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "localhost"))))
    })
}

pub(super) fn configured(provider: &Provider) -> bool {
    matches!(&provider.auth, Auth::OAuth { authorize_url: Some(a), token_url: Some(t), client_id: Some(c), .. }
        if safe_url(a) && safe_url(t) && !c.trim().is_empty())
}

pub(super) async fn login(
    provider: &Provider,
    ready: impl FnOnce(&str) -> Result<()>,
) -> Result<()> {
    if !configured(provider) {
        bail!("OAuth endpoints and public client ID are not configured");
    }
    let Auth::OAuth {
        authorize_url: Some(authorize),
        token_url: Some(token),
        client_id: Some(client),
        scopes,
        ..
    } = &provider.auth
    else {
        bail!("this provider does not support browser login");
    };
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let redirect = format!("http://127.0.0.1:{}", listener.local_addr()?.port());
    let pkce = oauth::Pkce::generate()?;
    let state = oauth::Pkce::generate()?.verifier;
    let attempt = oauth::Pkce::generate()?.verifier;
    {
        let _held = oauth::hold(&provider.id)?;
        Store::amend(|s| {
            s.attempts.insert(provider.id.clone(), attempt.clone());
        })?;
    }
    let url = oauth::authorize_url(authorize, client, &redirect, scopes, &pkce, &state);
    ready(&url)?;
    let code = callback(&listener, &state, Duration::from_secs(300))?;
    let tokens = oauth::exchange(
        &crate::mind::provider::http().build()?,
        token,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", &redirect),
            ("client_id", client),
            ("code_verifier", &pkce.verifier),
        ],
    )
    .await?;
    if tokens.access.trim().is_empty() {
        bail!("empty access token");
    }
    let _held = oauth::hold(&provider.id)?;
    let mut saved = false;
    Store::amend(|store| {
        if store.attempts.get(&provider.id) == Some(&attempt) {
            store.put(&provider.id, tokens);
            store
                .configurations
                .insert(provider.id.clone(), provider.credential_scope());
            store.attempts.remove(&provider.id);
            saved = true;
        }
    })?;
    if !saved {
        bail!("sign-in was superseded or removed");
    }
    Ok(())
}

fn callback(listener: &std::net::TcpListener, state: &str, patience: Duration) -> Result<String> {
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + patience;
    while Instant::now() < deadline {
        let mut socket = match listener.accept() {
            Ok((socket, _)) => socket,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        socket.set_write_timeout(Some(Duration::from_secs(2)))?;
        let Some(line) = request_line(
            &mut socket,
            deadline.min(Instant::now() + Duration::from_secs(2)),
        )?
        else {
            continue;
        };
        socket.set_nonblocking(false)?;
        let mut parts = line.split_whitespace();
        if parts.next() != Some("GET") {
            continue;
        }
        let Some(target) = parts.next().filter(|t| t.starts_with("/?")) else {
            continue;
        };
        let url = reqwest::Url::parse(&format!("http://127.0.0.1{target}"))?;
        let pairs: Vec<_> = url.query_pairs().collect();
        let valid = pairs.iter().filter(|(k, _)| k == "state").count() == 1
            && pairs.iter().any(|(k, v)| k == "state" && v == state);
        let answer = if valid {
            "Authorization received. Return to the terminal to check the result."
        } else {
            "Invalid sign-in state."
        };
        let _ = write!(
            socket,
            "HTTP/1.1 {}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            if valid { "200 OK" } else { "400 Bad Request" },
            answer.len(),
            answer
        );
        if !valid {
            continue;
        }
        if pairs.iter().any(|(k, _)| k == "error") {
            bail!("authorization denied");
        }
        if pairs.iter().filter(|(k, _)| k == "code").count() != 1 {
            bail!("invalid authorization code");
        }
        return pairs
            .into_iter()
            .find(|(k, v)| k == "code" && !v.is_empty())
            .map(|(_, v)| v.into_owned())
            .context("missing authorization code");
    }
    bail!("browser sign-in timed out")
}

fn request_line(socket: &mut std::net::TcpStream, deadline: Instant) -> Result<Option<String>> {
    socket.set_nonblocking(true)?;
    let mut bytes = Vec::new();
    while Instant::now() < deadline {
        let mut chunk = [0; 1024];
        match socket.read(&mut chunk) {
            Ok(0) => return Ok(None),
            Ok(n) => {
                let part = chunk.get(..n).context("invalid callback read length")?;
                let end = part.iter().position(|b| *b == b'\n').map(|at| at + 1);
                bytes.extend_from_slice(
                    part.get(..end.unwrap_or(n))
                        .context("invalid callback line length")?,
                );
                if bytes.len() > 8192 {
                    return Ok(None);
                }
                if end.is_some() {
                    return Ok(String::from_utf8(bytes).ok());
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) =>
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(_) => return Ok(None),
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_and_slow_request_have_absolute_deadlines() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("listener");
        let start = Instant::now();
        assert!(callback(&listener, "state", Duration::from_millis(30)).is_err());
        let _peer =
            std::net::TcpStream::connect(listener.local_addr().expect("address")).expect("peer");
        listener.set_nonblocking(false).expect("blocking accept");
        let (mut socket, _) = listener.accept().expect("accept");
        assert!(
            request_line(&mut socket, Instant::now() + Duration::from_millis(30))
                .expect("read")
                .is_none()
        );
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn browser_endpoints_allow_tls_and_loopback_but_not_userinfo_or_remote_plaintext() {
        assert!(safe_url("https://fixture.invalid/oauth"));
        assert!(safe_url("http://127.0.0.1:1234/token"));
        for url in [
            "http://fixture.invalid/token",
            "https://user:secret@fixture.invalid",
            "file:///tmp/code",
        ] {
            assert!(!safe_url(url));
        }
    }
}
