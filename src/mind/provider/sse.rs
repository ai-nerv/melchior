//! One SSE parser, for every provider.
//!
//! Pi ended up with four because four vendors frame slightly differently. There is only one
//! framing in the spec — `field: value` lines, blank line ends an event, `\r`, `\n`, or `\r\n`
//! all terminate — so there is one parser here and vendors differ in payload, not in framing.

#[cfg(test)]
mod partitions;

/// One server-sent event.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Event {
    /// The `event:` field, empty when absent.
    pub name: String,
    /// The `data:` field, with multiple data lines joined by newlines.
    pub data: String,
}

/// Accumulates bytes and yields whole events.
#[derive(Debug, Default)]
pub struct Parser {
    buffer: Vec<u8>,
    name: String,
    data: Vec<String>,
    skip_lf: bool,
    started: bool,
    failed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the event stream contains malformed UTF-8")]
pub struct InvalidUtf8;

impl Parser {
    /// A parser with nothing buffered.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed bytes, retaining partial UTF-8 lines and folding CRLF across chunks.
    pub fn push(&mut self, chunk: impl AsRef<[u8]>) -> Result<Vec<Event>, InvalidUtf8> {
        let mut out = Vec::new();
        self.feed(chunk, |event| out.push(event))?;
        Ok(out)
    }

    /// Deliver each completed event before inspecting subsequent bytes, including malformed tails.
    pub fn feed(
        &mut self,
        chunk: impl AsRef<[u8]>,
        mut emit: impl FnMut(Event),
    ) -> Result<(), InvalidUtf8> {
        if self.failed {
            return Err(InvalidUtf8);
        }
        for &byte in chunk.as_ref() {
            if std::mem::take(&mut self.skip_lf) && byte == b'\n' {
                continue;
            }
            if byte == b'\r' || byte == b'\n' {
                self.skip_lf = byte == b'\r';
                if let Some(event) = self.line()? {
                    emit(event);
                }
            } else {
                self.buffer.push(byte);
            }
        }
        Ok(())
    }

    /// Take whatever is buffered, for a stream that ended without a final blank line.
    pub fn finish(&mut self) -> Result<Option<Event>, InvalidUtf8> {
        if self.failed {
            return Err(InvalidUtf8);
        }
        if !self.buffer.is_empty() {
            let _ = self.line()?;
        }
        self.skip_lf = false;
        Ok(self.take())
    }

    fn line(&mut self) -> Result<Option<Event>, InvalidUtf8> {
        let bytes = std::mem::take(&mut self.buffer);
        let text = String::from_utf8(bytes).map_err(|_| {
            self.failed = true;
            InvalidUtf8
        })?;
        let line = if std::mem::replace(&mut self.started, true) {
            text.as_str()
        } else {
            text.strip_prefix('\u{feff}').unwrap_or(&text)
        };
        if line.is_empty() {
            return Ok(self.take());
        }
        self.field(line);
        Ok(None)
    }

    fn field(&mut self, line: &str) {
        // A comment keeps the connection warm and carries nothing.
        if line.starts_with(':') {
            return;
        }
        let (name, value) = match line.split_once(':') {
            Some((name, value)) => (name, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match name {
            "event" => self.name = value.to_owned(),
            "data" => self.data.push(value.to_owned()),
            _ => {}
        }
    }

    fn take(&mut self) -> Option<Event> {
        if self.data.is_empty() && self.name.is_empty() {
            return None;
        }
        Some(Event {
            name: std::mem::take(&mut self.name),
            data: std::mem::take(&mut self.data).join("\n"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(chunks: &[&str]) -> Vec<Event> {
        let mut parser = Parser::new();
        let mut out = Vec::new();
        for chunk in chunks {
            out.extend(parser.push(chunk).expect("valid UTF-8"));
        }
        out.extend(parser.finish().expect("valid tail"));
        out
    }

    #[test]
    fn a_whole_event_parses() {
        assert_eq!(
            parse(&["event: delta\ndata: hello\n\n"]),
            vec![Event {
                name: "delta".into(),
                data: "hello".into()
            }]
        );
    }

    #[test]
    fn an_event_split_across_chunks_still_parses() {
        assert_eq!(
            parse(&["event: del", "ta\ndata: hel", "lo\n\n"]),
            vec![Event {
                name: "delta".into(),
                data: "hello".into()
            }]
        );
    }

    #[test]
    fn all_three_line_terminators_work() {
        for terminator in ["\n", "\r", "\r\n"] {
            let source = format!("data: x{terminator}{terminator}");
            assert_eq!(parse(&[&source]).len(), 1, "terminator {terminator:?}");
        }
    }

    #[test]
    fn crlf_is_one_terminator_not_a_blank_line() {
        let events = parse(&["event: a\r\ndata: one\r\n\r\ndata: two\r\n\r\n"]);
        assert_eq!(events.len(), 2, "{events:?}");
        assert_eq!(events[0].data, "one");
    }

    #[test]
    fn multiple_data_lines_join_with_newlines() {
        assert_eq!(parse(&["data: a\ndata: b\n\n"])[0].data, "a\nb");
    }

    #[test]
    fn comments_are_ignored() {
        let events = parse(&[": keep-alive\ndata: real\n\n"]);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "real");
    }

    #[test]
    fn a_field_with_no_space_after_the_colon_still_parses() {
        assert_eq!(parse(&["data:tight\n\n"])[0].data, "tight");
    }

    #[test]
    fn a_stream_that_ends_without_a_blank_line_still_yields_its_event() {
        assert_eq!(parse(&["data: last\n"])[0].data, "last");
    }

    #[test]
    fn an_empty_stream_yields_nothing() {
        assert!(parse(&[""]).is_empty());
    }

    #[test]
    fn back_to_back_events_stay_separate() {
        let events = parse(&["data: one\n\ndata: two\n\ndata: three\n\n"]);
        assert_eq!(events.len(), 3);
        assert_eq!(events[2].data, "three");
    }
}
