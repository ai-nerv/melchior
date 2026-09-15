//! Being told how to behave, by whoever is coordinating: the same Lua, VM and registrars as
//! melchior's own `config/`, arriving down a socket instead of off a disk. [`needs`] is the list
//! a coordinator reads to know what to send, and anything else sent is refused by name rather
//! than ignored. What is asked per turn travels on an `Ask`; these are the standing answers.

use crate::mind::wire::{Applied, Kind, Need, Refused};

/// What melchior wants to be told. Everything here has a default and nothing is required.
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
        Need {
            name: "role".to_owned(),
            kind: Kind::Text,
            about: "what this session is for: a name, and on the lines after it a sentence a \
                    coordinator can route by. Outranked by `MAGI_MELCHIOR_ROLE` and by \
                    `melchior serve --role`, and the winner is taken whole — a name from one \
                    source and a description from another would describe a role nobody declared"
                .to_owned(),
            required: false,
            default: Some(serde_json::json!("main")),
        },
        Need {
            name: "agent_talk".to_owned(),
            kind: Kind::Text,
            about: "how far a session may reach: mains, instance, project. Set in the \
                    environment a session is spawned with, as `MAGI_MELCHIOR_TALK` — the \
                    socket and the tool are two processes, and a setting only one of them \
                    could see would leave a tool refusing what the socket allows"
                .to_owned(),
            required: false,
            default: Some(serde_json::json!("mains")),
        },
    ]
}

/// The settings a coordinator may set in a config chunk, beside the registrars.
///
/// Narrower than [`needs`]: a registrar reaches the VM by being called, and `agent_talk` reaches
/// [`crate::policy`] in the environment a session is spawned with, not by assignment here.
const SETTINGS: &[&str] = &["model", "thinking", "max_tokens", "discover", "role"];

/// What a person or a coordinator assigned to one setting, without building the model catalog:
/// binding a socket must not depend on the model layer loading cleanly, so nothing shipped is
/// read here and a file that raises costs only itself.
#[must_use]
pub fn assigned(name: &str) -> Option<serde_json::Value> {
    let dir = crate::mind::catalog::Catalog::dir();
    let mut engine = crate::mind::lua::engine::Engine::new();
    for (path, trust) in crate::mind::plugins::runtimepath(&crate::mind::plugins::Roots::at(&dir)) {
        if trust.needs_acknowledging() {
            continue;
        }
        if let Ok(source) = std::fs::read_to_string(&path) {
            let _ = engine.run(&source, &path.display().to_string());
        }
    }
    if let Ok(given) = std::fs::read_to_string(given()) {
        let _ = engine.run(&given, "given");
    }
    engine.harvest();
    engine.config().get(name).cloned()
}

/// Run a chunk of config Lua and say what it did.
///
/// The chunk runs in a VM of its own, so what it changes is what the *next* turn is built from.
/// A chunk that will not compile or raises is an error and applies nothing.
pub fn configure(source: &str) -> Result<Applied, String> {
    configure_into(&given(), source)
}

/// The same, with the path a parameter so a test can own one rather than share the live file.
pub fn configure_into(path: &std::path::Path, source: &str) -> Result<Applied, String> {
    let mut engine = crate::mind::lua::engine::Engine::new();
    // What the VM holds before the chunk runs; the difference is what the coordinator said.
    engine.harvest();
    let before = engine.config().settings.clone();

    engine
        .run(source, "configure")
        .map_err(|why| why.to_string())?;
    engine.harvest();

    let config = engine.config();
    let mut applied = Applied::default();
    for name in SETTINGS {
        // Changed, not merely present.
        if config.get(name).is_some() && config.get(name) != before.get(*name) {
            applied.set.push((*name).to_owned());
        }
    }
    // A registrar is a call rather than an assignment, so it is counted by what it left behind.
    let declared = config.all("provider").len();
    if declared > 0 {
        applied.set.push(format!("provider ({declared})"));
    }

    // Anything else the chunk set is refused by name.
    for name in config.settings.keys() {
        if !before.contains_key(name) && !SETTINGS.contains(&name.as_str()) {
            applied.refused.push(Refused {
                name: name.to_owned(),
                why: "melchior takes no setting by that name; `needs` lists what it takes"
                    .to_owned(),
            });
        }
    }
    drop(config);

    // Written where the next VM will read it, so it outlives this call.
    if applied.whole() {
        remember(path, source)?;
    }
    Ok(applied)
}

