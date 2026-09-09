//! Seven variables a session inherited from whoever started it, and one reader.

pub const PARENT: &str = "MAGI_MELCHIOR_PARENT";

pub const TOKEN: &str = "MAGI_MELCHIOR_TOKEN";

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
