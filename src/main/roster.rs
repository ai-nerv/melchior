//! Who came, who went, and whose phase moved, written to the log as the roster changes.

use super::Peer;

/// Log the difference between the roster that was and the one that is.
pub(super) fn noted(was: &[Peer], now: &[Peer]) {
    if !melchior::noted::enabled() {
        return;
    }
    for peer in now {
        match was.iter().find(|before| before.id == peer.id) {
            None => melchior::noted!(
                "roster: {} joined as {}{}",
                peer.id,
                peer.role,
                peer.parent
                    .as_ref()
                    .map(|parent| format!(" under {parent}"))
                    .unwrap_or_default()
            ),
            Some(before) if before.phase != peer.phase => melchior::noted!(
                "roster: {} is {} (was {})",
                peer.id,
                peer.phase.as_deref().unwrap_or("?"),
                before.phase.as_deref().unwrap_or("?")
            ),
            Some(_) => {}
        }
    }
    for gone in was.iter().filter(|p| !now.iter().any(|q| q.id == p.id)) {
        melchior::noted!("roster: {} left", gone.id);
    }
}
