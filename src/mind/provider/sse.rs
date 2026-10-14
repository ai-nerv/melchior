//! One SSE parser, for every provider.
//!
//! Pi ended up with four because four vendors frame slightly differently. There is only one
//! framing in the spec — `field: value` lines, blank line ends an event, `\r`, `\n`, or `\r\n`
//! all terminate — so there is one parser here and vendors differ in payload, not in framing.
//! What one line, one event and one whole stream may hold is bounded.

#[cfg(test)]
mod partitions;

/// The most one line may hold, in bytes.
pub const LINE: usize = 4 << 20;

/// The most one event's data may hold, in bytes.
pub const EVENT: usize = 8 << 20;

/// The most one stream may carry, in bytes.
pub const TOTAL: usize = 256 << 20;

/// One server-sent event.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Event {
    /// The `event:` field, empty when absent.
    pub name: String,
    /// The `data:` field, with multiple data lines joined by newlines.
    pub data: String,
}

/// What may be held, in bytes: a line, an event's data, the stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub line: usize,
    pub event: usize,
    pub total: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            line: LINE,
            event: EVENT,
            total: TOTAL,
        }
    }
}

/// Accumulates bytes and yields whole events.
#[derive(Debug, Default)]
pub struct Parser {
    buffer: Vec<u8>,
    name: String,
    data: Vec<String>,
    skip_lf: bool,
    started: bool,
    failed: Option<Malformed>,
    limits: Limits,
    /// Bytes of data held for the event being built, and bytes seen in the whole stream.
    held: usize,
    seen: usize,
}

/// Why a stream could not be read any further.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Malformed {
    #[error("the event stream contains malformed UTF-8")]
    Utf8,
    #[error("an event stream line is longer than this reads")]
    Line,
    #[error("an event in the stream is larger than this reads")]
    Event,
    #[error("the event stream is larger than this reads")]
    Total,
}

pub use Malformed::Utf8 as InvalidUtf8;

impl Parser {
    /// A parser with nothing buffered.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The same, holding no more than `limits`.
    #[must_use]
    pub fn within(limits: Limits) -> Self {
        Self {
            limits,
            ..Self::default()
        }
    }

    /// Feed bytes, retaining partial UTF-8 lines and folding CRLF across chunks.
    pub fn push(&mut self, chunk: impl AsRef<[u8]>) -> Result<Vec<Event>, Malformed> {
        let mut out = Vec::new();
        self.feed(chunk, |event| out.push(event))?;
        Ok(out)
    }

    /// Deliver each completed event before inspecting subsequent bytes, including malformed tails.
    pub fn feed(
        &mut self,
        chunk: impl AsRef<[u8]>,
        mut emit: impl FnMut(Event),
    ) -> Result<(), Malformed> {
        if let Some(why) = self.failed {
            return Err(why);
        }
        self.seen = self.seen.saturating_add(chunk.as_ref().len());
        if self.seen > self.limits.total {
            return Err(self.fail(Malformed::Total));
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
            } else if self.buffer.len() >= self.limits.line {
                return Err(self.fail(Malformed::Line));
            } else {
                self.buffer.push(byte);
            }
        }
        Ok(())
    }

    /// Take whatever is buffered, for a stream that ended without a final blank line.
    pub fn finish(&mut self) -> Result<Option<Event>, Malformed> {
        if let Some(why) = self.failed {
            return Err(why);
        }
        if !self.buffer.is_empty() {
            let _ = self.line()?;
        }
        self.skip_lf = false;
        Ok(self.take())
    }

    fn fail(&mut self, why: Malformed) -> Malformed {
        self.failed = Some(why);
        why
    }

    fn line(&mut self) -> Result<Option<Event>, Malformed> {
        let bytes = std::mem::take(&mut self.buffer);
        let Ok(text) = String::from_utf8(bytes) else {
            return Err(self.fail(Malformed::Utf8));
        };
        let line = if std::mem::replace(&mut self.started, true) {
            text.as_str()
        } else {
            text.strip_prefix('\u{feff}').unwrap_or(&text)
        };
        if line.is_empty() {
            return Ok(self.take());
        }
        self.field(line)?;
        Ok(None)
    }

    fn field(&mut self, line: &str) -> Result<(), Malformed> {
        // A comment keeps the connection warm and carries nothing.
        if line.starts_with(':') {
            return Ok(());
        }
        let (name, value) = match line.split_once(':') {
            Some((name, value)) => (name, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match name {
            "event" => self.name = value.to_owned(),
            "data" => {
                self.held = self.held.saturating_add(value.len() + 1);
                if self.held > self.limits.event {
                    return Err(self.fail(Malformed::Event));
                }
                self.data.push(value.to_owned());
            }
            _ => {}
        }
        Ok(())
    }

    fn take(&mut self) -> Option<Event> {
        self.held = 0;
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

/// A provider cannot make the reader hold more than it bounds.
#[cfg(test)]
mod bounds {
    use super::*;

    fn small() -> Parser {
        Parser::within(Limits {
            line: 16,
            event: 32,
            total: 128,
        })
    }

    #[test]
    fn a_line_with_no_end_is_refused_once_it_is_too_long() {
        let mut parser = small();
        assert_eq!(parser.push("data: ".repeat(4)), Err(Malformed::Line));
        assert_eq!(
            parser.push("\n\n"),
            Err(Malformed::Line),
            "and it stays refused"
        );
    }

    #[test]
    fn an_event_of_many_short_lines_is_refused_once_it_is_too_large() {
        let mut parser = small();
        assert_eq!(
            parser.push("data: 1234567\n".repeat(5)),
            Err(Malformed::Event)
        );
        let mut fine = small();
        let events = fine
            .push("data: 1234567\n\n".repeat(3))
            .expect("each event is small");
        assert_eq!(events.len(), 3, "the count is per event");
    }

    #[test]
    fn a_stream_is_refused_once_it_has_carried_too_much() {
        let mut parser = small();
        let mut outcome = Ok(Vec::new());
        for _ in 0..20 {
            outcome = parser.push("data: x\n\n");
            if outcome.is_err() {
                break;
            }
        }
        assert_eq!(outcome, Err(Malformed::Total));
    }
}
