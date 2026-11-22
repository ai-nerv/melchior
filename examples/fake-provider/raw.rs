use super::{Event, Turn};
use tokio::io::AsyncWriteExt as _;

pub(super) fn frame(event: &Event) -> std::io::Result<String> {
    let Event::Sse { name, data } = event else {
        return Err(std::io::Error::other("raw stream needs an SSE frame"));
    };
    if name.as_ref().is_some_and(|name| {
        name.is_empty()
            || name.len() > 128
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    }) {
        return Err(std::io::Error::other("invalid raw SSE event name"));
    }
    let named = name
        .as_ref()
        .map_or_else(String::new, |name| format!("event: {name}\n"));
    let frame = format!("{named}data: {data}\n\n");
    if frame.len() > 256 * 1024 {
        return Err(std::io::Error::other("raw SSE frame exceeds fixture cap"));
    }
    Ok(frame)
}

pub(super) fn validate(turn: &Turn) -> std::io::Result<()> {
    if turn.usage.is_some() || (turn.events.is_empty() && turn.refuse.is_none()) {
        return Err(std::io::Error::other(
            "raw SSE requires literal frames or a refusal",
        ));
    }
    for event in &turn.events {
        match event {
            Event::Pause(_) | Event::Drop => {}
            _ => {
                frame(event)?;
            }
        }
    }
    Ok(())
}

pub(super) async fn stream(socket: &mut tokio::net::TcpStream, turn: &Turn) -> std::io::Result<()> {
    super::start_stream(socket).await?;
    for event in &turn.events {
        match event {
            Event::Drop => return socket.shutdown().await,
            Event::Pause(ms) => {
                socket.flush().await?;
                tokio::time::sleep(std::time::Duration::from_millis(*ms)).await;
            }
            _ => super::chunk(socket, &frame(event)?).await?,
        }
    }
    socket.write_all(b"0\r\n\r\n").await?;
    socket.flush().await
}

#[cfg(test)]
#[path = "raw/tests.rs"]
mod tests;
