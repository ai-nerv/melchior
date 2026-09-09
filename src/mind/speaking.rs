//! The mind on the command line, in either encoding: JSON to read, CBOR to keep a signature
//! byte-for-byte.
//!
//! The reply is the family's shape either way — `{"ok":true,"n":N,"result":[…]}`. A refusal is a
//! reply with `ok:false` and the exit status stays zero, a non-zero exit being reserved for
//! melchior not running at all.

use crate::mind::catalog::Catalog;
use crate::mind::wire::{Ask, Said};
use crate::wire::{Fault, Reply};
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
pub fn reply<T: serde::Serialize>(
    out: &mut impl Write,
    how: As,
    values: &[T],
) -> std::io::Result<()> {
    let rows = values
        .iter()
        .map(|v| serde_json::to_value(v).unwrap_or(serde_json::Value::Null))
        .collect();
    emit(out, how, &Reply::rows(rows))
}

/// Write one refusal, which is a reply and not an error.
pub fn refuse(out: &mut impl Write, how: As, why: &str, fault: Fault) -> std::io::Result<()> {
    emit(out, how, &Reply::no(why, fault))
}

/// The one place a command-line answer becomes bytes, so both doors carry the same [`Reply`].
fn emit(out: &mut impl Write, how: As, body: &Reply) -> std::io::Result<()> {
    match how {
        As::Json => {
            let line = serde_json::to_string(body).map_err(std::io::Error::other)?;
            writeln!(out, "{line}")
        }
        As::Cbor => {
            let mut bytes = Vec::new();
            ciborium::into_writer(body, &mut bytes).map_err(std::io::Error::other)?;
            out.write_all(&bytes)
        }
    }
}

/// `melchior models` — what this machine could talk to.
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
        Err(why) => refuse(&mut out, how, &why.to_string(), Fault::Refused),
    }
}

/// `melchior ask` — read an [`Ask`] and stream what the model says.
///
/// The request arrives on stdin, in the encoding named by the flags. Every [`Said`] is written
/// as it arrives — one line of JSON, or one CBOR value — so a caller sees the answer forming
/// rather than waiting for the whole of it.
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
            Fault::Refused,
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
pub fn needs(flags: &std::collections::BTreeMap<String, String>) -> std::io::Result<()> {
    let how = As::asked(flags);
    let mut out = std::io::stdout().lock();
    reply(&mut out, how, &crate::mind::setup::needs())
}

/// `melchior acknowledge` — clear the packages under `site/pack/`, so their declarations may run
/// until the file they were cleared for changes.
pub fn acknowledge(flags: &std::collections::BTreeMap<String, String>) -> std::io::Result<()> {
    let how = As::asked(flags);
    let dir = crate::mind::catalog::Catalog::dir();
    let files: Vec<(std::path::PathBuf, String)> =
        crate::mind::plugins::runtimepath(&crate::mind::plugins::Roots::at(&dir))
            .into_iter()
            .filter(|(_, trust)| trust.needs_acknowledging())
            .filter_map(|(path, _)| {
                std::fs::read_to_string(&path)
                    .ok()
                    .map(|source| (path, source))
            })
            .collect();

    let manifest = crate::mind::acknowledged::manifest_in(&dir);
    let mut out = std::io::stdout().lock();
    match crate::mind::acknowledged::acknowledge(&manifest, &files) {
        Ok(_) => reply(
            &mut out,
            how,
            &files
                .iter()
                .map(|(path, _)| serde_json::json!({ "acknowledged": path.display().to_string() }))
                .collect::<Vec<_>>(),
        ),
        Err(why) => refuse(&mut out, how, &why, Fault::Refused),
    }
}
/// `melchior verbs` — what this instance answers.
///
/// In the family reply shape, so another program can parse the self-description.
pub fn verbs(flags: &std::collections::BTreeMap<String, String>) -> std::io::Result<()> {
    let how = As::asked(flags);
    let mut out = std::io::stdout().lock();
    // The registrar surface rides on the self-description and nowhere else.
    let mut body = Reply::rows(doors());
    body.surface = Some(crate::wire::SURFACE);
    emit(&mut out, how, &body)
}

/// Every verb this program answers, one row per verb per door. Three tables, not one: a verb
/// reachable on two doors is two rows, because what a caller needs is where to knock and a
/// deduplicated name says nothing about that.
fn doors() -> Vec<serde_json::Value> {
    crate::wire::CLI_VERBS
        .iter()
        .map(|(verb, about)| serde_json::json!({ "verb": verb, "about": about, "door": "cli" }))
        .chain(crate::wire::VERBS.iter().map(
            |(verb, about)| serde_json::json!({ "verb": verb, "about": about, "door": "socket" }),
        ))
        .chain(crate::verbs::VERBS.iter().map(
            |(verb, about)| serde_json::json!({ "verb": verb, "about": about, "door": "tool" }),
        ))
        .collect()
}

