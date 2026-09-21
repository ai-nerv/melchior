//! Bounded reads for finite provider and OAuth control responses.

use futures_util::StreamExt;
use std::time::Duration;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy)]
pub(super) struct Limits {
    pub bytes: usize,
    pub timeout: Duration,
}

pub(super) const LIMITS: Limits = Limits {
    bytes: 64 * 1024,
    timeout: Duration::from_secs(10),
};

#[derive(Debug, thiserror::Error)]
pub(super) enum Error {
    #[error("control response exceeded its byte limit")]
    TooLarge,
    #[error("control response exceeded its deadline")]
    Timeout,
    #[error("control response could not be read")]
    Transport,
    #[error("control response contains malformed UTF-8")]
    Encoding,
}

pub(super) async fn read(
    response: reqwest::Response,
    limits: Limits,
    deadline: tokio::time::Instant,
) -> Result<String, Error> {
    let reading = async {
        if response
            .content_length()
            .is_some_and(|length| length > limits.bytes as u64)
        {
            return Err(Error::TooLarge);
        }
        let mut chunks = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = chunks.next().await {
            let chunk = chunk.map_err(|_| Error::Transport)?;
            if chunk.len() > limits.bytes.saturating_sub(bytes.len()) {
                return Err(Error::TooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        String::from_utf8(bytes).map_err(|_| Error::Encoding)
    };
    tokio::time::timeout_at(deadline, reading)
        .await
        .map_err(|_| Error::Timeout)?
}
