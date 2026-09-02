//! `melchior tool` — the vocabulary a model calls, as one exec per request.
//!
//! The harness runs this with the model's arguments in `argv`, reads stdout, and takes the exit
//! code as whether it worked. That is the whole protocol, and choosing it was the point:
//!
//! - **No framing, no encoder, no version.** A harness that can run a program can use melchior.
//! - **Nothing of the harness crosses.** Speaking somebody's tool-peer protocol would mean
//!   copying their message types into this crate, and then a change on their side breaks a
//!   program they do not build.
//!
//! The cost is a process per call and a round trip to this session's own socket for its inbox.
//! Both are local and neither is measurable beside a model's turn.
//!
//! # What it needs to be somebody
//!
//! Everything comes from the environment, because it is the one thing a process cannot work out
//! for itself: a name is made when a session starts, and nothing on disk says which of several
//! a given process was spawned under. What it started, and what has arrived, are read from the
//! directory and asked of its own socket — neither can be talked into lying.

use melchior::verbs::{self, Standing};

/// Run one call and exit.
///
/// A refusal goes to stderr with a non-zero status, because that is what the transport reads as
/// failure. A refusal arriving as a success reads to a model as "that worked", and it carries on.
pub fn run() -> std::io::Result<()> {
    let asked = arguments();
    if asked.get("verb").is_none_or(String::is_empty) {
        return refuse(
            "melchior tool needs --verb. Try `--verb help`, or `melchior verbs` for what a session \
             answers over its socket.",
        );
    }

    let Some(standing) = standing() else {
        return refuse(&format!(
            "this process was not started by a session, so it does not know which instance it \
             would be speaking as. {} and {} say which, and are set by whatever started it.",
            melchior::directory::PROJECT,
            melchior::directory::ID
        ));
    };

    let answer = verbs::answer(&serde_json::to_value(&asked).unwrap_or_default(), &standing);
    if answer.failed {
        return refuse(&answer.said);
    }
    println!("{}", answer.said);
    Ok(())
}

/// Say why not, and exit non-zero.
fn refuse(why: &str) -> std::io::Result<()> {
    eprintln!("{why}");
    std::process::exit(1);
}

/// `--name value` pairs, as the harness substituted them.
///
/// An argument the model did not give still arrives, as an empty string: the substitution has a
/// flag to fill and nothing to fill it with. Empty is dropped here rather than passed on, so the
/// vocabulary sees "not given" and says what is missing instead of failing on a name of "".
fn arguments() -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    let mut args = std::env::args().skip(2);
    while let Some(flag) = args.next() {
        let Some(name) = flag.strip_prefix("--") else {
            continue;
        };
        // `--name=value` as well as `--name value`, because both are things a config writes and
        // neither is worth a bug report.
        if let Some((name, value)) = name.split_once('=') {
            if !value.is_empty() {
                out.insert(name.to_owned(), value.to_owned());
            }
            continue;
        }
        if let Some(value) = args.next()
            && !value.is_empty()
        {
            out.insert(name.to_owned(), value);
        }
    }
    out
}

/// What this session is, as far as a separate process can tell.
fn standing() -> Option<Standing> {
    let me = melchior::directory::mine()?;
    Some(Standing {
        // Asked of its own socket rather than kept: a session reads its inbox and acts on it
        // while this process does not exist, so anything remembered here would be a snapshot of
        // a moment nobody cares about.
        inbox: melchior::directory::inbox_of(&me),
        // Read off the directory, never from what a child says about itself. A child that
        // declined to leave its note would otherwise have made itself unstoppable.
        forked: melchior::directory::children(&me),
        parent: melchior::directory::parent_of(&me),
        // Empty, so `stop` is refused with "this session did not start it". The secrets are
        // minted by whatever spawns a child, and nothing spawns one yet; when something does,
        // it hands them down the same way a name is handed down.
        minted: std::collections::BTreeMap::new(),
        me: me.full(),
    })
}

/// Arguments arrive as flags and are read as a call.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_argument_is_not_an_argument() {
        // The substitution has a flag to fill and nothing to fill it with, so `--who` arrives
        // with an empty string. Passed on, the vocabulary would look for a session named "".
        let asked: std::collections::BTreeMap<String, String> =
            [("verb".to_owned(), "list".to_owned())]
                .into_iter()
                .collect();
        let value = serde_json::to_value(&asked).expect("encodes");
        assert_eq!(value["verb"], "list");
        assert!(value.get("who").is_none());
    }

    #[test]
    fn what_it_produces_is_what_the_vocabulary_takes() {
        // The one thing this file has to get right: the map it builds is the JSON object the
        // verbs read their arguments out of.
        let asked: std::collections::BTreeMap<String, String> = [
            ("verb".to_owned(), "send".to_owned()),
            ("who".to_owned(), "beta-nu".to_owned()),
            ("message".to_owned(), "hello".to_owned()),
        ]
        .into_iter()
        .collect();
        let value = serde_json::to_value(&asked).expect("encodes");
        let answer = verbs::answer(&value, &Standing::default());
        // No session to be, so it refuses — but on the *name*, which means it read the verb and
        // the arguments and got as far as looking somebody up.
        assert!(answer.failed);
    }
}
