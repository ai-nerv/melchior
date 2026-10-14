//! The roster: who is listening and what each says it is doing, gathered off the async loop; the
//! signals a change in it raises; and the log of who came, who went, and whose phase moved.

use super::{Heard, Peer, status};

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

/// Everyone listening in `project`, with each one's live status asked of its socket. Blocking — it
/// dials every peer — so it runs on the poller thread, never the async loop. `me` is the asker each
/// dial names. A peer that does not answer this time keeps what it said last, so a missed poll is
/// not read as a change of phase.
pub(super) fn around(
    me: &melchior::identity::Identity,
    known: &mut std::collections::BTreeMap<String, status::Status>,
) -> Vec<Peer> {
    let project = &me.project;
    // The claims once, mapped by the id of whoever holds each, rather than dialled per peer.
    let claims: std::collections::BTreeMap<String, String> =
        melchior::directory::claims::all(project)
            .into_iter()
            .map(|held| (id_of(&held.by), held.about))
            .collect();
    let listening = melchior::directory::listening(project);
    known.retain(|id, _| listening.contains(id));
    listening
        .into_iter()
        .map(|id| {
            let status = match status::status_of(project, &id, me) {
                Some(status) => {
                    known.insert(id.clone(), status.clone());
                    status
                }
                None => known.get(&id).cloned().unwrap_or_default(),
            };
            Peer {
                role: melchior::directory::roles::role_in(project, &id)
                    .map_or_else(|| melchior::directory::roles::MAIN.to_owned(), |it| it.name),
                ui: melchior::directory::screens::ui_in(project, &id)
                    .map(|at| at.display().to_string()),
                parent: melchior::directory::parent_note(project, &id),
                session: melchior::directory::sessions::session_in(project, &id),
                claim: claims.get(&id).cloned(),
                busy: status.busy,
                working_for: status.working_for,
                waiting: status.waiting,
                phase: status.phase,
                cause: status.cause,
                spent: status.spent,
                id,
            }
        })
        .collect()
}

/// The id out of a `project/role/id`, or the whole of a bare one.
fn id_of(full: &str) -> String {
    full.rsplit('/').next().unwrap_or(full).to_owned()
}

/// A [`Heard::Signal`] for each watched agent whose phase changed since the last roster. Watched is
/// this session's children (peers whose parent is `me`) and its parent — the mechanical edges a
/// coordinator reacts to. Only a real change fires, a peer that reports no phase never does, and
/// one that vanishes mid-work is signalled as `lost`.
pub(super) fn signal_changes(
    me: &str,
    my_parent: Option<&str>,
    watched: &std::collections::BTreeSet<String>,
    was: &[Peer],
    now: &[Peer],
) -> Vec<Heard> {
    let kin_of = |peer: &Peer| {
        if peer.parent.as_deref() == Some(me) {
            Some("child")
        } else if Some(peer.id.as_str()) == my_parent {
            Some("parent")
        } else {
            watched.contains(&peer.id).then_some("watched")
        }
    };
    let signal = |peer: &Peer, kin: &str, kind: &str| Heard::Signal {
        from: peer.id.clone(),
        kind: kind.to_owned(),
        kin: kin.to_owned(),
        cause: peer.cause.clone(),
    };
    let mut out = Vec::new();
    for peer in now {
        let (Some(kin), Some(phase)) = (kin_of(peer), peer.phase.as_deref()) else {
            continue;
        };
        let before = was.iter().find(|p| p.id == peer.id);
        if before.and_then(|p| p.phase.as_deref()) != Some(phase) {
            out.push(signal(peer, kin, phase));
        }
    }
    // One that left the roster mid-work was killed, and would be waited on for ever.
    for left in was.iter().filter(|p| !now.iter().any(|q| q.id == p.id)) {
        if let Some(kin) = kin_of(left)
            && !matches!(left.phase.as_deref(), Some("gone" | "finished"))
        {
            out.push(signal(left, kin, "lost"));
        }
    }
    out
}
