//! Being told how to behave, by whoever is coordinating.
//!
//! melchior runs alone perfectly well and reads its own `config/` when it does. Under a
//! coordinator it should not: magi decides, and two configurations that have to agree are one
//! that will not. So this is the other way in — the same Lua, the same VM, the same registrars,
//! arriving down a socket instead of off a disk.
//!
//! **What it takes is declared, not guessed.** [`needs`] is the list a coordinator reads to
//! know what to send; anything else sent is refused by name rather than ignored, because a
//! setting that silently does nothing is the worst kind of typo.
//!
//! What is *asked* of melchior per turn — which model, how much thinking, how many tokens —
//! travels on an `Ask` and is not configuration. These are the standing answers: what to do
//! when a turn does not say.

use crate::mind::wire::{Applied, Kind, Need, Refused};

/// What melchior wants to be told.
///
/// Short on purpose. A coordinator that had to fill in twenty fields before starting one would
/// be a coordinator nobody uses, so everything here has a default and nothing is required.
#[must_use]
pub fn needs() -> Vec<Need> {
    vec![
        Need {
            name: "model".to_owned(),
            kind: Kind::Text,
            about: "which model to use when an ask does not name one, as `provider/model`"
                .to_owned(),
            required: false,
            default: None,
        },
        Need {
            name: "thinking".to_owned(),
            kind: Kind::Text,
            about: "how much reasoning to ask for: off, low, medium, high".to_owned(),
            required: false,
            default: Some(serde_json::json!("off")),
        },
        Need {
            name: "max_tokens".to_owned(),
            kind: Kind::Number,
            about: "cap an answer below the model's own maximum".to_owned(),
            required: false,
            default: None,
        },
        Need {
            name: "discover".to_owned(),
            kind: Kind::Flag,
            about: "ask providers what they offer, rather than only what is written down"
                .to_owned(),
            required: false,
            default: Some(serde_json::json!(true)),
        },
        Need {
            name: "provider".to_owned(),
            kind: Kind::Table,
            about: "a provider declaration, as `melchior.provider(id, { … })`".to_owned(),
            required: false,
            default: None,
        },
    ]
}

/// The settings a coordinator may set, beside the registrars.
///
/// A registrar is named in [`needs`] too, but reaches the VM by being called rather than
/// assigned, so it is not in this list.
const SETTINGS: &[&str] = &["model", "thinking", "max_tokens", "discover"];

/// Run a chunk of config Lua and say what it did.
///
/// The chunk is run in a VM of its own rather than the one a turn will use: a turn builds its
/// VM when it runs, from the configuration standing at that moment, so what this changes is
/// what the *next* turn is built from. Running it into a live VM would configure a turn already
/// in flight, which is a turn nobody asked for.
///
/// # Errors
/// When the chunk will not compile or raises. Fatal rather than partial: a description that did
/// not run has expressed no intention, and applying half of one is worse than applying none.
pub fn configure(source: &str) -> Result<Applied, String> {
    let mut engine = crate::mind::lua::engine::Engine::new();
    // What the VM holds before the chunk runs. The module carries entries of its own — `ui`,
    // `self` — and a hardcoded list of those would be a list to keep in step with the VM. The
    // *difference* is what the coordinator said, whatever the VM happens to install.
    engine.harvest();
    let before: std::collections::BTreeSet<String> =
        engine.config().settings.keys().cloned().collect();

    engine
        .run(source, "configure")
        .map_err(|why| why.to_string())?;
    engine.harvest();

    let config = engine.config();
    let mut applied = Applied::default();
    for name in SETTINGS {
        if config.get(name).is_some() {
            applied.set.push((*name).to_owned());
        }
    }
    // A registrar is a call rather than an assignment, so it is counted by what it left behind.
    let declared = config.all("provider").len();
    if declared > 0 {
        applied.set.push(format!("provider ({declared})"));
    }

    // Anything else the chunk set is refused by name. Silence here is how a coordinator's typo
    // becomes an afternoon: the setting reads as accepted and does nothing for the rest of the
    // session.
    for name in config.settings.keys() {
        if !before.contains(name) && !SETTINGS.contains(&name.as_str()) {
            applied.refused.push(Refused {
                name: name.to_owned(),
                why: "melchior takes no setting by that name; `needs` lists what it takes"
                    .to_owned(),
            });
        }
    }
    drop(config);

    // Written where the next VM will read it, so it outlives this call. A setting held only in
    // memory would be lost the moment a turn built its own VM, which is every turn.
    if applied.whole() {
        remember(source)?;
    }
    Ok(applied)
}

/// Where configuration sent over the wire is kept.
///
/// A file, in the runtime directory rather than the config directory: this is what a coordinator
/// said for as long as it is running, not something a person edits or should find later. It goes
/// when the machine restarts, which is exactly the lifetime a coordinator's instructions have.
#[must_use]
pub fn given() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("melchior").join("given.lua")
}

fn remember(source: &str) -> Result<(), String> {
    let path = given();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|why| why.to_string())?;
    }
    std::fs::write(&path, source).map_err(|why| why.to_string())
}

/// Forget what a coordinator said.
///
/// So a melchior restarted without one reads its own files again rather than yesterday's
/// instructions.
pub fn forget() {
    let _ = std::fs::remove_file(given());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everything_it_needs_has_a_default_or_is_optional() {
        // A coordinator must be able to start one by saying nothing at all.
        for need in needs() {
            assert!(!need.required, "{} is required: {need:?}", need.name);
        }
    }

    #[test]
    fn what_it_declares_is_what_it_takes() {
        // The list a coordinator reads and the list `configure` accepts are the same list, or
        // the declaration is a lie a coordinator finds out about one setting at a time.
        let declared: Vec<String> = needs().into_iter().map(|need| need.name).collect();
        for name in SETTINGS {
            assert!(declared.contains(&(*name).to_owned()), "{name} undeclared");
        }
    }

    #[test]
    fn a_setting_it_takes_is_applied_and_named() {
        forget();
        let applied = configure(r#"melchior.thinking = "high""#).expect("runs");
        assert_eq!(applied.set, vec!["thinking".to_owned()], "{applied:?}");
        assert!(applied.whole());
        forget();
    }

    #[test]
    fn a_setting_it_does_not_take_is_refused_by_name() {
        forget();
        let applied = configure(r#"melchior.colour = "green""#).expect("runs");
        assert!(!applied.whole(), "{applied:?}");
        assert_eq!(applied.refused[0].name, "colour");
        forget();
    }

    #[test]
    fn a_chunk_that_will_not_run_is_an_error_rather_than_a_partial_apply() {
        forget();
        let why = configure("this is not lua at all !!").expect_err("must fail");
        assert!(!why.is_empty());
        assert!(
            !given().exists(),
            "a chunk that did not run must leave nothing behind"
        );
    }

    #[test]
    fn a_provider_declared_over_the_wire_is_counted() {
        forget();
        let applied = configure(
            r#"melchior.provider("wired", {
                 name = "Wired",
                 api = "openai-completions",
                 auth = { kind = "none" },
                 models = { { id = "m", name = "M", context_window = 8000, max_tokens = 900 } },
               })"#,
        )
        .expect("runs");
        assert!(
            applied.set.iter().any(|s| s.starts_with("provider")),
            "{applied:?}"
        );
        forget();
    }

    #[test]
    fn what_was_given_outlives_the_call() {
        forget();
        configure(r#"melchior.thinking = "low""#).expect("runs");
        let held = std::fs::read_to_string(given()).expect("kept");
        assert!(held.contains("thinking"), "{held}");
        forget();
        assert!(!given().exists(), "forget must actually forget");
    }
}
