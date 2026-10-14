//! Being taken on, both halves. `adopt` arrives at the session asked to become a parent and puts
//! the question to a person; `adopted` arrives back at the asker once somebody said yes, and is
//! believed only when it quotes the nonce the asker left in its own `.adopting` note.

use super::{About, Then, text_at};
use crate::directory::pending;
use crate::identity::Identity;
use crate::policy::Whom;
use crate::wire::{Call, Reply};

/// Put `caller`'s request to be taken on in front of the person here, bound to the nonce it sent
/// and to the run it is in now.
pub(super) fn adopt(call: &Call, about: &About, caller: &Whom) -> (Reply, Then) {
    let them = Identity {
        project: caller.project.clone(),
        role: crate::directory::roles::MAIN.to_owned(),
        id: caller.id.clone(),
    };
    if let Err(why) = pending::adoptable(&about.me, &them, 0) {
        return (Reply::refused(why), Then::Nothing);
    }
    let Some(nonce) = text_at(call, 1).filter(|nonce| !nonce.is_empty()) else {
        return (
            Reply::refused(
                "an adoption is asked with the nonce the asker left for the answer to quote",
            ),
            Then::Nothing,
        );
    };
    let from = Identity {
        role: crate::directory::roles::role_in(&them.project, &them.id).map_or_else(
            || crate::directory::roles::MAIN.to_owned(),
            |role| role.name,
        ),
        ..them
    }
    .full();
    let mut request = crate::wire::Request::made(&from, &text_at(call, 0).unwrap_or_default());
    request.nonce = nonce;
    request.session = crate::directory::sessions::session_in(&caller.project, &caller.id);
    (
        Reply::of(serde_json::json!({
            "asked": request.id,
            "of": about.me.full(),
        })),
        Then::Ask(request),
    )
}

/// The answer to an adoption this session asked for: `[by, handover, secret, nonce]`. Taken once,
/// from the session it asked and with the nonce it left, and checked again against the tree.
pub(super) fn adopted(call: &Call, about: &About) -> (Reply, Then) {
    let refused = |why: &str| (Reply::refused(why), Then::Nothing);
    let Some(by) = text_at(call, 0).and_then(|named| Identity::read(&named)) else {
        return refused("`adopted` names who took this session on, first");
    };
    let Some(nonce) = text_at(call, 3) else {
        return refused("`adopted` quotes the nonce the asking left, fourth");
    };
    if by.project != about.me.project || !pending::consume(&about.me, &by.id, &nonce) {
        return refused("this session asked nobody that for an adoption it could quote");
    }
    if let Err(why) = pending::adoptable(&by, &about.me, 0) {
        return refused(&why);
    }
    (
        Reply::done(),
        Then::Adopted {
            by: by.full(),
            handover: text_at(call, 1),
            secret: text_at(call, 2),
        },
    )
}
