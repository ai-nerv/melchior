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
            name: crate::mind::spending::SETTING.to_owned(),
            kind: Kind::Number,
            about: "how many US dollars one run may spend on models — the root session and every \
                    session started under it — before an ask is refused unsent; 0 or false for no \
                    cap. Priced as the provider reports, else at the model's own rates"
                .to_owned(),
            required: false,
            default: Some(serde_json::json!(crate::mind::spending::DEFAULT_USD)),
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
const SETTINGS: &[&str] = &[
    "model",
    "thinking",
    "max_tokens",
    "discover",
    "role",
    crate::mind::spending::SETTING,
];

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
    if let Some(given) = given().and_then(|path| read_given(&path)) {
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
    configure_into(given().as_deref(), source)
}

/// The same, with the path a parameter so a test can own one rather than share the live file.
/// With no path there is nowhere to keep it, and a chunk that would be kept is refused.
pub fn configure_into(path: Option<&std::path::Path>, source: &str) -> Result<Applied, String> {
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
        let Some(path) = path else {
            return Err(format!(
                "nowhere to keep it: a coordinator names this session's own directory in ${GIVEN}, \
                 and none is named"
            ));
        };
        remember(path, source)?;
    }
    Ok(applied)
}

/// Names the directory a coordinator keeps one session's configuration in.
pub const GIVEN: &str = "NERV_GIVEN";

/// Where configuration a coordinator sent this session is kept: `melchior.lua` in the directory
/// `$NERV_GIVEN` names, one per session. No session named, nothing is kept or read.
#[must_use]
pub fn given() -> Option<std::path::PathBuf> {
    given_from(std::env::var_os(GIVEN))
}

/// The same, from what the environment said.
#[must_use]
pub fn given_from(said: Option<std::ffi::OsString>) -> Option<std::path::PathBuf> {
    let dir = std::path::PathBuf::from(said.filter(|dir| !dir.is_empty())?);
    dir.is_absolute().then(|| dir.join("melchior.lua"))
}

/// The directory `$NERV_GIVEN` names, when it names one.
#[must_use]
pub fn given_dir() -> Option<std::path::PathBuf> {
    given().and_then(|file| file.parent().map(std::path::Path::to_path_buf))
}

/// What a coordinator said, read from a regular file and never through a link.
#[must_use]
pub fn read_given(path: &std::path::Path) -> Option<String> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits().cast_signed())
        .open(path)
        .ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut said = String::new();
    file.take(1 << 20).read_to_string(&mut said).ok()?;
    Some(said)
}

/// Keep `source` at `path`: its directory this user's and closed, the file `0600`, written beside
/// and renamed into place so a reader never meets half of it.
fn remember(path: &std::path::Path, source: &str) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(parent) = path.parent() {
        crate::places::ensure(parent)?;
    }
    let beside = path.with_extension(format!("lua.{}", std::process::id()));
    let kept = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&beside)
        .and_then(|mut file| file.write_all(source.as_bytes()))
        .and_then(|()| std::fs::rename(&beside, path));
    let _ = std::fs::remove_file(&beside);
    kept.map_err(|why| format!("{}: {why}", path.display()))
}

/// Forget what a coordinator said, so a restart reads melchior's own files again.
pub fn forget() {
    if let Some(path) = given() {
        forget_at(&path);
    }
}

/// The same, for a named file.
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
        let applied = configure_into(Some(&path), r#"melchior.thinking = "high""#).expect("runs");
        assert_eq!(applied.set, vec!["thinking".to_owned()], "{applied:?}");
        assert!(applied.whole());
        forget_at(&path);
    }

    #[test]
    fn a_setting_it_does_not_take_is_refused_by_name() {
        let path = mine("refused");
        let applied = configure_into(Some(&path), r#"melchior.colour = "green""#).expect("runs");
        assert!(!applied.whole(), "{applied:?}");
        assert_eq!(applied.refused[0].name, "colour");
        forget_at(&path);
    }

    #[test]
    fn a_chunk_that_will_not_run_is_an_error_rather_than_a_partial_apply() {
        // The file this call was given, not the shared one `given()` answers.
        let path = mine("broken");
        let why = configure_into(Some(&path), "this is not lua at all !!").expect_err("must fail");
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
            Some(&path),
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
        configure_into(Some(&path), r#"melchior.thinking = "low""#).expect("runs");
        let held = std::fs::read_to_string(&path).expect("kept");
        assert!(held.contains("thinking"), "{held}");
        forget_at(&path);
        assert!(!path.exists(), "forget must actually forget");
    }

    #[test]
    fn what_a_coordinator_said_is_kept_per_session_or_not_at_all() {
        let dir = std::path::PathBuf::from("/run/session-7");
        assert_eq!(
            given_from(Some(dir.clone().into_os_string())),
            Some(dir.join("melchior.lua"))
        );
        for unset in [None, Some("".into()), Some("relative/dir".into())] {
            assert_eq!(given_from(unset.clone()), None, "{unset:?}");
        }
        let why = configure_into(None, r#"melchior.thinking = "low""#)
            .expect_err("kept nowhere and said so");
        assert!(why.contains(GIVEN), "{why}");
    }

    #[test]
    fn what_is_kept_is_this_user_s_alone_and_never_read_through_a_link() {
        use std::os::unix::fs::MetadataExt;
        let path = mine("modes");
        configure_into(Some(&path), r#"melchior.thinking = "low""#).expect("runs");
        assert_eq!(
            std::fs::metadata(&path).expect("kept").mode() & 0o777,
            0o600
        );
        assert!(read_given(&path).is_some());

        let link = path.with_file_name("linked.lua");
        std::os::unix::fs::symlink(&*path, &link).expect("a link");
        assert_eq!(read_given(&link), None, "a link was followed");
    }
}