/// `melchior configure` — read config Lua on stdin and apply it.
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
        Err(why) => refuse(&mut out, how, &why, Fault::Refused),
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
        refuse(&mut out, As::Json, "no such model", Fault::Refused).expect("write");
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

#[cfg(test)]
mod door_tests {
    use super::*;

    fn rows() -> Vec<serde_json::Value> {
        doors()
    }

    fn on(door: &str) -> Vec<String> {
        rows()
            .into_iter()
            .filter(|row| row["door"] == serde_json::json!(door))
            .filter_map(|row| row["verb"].as_str().map(ToOwned::to_owned))
            .collect()
    }

    #[test]
    fn every_row_names_one_of_the_three_doors_and_says_what_it_does() {
        for row in rows() {
            let door = row["door"].as_str().expect("a door");
            assert!(
                ["cli", "socket", "tool"].contains(&door),
                "`{door}` is not a door FAMILY.md knows: {row}"
            );
            assert!(row["verb"].as_str().is_some_and(|v| !v.is_empty()), "{row}");
            assert!(
                row["about"].as_str().is_some_and(|v| !v.is_empty()),
                "{row}"
            );
        }
    }

    #[test]
    fn the_tool_door_is_advertised_whole() {
        // The eleven coordination verbs were dispatched and named on no door at all.
        let listed = on("tool");
        for (verb, _) in crate::verbs::VERBS {
            assert!(
                listed.iter().any(|named| named == verb),
                "`{verb}` is answered by the tool and advertised nowhere: {listed:?}"
            );
        }
        assert_eq!(listed.len(), crate::verbs::VERBS.len());
    }

    #[test]
    fn each_door_advertises_its_own_table_and_nothing_is_deduplicated_away() {
        assert_eq!(on("cli").len(), crate::wire::CLI_VERBS.len());
        assert_eq!(on("socket").len(), crate::wire::VERBS.len());
        // The verbs on more than one door: each is listed once per door it is actually on.
        for verb in ["verbs", "needs", "inbox", "role", "stop", "status", "adopt"] {
            let doors: Vec<&str> = ["cli", "socket", "tool"]
                .into_iter()
                .filter(|door| on(door).iter().any(|named| named == verb))
                .collect();
            assert!(
                doors.len() > 1,
                "`{verb}` is on one door only, so this test is watching the wrong verb"
            );
        }
    }

    #[test]
    fn a_verb_a_door_does_not_answer_is_not_listed_on_it() {
        // The tool's own vocabulary is not reachable by `melchior crew` and never claims to be.
        for verb in ["crew", "claim", "claims", "release", "whoami", "announce"] {
            assert!(!on("cli").contains(&(*verb).to_owned()), "{verb}");
            assert!(!on("socket").contains(&(*verb).to_owned()), "{verb}");
            assert!(on("tool").contains(&(*verb).to_owned()), "{verb}");
        }
    }
}

#[cfg(test)]
mod family_tests {
    use super::*;

    #[test]
    fn a_one_shot_reply_says_which_wire_it_is() {
        // A missing `family` is taken for a peer older than the version check.
        let mut out = Vec::new();
        reply(&mut out, As::Json, &["a"]).expect("write");
        let value: serde_json::Value = serde_json::from_slice(&out).expect("decode");
        assert_eq!(value["family"], serde_json::json!(crate::wire::FAMILY));

        let mut refused = Vec::new();
        refuse(&mut refused, As::Json, "no", Fault::Refused).expect("write");
        let value: serde_json::Value = serde_json::from_slice(&refused).expect("decode");
        assert_eq!(
            value["family"],
            serde_json::json!(crate::wire::FAMILY),
            "a refusal too"
        );
    }

    #[test]
    fn the_version_rides_on_both_encodings() {
        let (mut json, mut cbor) = (Vec::new(), Vec::new());
        reply(&mut json, As::Json, &["a"]).expect("json");
        reply(&mut cbor, As::Cbor, &["a"]).expect("cbor");
        let from_json: serde_json::Value = serde_json::from_slice(&json).expect("json");
        let from_cbor: serde_json::Value = ciborium::from_reader(cbor.as_slice()).expect("cbor");
        assert_eq!(from_json, from_cbor, "one shape, two encodings");
        assert_eq!(from_cbor["family"], serde_json::json!(crate::wire::FAMILY));
    }
}
