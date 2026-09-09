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
/// Narrower than [`needs`], and on purpose: two things it declares do not arrive by assignment.
/// A registrar reaches the VM by being called, and `agent_talk` reaches [`crate::policy`] in the
/// environment a session is spawned with — a chunk could only set it for the process running the
/// chunk, which is not the process holding the socket.
const SETTINGS: &[&str] = &["model", "thinking", "max_tokens", "discover", "role"];

/// What a config assigned to one setting, without building the model catalog.
///
/// [`crate::mind::catalog::Catalog::load`] is the full read and would answer this too, at the
/// cost of running eight hundred lines of shipped provider descriptions and everything installed
/// beside them. Binding a socket must not depend on the model layer loading cleanly: melchior
/// answers for its session whether or not a provider file compiles, and a role that could not be
/// read because somebody's `apis.lua` raised would be a session with no word for what it does.
///
/// So: what a person or a coordinator declared, and nothing shipped. A file that raises costs
/// itself and nothing else, for the same reason it does in the catalog — it is somebody else's
/// package, and refusing to start over it would make installing one a risk rather than a try.
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
/// The chunk is run in a VM of its own rather than the one a turn will use: a turn builds its
/// VM when it runs, from the configuration standing at that moment, so what this changes is
/// what the *next* turn is built from. Running it into a live VM would configure a turn already
/// in flight, which is a turn nobody asked for.
///
/// # Errors
/// When the chunk will not compile or raises. Fatal rather than partial: a description that did
/// not run has expressed no intention, and applying half of one is worse than applying none.
pub fn configure(source: &str) -> Result<Applied, String> {
    configure_into(&given(), source)
}

/// The same, kept somewhere named.
///
/// The path is a parameter so a test can own one. It was process-wide, and tests running
/// together deleted each other's file — the same shape as two coordinators sharing a melchior,
/// which is a thing neither should do.
///
/// # Errors
/// As [`configure`].
pub fn configure_into(path: &std::path::Path, source: &str) -> Result<Applied, String> {
    let mut engine = crate::mind::lua::engine::Engine::new();
    // What the VM holds before the chunk runs. The module carries entries of its own — `ui`,
    // `self` — and a hardcoded list of those would be a list to keep in step with the VM. The
    // *difference* is what the coordinator said, whatever the VM happens to install.
    engine.harvest();
    let before = engine.config().settings.clone();

    engine
        .run(source, "configure")
        .map_err(|why| why.to_string())?;
    engine.harvest();

    let config = engine.config();
    let mut applied = Applied::default();
    for name in SETTINGS {
        // Changed, not merely present. A VM that installs a setting of its own would otherwise
        // report it as something the coordinator said.
        if config.get(name).is_some() && config.get(name) != before.get(*name) {
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
        if !before.contains_key(name) && !SETTINGS.contains(&name.as_str()) {
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
        remember(path, source)?;
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

fn remember(path: &std::path::Path, source: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|why| why.to_string())?;
    }
    std::fs::write(path, source).map_err(|why| why.to_string())
}

/// Forget what a coordinator said.
///
/// So a melchior restarted without one reads its own files again rather than yesterday's
/// instructions.
pub fn forget() {
    forget_at(&given());
}

/// The same, for a named file.
///
/// The other half of [`configure_into`], and for the same reason: [`given`] is one path shared by
/// every melchior this user runs, so a test that forgot *it* would delete the settings of
/// whatever session happens to be running -- and two tests doing it would delete each other's.
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
    fn the_setting_every_refusal_names_is_one_a_coordinator_can_find() {
        // `agent_talk` decides who may reach whom and is named by name in every refusal it
        // causes, and it was declared nowhere — so a coordinator reading `needs` could see the
        // refusal, read the setting out of it, and still have no way to know it was melchior's
        // to set or what the levels were called.
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
        // The file this call was given, not the shared one. Asking after `given()` here was
        // asking whether *some other* melchior had been configured: it passed while nothing
        // else on the machine had run, and failed the moment a real session did.
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
        // What the bug was. Every one of these tests called the no-argument `forget`, which
        // deletes the path a *running* melchior reads -- so the suite quietly took the
        // settings out from under whatever session the person had open.
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
