//! What a session inherited from whoever started it.
//!
//! Six variables and one reader. They are here rather than in [`crate::directory`], where they
//! grew up, because [`crate::policy`] needs one of them and the directory needs policy — and a
//! module that reads an environment variable does not need to know that sessions have sockets.
//! That was the whole of one of the three cycles among these four modules.
//!
//! Inherited rather than passed as arguments on purpose. A child that re-execs, or starts a
//! shell that starts another session, still knows where it came from; an argument would survive
//! exactly one hop.

/// The variable a spawned instance learns its parent from.
///
/// Inherited across the spawn rather than passed as an argument, so a child that re-execs or
/// starts a shell that starts another session still knows where it came from.
pub const PARENT: &str = "MAGI_MELCHIOR_PARENT";

/// The variable carrying the secret that makes a `stop` honourable.
///
/// Minted by the parent, handed to the child, held by both and by nobody else.
pub const TOKEN: &str = "MAGI_MELCHIOR_TOKEN";

/// The three that tell a spawned process which session it belongs to.
///
/// Set by whatever started the session and inherited from there by everything it starts. It is
/// the one thing a separate process cannot work out for itself: a name is made when a session
/// starts, and nothing on disk says which of several a given process was spawned under.
pub const PROJECT: &str = "MAGI_MELCHIOR_PROJECT";
/// What that session is for, the middle part of its name.
pub const ROLE: &str = "MAGI_MELCHIOR_ROLE";
/// The last of the three, and the only one the socket is named after.
pub const ID: &str = "MAGI_MELCHIOR_ID";

/// How far this session may reach, as [`Talk`](crate::policy::Talk) names it.
///
/// A setting rather than a name, and it travels the same way the names do because it has to
/// reach the same two processes: the one holding the socket and the one a model calls. A harness
/// that set it on only one of them would have a tool refusing what the socket allows.
pub const TALK: &str = "MAGI_MELCHIOR_TALK";

/// What one of those says, if it says anything.
///
/// `MAGI_MELCHIOR_*` first, then the `MAGI_*` name the same variable grew up under. Both, because this
/// layer was lifted out of one harness and that harness is still setting the old names — and a
/// variable written under one name and read under another is a session that cannot find itself,
/// which presents as "nobody is running" rather than as a rename anybody would guess at.
pub(crate) fn said(name: &str) -> Option<String> {
    let older = format!("MAGI_{}", name.trim_start_matches("MAGI_MELCHIOR_"));
    std::env::var(name)
        .ok()
        .or_else(|| std::env::var(&older).ok())
        .filter(|value| !value.is_empty())
}
