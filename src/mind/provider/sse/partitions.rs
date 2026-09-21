use super::*;

fn parse(chunks: &[&[u8]]) -> Vec<Event> {
    let mut parser = Parser::new();
    let mut events = Vec::new();
    for chunk in chunks {
        events.extend(parser.push(chunk).expect("valid UTF-8"));
    }
    events.extend(parser.finish().expect("valid tail"));
    events
}

#[test]
fn every_byte_split_preserves_unicode_fields_and_mixed_terminators() {
    let source = ": keepalive\r\nevent: text\r\ndata: café 🦀 漢字\r\ndata: continued\r\n\r\nevent: tool\ndata: {\"city\":\"Zürich\"}\n\nevent: usage\rdata: 7\r\rdata: tail";
    let expected = vec![
        Event {
            name: "text".into(),
            data: "café 🦀 漢字\ncontinued".into(),
        },
        Event {
            name: "tool".into(),
            data: "{\"city\":\"Zürich\"}".into(),
        },
        Event {
            name: "usage".into(),
            data: "7".into(),
        },
        Event {
            name: String::new(),
            data: "tail".into(),
        },
    ];
    assert_eq!(parse(&[source.as_bytes()]), expected);
    for at in 0..=source.len() {
        assert_eq!(
            parse(&[&source.as_bytes()[..at], &[], &source.as_bytes()[at..]]),
            expected,
            "split {at}"
        );
    }
    for width in 1..=17 {
        assert_eq!(
            parse(&source.as_bytes().chunks(width).collect::<Vec<_>>()),
            expected,
            "width {width}"
        );
    }
}

#[test]
fn a_multibyte_scalar_split_across_http_chunks_is_not_replaced() {
    assert_eq!(
        parse(&[b"data: \xf0\x9f", b"\xa6\x80\n\n"]),
        vec![Event {
            name: String::new(),
            data: "🦀".into()
        }]
    );
}

#[test]
fn malformed_and_incomplete_utf8_are_explicit_terminal_failures() {
    for bytes in [
        b"data: \xff\n\n".as_slice(),
        b"data: \xc0\xaf",
        b"data: \xed\xa0\x80",
        b"data: \xf0\x9f",
    ] {
        for split in 0..=bytes.len() {
            let mut parser = Parser::new();
            let outcome = parser
                .push(&bytes[..split])
                .and_then(|_| parser.push(&bytes[split..]))
                .and_then(|_| parser.finish().map(|_| ()));
            assert_eq!(outcome, Err(InvalidUtf8), "{bytes:?} split {split}");
            assert_eq!(parser.push("data: valid\n\n"), Err(InvalidUtf8));
        }
    }
}

#[test]
fn a_bom_is_stripped_only_at_the_start_even_when_split() {
    let bytes = "\u{feff}data: first\r\ndata: \u{feff}kept\r\n\r\n".as_bytes();
    for split in 0..=bytes.len() {
        assert_eq!(
            parse(&[&bytes[..split], &bytes[split..]]),
            vec![Event {
                name: String::new(),
                data: "first\n\u{feff}kept".into()
            }]
        );
    }
}
