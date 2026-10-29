use super::{Counts, Event, Turn};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt as _;

type Frame = (&'static str, Value);

struct Encoder {
    index: usize,
    open: Option<&'static str>,
    signed: bool,
    finished: bool,
    usage: Counts,
}

impl Encoder {
    fn new(usage: Counts) -> Self {
        Self {
            index: 0,
            open: None,
            signed: false,
            finished: false,
            usage,
        }
    }

    fn start(&self) -> Frame {
        (
            "message_start",
            json!({"type":"message_start","message":{"id":"msg-fixture","type":"message",
            "role":"assistant","model":"fake-model","content":[],"stop_reason":null,"stop_sequence":null,
            "usage":{"input_tokens":self.usage.input,"output_tokens":0,
                "cache_read_input_tokens":self.usage.cache_read,"cache_creation_input_tokens":self.usage.cache_write}}}),
        )
    }

    fn close(&mut self, out: &mut Vec<Frame>) {
        if self.open.take().is_some() {
            out.push((
                "content_block_stop",
                json!({"type":"content_block_stop","index":self.index}),
            ));
            self.index += 1;
        }
    }

    fn block(&mut self, kind: &'static str, out: &mut Vec<Frame>) {
        if self.open != Some(kind) {
            self.close(out);
            self.open = Some(kind);
            self.signed = false;
            let block = if kind == "thinking" {
                json!({"type":"thinking","thinking":"","signature":""})
            } else {
                json!({"type":"text","text":""})
            };
            out.push((
                "content_block_start",
                json!({"type":"content_block_start","index":self.index,"content_block":block}),
            ));
        }
    }

    fn event(&mut self, event: &Event) -> std::io::Result<Vec<Frame>> {
        let mut out = Vec::new();
        if matches!(event, Event::Pause(_) | Event::Drop) {
            return Ok(out);
        }
        if self.finished {
            return Err(std::io::Error::other("event after Anthropic message_stop"));
        }
        match event {
            Event::Text(text) | Event::Split(text) => {
                self.block("text", &mut out);
                out.push(("content_block_delta",json!({"type":"content_block_delta","index":self.index,"delta":{"type":"text_delta","text":text}})));
            }
            Event::Thinking(text) => {
                self.block("thinking", &mut out);
                if self.signed {
                    return Err(std::io::Error::other(
                        "thinking delta follows its signature",
                    ));
                }
                out.push(("content_block_delta",json!({"type":"content_block_delta","index":self.index,"delta":{"type":"thinking_delta","thinking":text}})));
            }
            Event::Signature(signature) => {
                if self.open != Some("thinking") {
                    return Err(std::io::Error::other(
                        "signature without an open thinking block",
                    ));
                }
                self.signed = true;
                out.push(("content_block_delta",json!({"type":"content_block_delta","index":self.index,"delta":{"type":"signature_delta","signature":signature}})));
            }
            Event::ToolCall {
                id,
                name,
                arguments,
            } => {
                self.close(&mut out);
                out.push((
                    "content_block_start",
                    json!({"type":"content_block_start","index":self.index,
                    "content_block":{"type":"tool_use","id":id,"name":name,"input":{}}}),
                ));
                out.push((
                    "content_block_delta",
                    json!({"type":"content_block_delta","index":self.index,
                    "delta":{"type":"input_json_delta","partial_json":arguments}}),
                ));
                out.push((
                    "content_block_stop",
                    json!({"type":"content_block_stop","index":self.index}),
                ));
                self.index += 1;
            }
            Event::Finish(reason) => {
                let reason = match reason.as_str() {
                    "stop" | "end_turn" => "end_turn",
                    "tool_calls" | "tool_use" => "tool_use",
                    "length" | "max_tokens" => "max_tokens",
                    _ => return Err(std::io::Error::other("unknown Anthropic finish reason")),
                };
                self.close(&mut out);
                out.push(("message_delta",json!({"type":"message_delta","delta":{"stop_reason":reason,"stop_sequence":null},
                    "usage":{"output_tokens":self.usage.output}})));
                out.push(("message_stop", json!({"type":"message_stop"})));
                self.finished = true;
            }
            Event::Usage { .. } | Event::Metrics(_) => {
                return Err(std::io::Error::other(
                    "Anthropic counts belong in turn.usage",
                ));
            }
            Event::Drop | Event::Pause(_) => {}
        }
        Ok(out)
    }
}

pub(super) fn validate(turn: &Turn) -> std::io::Result<()> {
    let mut encoder = Encoder::new(turn.usage.unwrap_or_default());
    for event in &turn.events {
        if matches!(event, Event::Drop) {
            return Ok(());
        }
        encoder.event(event)?;
    }
    if !encoder.finished {
        return Err(std::io::Error::other(
            "Anthropic turn has neither finish nor drop",
        ));
    }
    Ok(())
}

fn render((name, data): &Frame, ending: &str) -> String {
    format!("event: {name}{ending}data: {data}{ending}{ending}")
}

pub(super) async fn stream(socket: &mut tokio::net::TcpStream, turn: &Turn) -> std::io::Result<()> {
    super::start_stream(socket).await?;
    let mut encoder = Encoder::new(turn.usage.unwrap_or_default());
    super::chunk(socket, &render(&encoder.start(), "\n")).await?;
    for event in &turn.events {
        match event {
            Event::Pause(ms) => {
                socket.flush().await?;
                tokio::time::sleep(std::time::Duration::from_millis(*ms)).await;
                continue;
            }
            Event::Drop => return socket.shutdown().await,
            _ => {}
        }
        let frames = encoder.event(event)?;
        if matches!(event, Event::Split(_)) {
            let bytes: String = frames.iter().map(|frame| render(frame, "\r\n")).collect();
            let bytes = bytes.as_bytes();
            let at = bytes
                .iter()
                .position(|byte| *byte >= 0x80)
                .map_or(bytes.len() / 2, |index| index + 1);
            let (head, tail) = bytes
                .split_at_checked(at)
                .ok_or_else(|| std::io::Error::other("invalid fixture split"))?;
            for part in [head, tail] {
                let mut framed = format!("{:x}\r\n", part.len()).into_bytes();
                framed.extend_from_slice(part);
                framed.extend_from_slice(b"\r\n");
                socket.write_all(&framed).await?;
                socket.flush().await?;
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        } else {
            for frame in frames {
                super::chunk(socket, &render(&frame, "\n")).await?;
            }
        }
    }
    socket.write_all(b"0\r\n\r\n").await
}

#[cfg(test)]
#[path = "anthropic/tests.rs"]
mod tests;
