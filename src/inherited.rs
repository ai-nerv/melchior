//! Six variables naming the session that started a process, and one reader; none is a secret.

pub const PARENT: &str = "MAGI_MELCHIOR_PARENT";

pub const PROJECT: &str = "MAGI_MELCHIOR_PROJECT";
pub const ROLE: &str = "MAGI_MELCHIOR_ROLE";
pub const ID: &str = "MAGI_MELCHIOR_ID";

/// The id of the root that started the run, inherited unchanged however deep the tree gets.
pub const SESSION: &str = "MAGI_MELCHIOR_SESSION";

pub const TALK: &str = "MAGI_MELCHIOR_TALK";

/// Read under the `MAGI_MELCHIOR_*` name, then the `MAGI_*` name a lifted harness still sets.
pub(crate) fn said(name: &str) -> Option<String> {
    let older = format!("MAGI_{}", name.trim_start_matches("MAGI_MELCHIOR_"));
    std::env::var(name)
        .ok()
        .or_else(|| std::env::var(&older).ok())
        .filter(|value| !value.is_empty())
}

/// Whether a variable is one FAMILY.md lists as carrying capability, or a credential: the family's
/// own prefixes, and anything named as a key, token or secret.
#[must_use]
pub fn carries_capability(name: &str) -> bool {
    const FAMILY: &[&str] = &["MAGI_", "CASPER_", "BALTHASAR_", "MELCHIOR_", "NERV_"];
    const CREDENTIAL: &[&str] = &["_API_KEY", "_KEY", "_TOKEN", "_SECRET"];
    FAMILY.iter().any(|prefix| name.starts_with(prefix))
        || CREDENTIAL.iter().any(|suffix| name.ends_with(suffix))
}

/// Take every such variable out of what `command` will start with, from this process's
/// environment and from anything set on the command: for a program that is not of the family.
pub fn scrub(command: &mut std::process::Command) {
    let named: Vec<std::ffi::OsString> = std::env::vars_os()
        .map(|(name, _)| name)
        .chain(command.get_envs().map(|(name, _)| name.to_owned()))
        .filter(|name| name.to_str().is_some_and(carries_capability))
        .collect();
    for name in named {
        command.env_remove(name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_program_outside_the_family_is_started_with_no_capability() {
        let mut command = std::process::Command::new("true");
        command
            .env(ID, "iota-mu")
            .env("NERV_GIVEN", "/run/given")
            .env("OPENAI_API_KEY", "sk-secret")
            .env("GITHUB_TOKEN", "ghp")
            .env("DISPLAY", ":0");
        scrub(&mut command);
        let left: Vec<(String, Option<String>)> = command
            .get_envs()
            .map(|(name, value)| {
                (
                    name.to_string_lossy().into_owned(),
                    value.map(|value| value.to_string_lossy().into_owned()),
                )
            })
            .collect();
        for gone in [ID, "NERV_GIVEN", "OPENAI_API_KEY", "GITHUB_TOKEN"] {
            assert!(
                left.iter()
                    .any(|(name, value)| name == gone && value.is_none()),
                "{gone} was handed on: {left:?}"
            );
        }
        assert!(left.contains(&("DISPLAY".to_owned(), Some(":0".to_owned()))));
    }
}