/// Where configuration sent over the wire is kept: the runtime directory rather than the config
/// directory, so it goes when the machine restarts.
#[must_use]
pub fn given() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("melchior").join("given.lua")
}

fn remember(path: &std::path::Path, source: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|why| why.to_string())?;
    }
    std::fs::write(path, source).map_err(|why| why.to_string())
}

/// Forget what a coordinator said, so a restart reads melchior's own files again.
pub fn forget() {
    forget_at(&given());
}

/// The same, for a named file: [`given`] is one path shared by every melchior this user runs, so
/// a test must forget its own rather than that one.
pub fn forget_at(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::{Scratch, ScratchFile};

    /// A place of this test's own, so tests running together do not delete each other's.
    fn mine(name: &str) -> ScratchFile {
        Scratch::file("melchior-setup", name, "given.lua")
    }

    #[test]
    fn everything_it_needs_has_a_default_or_is_optional() {
        for need in needs() {
            assert!(!need.required, "{} is required: {need:?}", need.name);
        }
    }

    #[test]
    fn what_it_declares_is_what_it_takes() {
        let declared: Vec<String> = needs().into_iter().map(|need| need.name).collect();
        for name in SETTINGS {
            assert!(declared.contains(&(*name).to_owned()), "{name} undeclared");
        }
    }

    #[test]
    fn the_setting_every_refusal_names_is_one_a_coordinator_can_find() {
        // `agent_talk` is named in every refusal it causes, so `needs` must name it and its
        // levels too.
        let declared = needs();
        let talk = declared
            .iter()
            .find(|need| need.name == "agent_talk")
            .expect("the setting every refusal names");
        for level in [crate::policy::Talk::Mains.named(), "instance", "project"] {
            assert!(talk.about.contains(level), "{level} is not named: {talk:?}");
        }
        assert!(
            talk.about.contains(crate::inherited::TALK),
            "and how to set it: {talk:?}"
        );
    }

    #[test]
    fn a_setting_it_takes_is_applied_and_named() {
        let path = mine("takes");
        let applied = configure_into(&path, r#"melchior.thinking = "high""#).expect("runs");
        assert_eq!(applied.set, vec!["thinking".to_owned()], "{applied:?}");
        assert!(applied.whole());
        forget_at(&path);
    }

    #[test]
    fn a_setting_it_does_not_take_is_refused_by_name() {
        let path = mine("refused");
        let applied = configure_into(&path, r#"melchior.colour = "green""#).expect("runs");
        assert!(!applied.whole(), "{applied:?}");
        assert_eq!(applied.refused[0].name, "colour");
        forget_at(&path);
    }

    #[test]
    fn a_chunk_that_will_not_run_is_an_error_rather_than_a_partial_apply() {
        // The file this call was given, not the shared one `given()` answers.
        let path = mine("broken");
        let why = configure_into(&path, "this is not lua at all !!").expect_err("must fail");
        assert!(!why.is_empty());
        assert!(
            !path.exists(),
            "a chunk that did not run must leave nothing behind"
        );
    }

    #[test]
    fn a_provider_declared_over_the_wire_is_counted() {
        let path = mine("provider");
        let applied = configure_into(
            &path,
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
        forget_at(&path);
    }

    #[test]
    fn what_was_given_outlives_the_call() {
        let path = mine("outlives");
        configure_into(&path, r#"melchior.thinking = "low""#).expect("runs");
        let held = std::fs::read_to_string(&path).expect("kept");
        assert!(held.contains("thinking"), "{held}");
        forget_at(&path);
        assert!(!path.exists(), "forget must actually forget");
    }

    #[test]
    fn forgetting_is_aimed_at_one_file_and_not_at_the_shared_one() {
        let path = mine("aimed");
        configure_into(&path, r#"melchior.thinking = "low""#).expect("runs");
        let live = given();
        let stood = live.exists();
        forget_at(&path);
        assert!(!path.exists(), "it did not forget what it was pointed at");
        assert_eq!(
            live.exists(),
            stood,
            "forgetting one file disturbed the shared one"
        );
    }
}
