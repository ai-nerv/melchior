//! A person here answered a request to be taken on: checked again against the tree as it stands
//! now, then handed over to the asker with the nonce it left, and only once the asker takes it is
//! the note written that makes it this session's child.
//!
//! Blocking — it dials the asker — so it runs off the async loop.

use melchior::answering::About;
use melchior::directory::{self, pending};
use melchior::identity::Identity;
use melchior::wire::Request;

/// Settle one answered request, and tell the asker either way.
pub(super) fn answered(
    me: &Identity,
    request: &Request,
    accept: bool,
    handover: Option<&str>,
    about: &tokio::sync::watch::Sender<About>,
) {
    let Some(them) = Identity::read(&request.from) else {
        return;
    };
    // Withdrawn first, so two sessions taking each other on at once cannot both succeed.
    pending::withdraw(me, &them.id);
    let outcome = if accept {
        took(me, &them, request, handover, about)
    } else {
        Err("it declined".to_owned())
    };
    let said = match &outcome {
        Ok(()) => format!("`{}` accepted: it is this session's parent now.", me.full()),
        Err(why) => format!("`{}` did not take this session on: {why}.", me.full()),
    };
    if let Err(why) = &outcome {
        melchior::noted!("serve: {} was not taken on: {why}", request.from);
    }
    directory::answer_request(&them, me, &said);
}

/// The checks, the handover and the note, in that order.
fn took(
    me: &Identity,
    them: &Identity,
    request: &Request,
    handover: Option<&str>,
    about: &tokio::sync::watch::Sender<About>,
) -> Result<(), String> {
    if directory::sessions::session_in(&them.project, &them.id) != request.session {
        return Err(format!(
            "`{}` is not the session that asked: its run has changed",
            them.id
        ));
    }
    pending::adoptable(me, them, melchior::answering::coming(&about.borrow()))?;
    let secret = melchior::identity::secret().map_err(|why| why.to_string())?;
    directory::hand_over(them, me, handover, &secret, &request.nonce)?;
    directory::adopted(them, &me.id)?;
    about.send_modify(|about| {
        about.minted.insert(them.id.clone(), secret);
    });
    Ok(())
}
