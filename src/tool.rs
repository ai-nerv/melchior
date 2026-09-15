//! `melchior tool` — the vocabulary a model calls, as one exec per request: the harness runs this
//! with the model's arguments in `argv`, reads stdout, and takes the exit code as whether it
//! worked. Which session it speaks as comes from the environment, because nothing on disk says
//! which of several a given process was spawned under.

use melchior::verbs::{self, Standing};

/// Run one call and exit. A refusal goes to stderr with a non-zero status, because a refusal
/// arriving as a success reads to a model as "that worked".
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
            melchior::inherited::PROJECT,
            melchior::inherited::ID
        ));
    };

    let answer = verbs::answer(&serde_json::to_value(&asked).unwrap_or_default(), &standing);
    if answer.failed {
        return refuse(&answer.said);
    }
    println!("{}", answer.said);
    Ok(())
}

fn refuse(why: &str) -> std::io::Result<()> {
    eprintln!("{why}");
    std::process::exit(1);
}

/// `--name value` and `--name=value` pairs, as the harness substituted them. An argument the model
/// did not give still arrives as an empty string, and is dropped here rather than passed on, so
/// the vocabulary sees "not given" instead of failing on a name of "".
fn arguments() -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    let mut args = std::env::args().skip(2);
    while let Some(flag) = args.next() {
        let Some(name) = flag.strip_prefix("--") else {
            continue;
        };
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

/// What this session is, as far as a separate process can tell. Every field is read off the
/// directory or asked of the session's own socket, never taken from what a child says about
/// itself: a child that declined to leave its note would otherwise have made itself unstoppable.
fn standing() -> Option<Standing> {
    let me = melchior::directory::mine()?;
    Some(Standing {
        inbox: melchior::directory::inbox_of(&me),
        forked: melchior::directory::children(&me),
        parent: melchior::directory::parent_of(&me),
        minted: melchior::directory::minted_by(&me),
        me: me.full(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_argument_is_not_an_argument() {
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
        // The map this file builds is the JSON object the verbs read their arguments out of.
        let asked: std::collections::BTreeMap<String, String> = [
            ("verb".to_owned(), "send".to_owned()),
            ("who".to_owned(), "beta-nu".to_owned()),
            ("message".to_owned(), "hello".to_owned()),
        ]
        .into_iter()
        .collect();
        let value = serde_json::to_value(&asked).expect("encodes");
        let answer = verbs::answer(&value, &Standing::default());
        // Refused on the name, which means it read the verb and the arguments.
        assert!(answer.failed);
    }
}
