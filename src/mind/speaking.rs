//! The mind on the command line, in either encoding.
//!
//! Two ways in and the same answers out of both. A sibling with a Lua VM dials the socket and
//! gets JSON, because the family's stub cannot decode anything else. A sibling that would rather
//! spawn than dial runs this, and picks: JSON to read, CBOR to keep a signature byte-for-byte.
//!
//! The reply is the family's shape either way — `{"ok":true,"n":N,"result":[…]}` — because a
//! caller should not need a second parser to find out that a call was refused. A refusal is a
//! reply with `ok:false`, and the exit status stays zero: a non-zero exit is how a program says
//! it did not run, and melchior answering "no" is not that.

use crate::mind::catalog::Catalog;
use crate::mind::wire::{Ask, Said};
use std::io::Write;

/// Which encoding an answer goes out in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum As {
    /// Human-readable, and what a Lua sibling can decode.
    Json,
    /// Byte-exact, for a caller that is not going to read it.
    Cbor,
}

impl As {
    /// Which encoding the flags asked for. JSON unless CBOR was named.
    #[must_use]
    pub fn asked(flags: &std::collections::BTreeMap<String, String>) -> Self {
        if flags.contains_key("cbor") {
            Self::Cbor
        } else {
            Self::Json
        }
    }
}

/// Write one reply in the family's shape.
///
/// # Errors
/// When the answer will not encode, or stdout will not take it.
pub fn reply<T: serde::Serialize>(
    out: &mut impl Write,
    how: As,
    values: &[T],
) -> std::io::Result<()> {
    let body = serde_json::json!({
        "ok": true,
        "n": values.len(),
        "result": values.iter().map(|v| serde_json::to_value(v).unwrap_or(serde_json::Value::Null)).collect::<Vec<_>>(),
    });
    emit(out, how, &body)
}

/// Write one refusal, which is a reply and not an error.
///
/// # Errors
/// When stdout will not take it.
pub fn refuse(out: &mut impl Write, how: As, why: &str, fault: &str) -> std::io::Result<()> {
    let body = serde_json::json!({ "ok": false, "error": why, "fault": fault });
    emit(out, how, &body)
}

fn emit(out: &mut impl Write, how: As, body: &serde_json::Value) -> std::io::Result<()> {
    match how {
        As::Json => writeln!(out, "{body}"),
        As::Cbor => {
            let mut bytes = Vec::new();
            ciborium::into_writer(body, &mut bytes).map_err(std::io::Error::other)?;
            out.write_all(&bytes)
        }
    }
}

/// `melchior models` — what this machine could talk to.
///
/// # Errors
/// When the answer will not be written.
pub fn models(flags: &std::collections::BTreeMap<String, String>) -> std::io::Result<()> {
    let how = As::asked(flags);
    let mut out = std::io::stdout().lock();
    match Catalog::load(&Catalog::dir()) {
        Ok(catalog) => {
            let mut cards = catalog.cards();
            // Ready first. A list that opens with forty models needing keys buries the one that
            // would answer.
            cards.sort_by_key(|card| (!card.ready, card.id.clone()));
            if let Some(only) = flags.get("ready") {
                let _ = only;
                cards.retain(|card| card.ready);
            }
            reply(&mut out, how, &cards)
        }
        Err(why) => refuse(&mut out, how, &why.to_string(), "refused"),
    }
}

/// `melchior ask` — read an [`Ask`] and stream what the model says.
///
/// The request arrives on stdin, in the encoding named by the flags. Every [`Said`] is written
/// as it arrives — one line of JSON, or one CBOR value — so a caller sees the answer forming
/// rather than waiting for the whole of it.
///
/// # Errors
/// When the request will not decode, or the answer will not be written.
pub fn ask(flags: &std::collections::BTreeMap<String, String>) -> std::io::Result<()> {
    let how = As::asked(flags);
    let mut out = std::io::stdout().lock();

    let mut body = Vec::new();
    std::io::Read::read_to_end(&mut std::io::stdin().lock(), &mut body)?;
    let asked: Result<Ask, String> = match how {
        As::Json => serde_json::from_slice(&body).map_err(|e| e.to_string()),
        As::Cbor => ciborium::from_reader(body.as_slice()).map_err(|e| e.to_string()),
    };
    let Ok(asked) = asked else {
        let why = asked.err().unwrap_or_default();
        return refuse(
            &mut out,
            how,
            &format!("that is not an ask: {why}"),
            "refused",
        );
    };

    // Its own runtime, because the mind is the only part of melchior that does I/O of this
    // shape: a `serve` already has one, and a one-shot `ask` has none to borrow.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let mut wrote: std::io::Result<()> = Ok(());
    runtime.block_on(crate::mind::running::run(&asked, |said| {
        // Flushed per delta. A caller reading this to draw a screen wants the answer forming,
        // and a buffer that filled first would hand it over all at once at the end.
        if wrote.is_ok() {
            wrote = stream(&mut out, how, &said);
        }
    }));
    wrote
}

/// Write one [`Said`] as it happens.
///
/// # Errors
/// When stdout will not take it.
pub fn stream(out: &mut impl Write, how: As, said: &Said) -> std::io::Result<()> {
    match how {
        As::Json => {
            let line = serde_json::to_string(said).map_err(std::io::Error::other)?;
            writeln!(out, "{line}")?;
        }
        As::Cbor => {
            let mut bytes = Vec::new();
            ciborium::into_writer(said, &mut bytes).map_err(std::io::Error::other)?;
            out.write_all(&bytes)?;
        }
    }
    out.flush()
}

/// `melchior needs` — what this sibling wants to be told.
///
/// # Errors
/// When the answer will not be written.
pub fn needs(flags: &std::collections::BTreeMap<String, String>) -> std::io::Result<()> {
    let how = As::asked(flags);
    let mut out = std::io::stdout().lock();
    reply(&mut out, how, &crate::mind::setup::needs())
}

/// `melchior configure` — read config Lua on stdin and apply it.
///
/// # Errors
/// When the chunk cannot be read, or the answer cannot be written.
pub fn configure(flags: &std::collections::BTreeMap<String, String>) -> std::io::Result<()> {
    let how = As::asked(flags);
    let mut out = std::io::stdout().lock();

    if flags.contains_key("forget") {
        crate::mind::setup::forget();
        return reply(&mut out, how, &[crate::mind::wire::Applied::default()]);
    }

    let mut source = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin().lock(), &mut source)?;
    match crate::mind::setup::configure(&source) {
        Ok(applied) => reply(&mut out, how, &[applied]),
        // A chunk that will not run is a refusal, not a crash: the coordinator sent something,
        // and what it needs back is which part was wrong.
        Err(why) => refuse(&mut out, how, &why, "refused"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mind::wire::Card;

    fn flags(pairs: &[(&str, &str)]) -> std::collections::BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn json_is_the_default_and_cbor_is_asked_for() {
        assert_eq!(As::asked(&flags(&[])), As::Json);
        assert_eq!(As::asked(&flags(&[("cbor", "")])), As::Cbor);
    }

    #[test]
    fn a_reply_is_the_familys_shape() {
        let mut out = Vec::new();
        reply(&mut out, As::Json, &["a", "b"]).expect("write");
        let value: serde_json::Value = serde_json::from_slice(&out).expect("decode");
        assert_eq!(value["ok"], serde_json::json!(true));
        assert_eq!(value["n"], serde_json::json!(2));
        assert!(value["result"].is_array(), "result must be a list");
    }

    #[test]
    fn a_refusal_is_a_reply_and_says_which_kind() {
        let mut out = Vec::new();
        refuse(&mut out, As::Json, "no such model", "refused").expect("write");
        let value: serde_json::Value = serde_json::from_slice(&out).expect("decode");
        assert_eq!(value["ok"], serde_json::json!(false));
        assert_eq!(value["fault"], serde_json::json!("refused"));
    }

    #[test]
    fn the_same_answer_comes_out_of_both_encodings() {
        let card = Card {
            id: "p/m".into(),
            provider: "p".into(),
            name: "m".into(),
            api: "openai-completions".into(),
            context_window: Some(1),
            max_output: None,
            reasons: false,
            ready: true,
            needs: None,
        };
        let mut as_json = Vec::new();
        reply(&mut as_json, As::Json, std::slice::from_ref(&card)).expect("json");
        let mut as_cbor = Vec::new();
        reply(&mut as_cbor, As::Cbor, std::slice::from_ref(&card)).expect("cbor");

        let from_json: serde_json::Value = serde_json::from_slice(&as_json).expect("decode json");
        let from_cbor: serde_json::Value =
            ciborium::from_reader(as_cbor.as_slice()).expect("decode cbor");
        assert_eq!(from_json, from_cbor, "one shape, two encodings");
    }

    #[test]
    fn a_streamed_said_is_one_line_of_json() {
        let mut out = Vec::new();
        stream(&mut out, As::Json, &Said::Text { text: "hi".into() }).expect("write");
        let text = String::from_utf8(out).expect("utf8");
        assert_eq!(text.lines().count(), 1, "one said, one line: {text:?}");
        assert!(text.contains("\"event\""), "untagged: {text}");
    }
}
