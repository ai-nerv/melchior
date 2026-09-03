//! Running a turn: an [`Ask`] in, a stream of [`Said`] out.
//!
//! This is what makes melchior the mind rather than a catalogue of one. The protocol comes out
//! of Lua, the credential is resolved here and never travels, and every delta is handed to the
//! caller as it arrives — a broker watching a spinner needs to see the answer forming, and
//! afterwards is too late to say so.
//!
//! The stream always ends. [`Said::Stop`] when the model finished, [`Said::Failed`] when it
//! could not be asked, and never silence: a caller that gets neither cannot tell a mind that
//! refused from one that was lost, and would wait forever on the difference.

use crate::mind::catalog::Catalog;
use crate::mind::lua::adapter::LuaAdapter;
use crate::mind::provider::api::{Delta, Options};
use crate::mind::provider::client::{Call, Client};
use crate::mind::wire::{Ask, Refusal, Said};

/// Run one ask, handing over each [`Said`] as it happens.
///
/// Never returns an error: everything that could go wrong is a [`Said::Failed`], because the
/// caller is reading a stream and a stream that stops without saying so is the one failure it
/// cannot interpret.
pub async fn run(asked: &Ask, mut say: impl FnMut(Said)) {
    let catalog = match Catalog::load(&Catalog::dir()) {
        Ok(catalog) => catalog,
        Err(why) => {
            say(Said::Failed {
                message: format!("the configuration will not load: {why}"),
                why: Refusal::Invalid,
            });
            return;
        }
    };

    let Some((provider, model)) = catalog.find(&asked.model) else {
        say(Said::Failed {
            message: format!(
                "no model called {:?}. `melchior models` lists what there is",
                asked.model
            ),
            why: Refusal::Invalid,
        });
        return;
    };
    // Cloned out of the catalog so the engine can be moved into the adapter, which takes it.
    let (provider, model) = (provider.clone(), model.clone());

    let api = match serde_json::to_value(model.api)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
    {
        Some(api) => api,
        None => {
            say(Said::Failed {
                message: "that model names no interface".to_owned(),
                why: Refusal::Invalid,
            });
            return;
        }
    };
    let adapter = match LuaAdapter::new(catalog.engine, &api) {
        Ok(adapter) => adapter,
        Err(why) => {
            say(Said::Failed {
                message: why,
                why: Refusal::Invalid,
            });
            return;
        }
    };

    let options = Options {
        thinking: asked.wants.thinking,
        max_tokens: asked.wants.max_tokens,
        schema: asked
            .wants
            .schema
            .as_ref()
            .map(|s| crate::mind::provider::api::Schema {
                name: s.name.clone(),
                schema: s.schema.clone(),
            }),
    };
    let call = Call {
        adapter: &adapter,
        provider: &provider,
        model: &model,
        context: &asked.context,
        options: &options,
    };

    // Held back rather than forwarded. A protocol may report the stop before the token counts
    // and may report it twice — the adapter says so, and the client says so again at the end of
    // the body — and a reader that stops at the first terminal would drop the usage that came
    // after it. Exactly one ends the stream, and it is last.
    // Shared, because both callbacks say things and a closure cannot borrow the same `FnMut`
    // twice. A cell rather than a channel: this is one thread and the cost is a borrow check.
    let ended = std::cell::Cell::new(None);
    let say = std::cell::RefCell::new(say);
    let outcome = Client::new()
        .stream_reporting(
            &call,
            |delta| match carried(delta) {
                Said::Stop { reason } => ended.set(Some(reason)),
                said => (say.borrow_mut())(said),
            },
            // Said during the wait, not after it. A broker showing a spinner has no other way to
            // tell somebody that forty seconds of nothing is a backoff rather than a hang.
            |retry| {
                (say.borrow_mut())(Said::Retrying {
                    attempt: retry.attempt,
                    of: retry.max_attempts,
                    seconds: retry.delay.as_secs_f64(),
                    why: retry.why.clone(),
                });
            },
        )
        .await;

    let mut say = say.into_inner();
    match outcome {
        Err(why) => say(Said::Failed {
            message: why.to_string(),
            why: refused(why.class),
        }),
        // A stream that ended without the protocol saying why still ended. `EndTurn` is the
        // honest reading: the body finished and nothing complained.
        Ok(()) => say(Said::Stop {
            reason: ended
                .into_inner()
                .unwrap_or(crate::mind::model::StopReason::EndTurn),
        }),
    }
}

/// One provider delta, as the wire says it.
///
/// A translation and nothing more. What the protocols produce is already the small vocabulary a
/// caller wants; this only puts it in the shape both sides agreed on.
fn carried(delta: Delta) -> Said {
    match delta {
        Delta::Text(text) => Said::Text { text },
        Delta::Thinking(text) => Said::Thinking { text },
        Delta::Signature(signature) => Said::Signature { signature },
        Delta::ToolCallStart { id, name } => Said::ToolCallStart { id, name },
        Delta::ToolCallArgs(args) => Said::ToolCallArgs { args },
        Delta::Usage(usage) => Said::Spent { usage },
        Delta::Stop(reason) => Said::Stop { reason },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mind::model::{Context, Message, StopReason, Usage};

    fn asking(model: &str) -> Ask {
        Ask {
            model: model.to_owned(),
            context: Context {
                messages: vec![Message::user("hi")],
                ..Context::default()
            },
            wants: crate::mind::wire::Wants::default(),
            about: String::new(),
        }
    }

    /// Collect a whole stream, which is what a one-shot caller does.
    async fn everything(asked: &Ask) -> Vec<Said> {
        let said = std::cell::RefCell::new(Vec::new());
        run(asked, |one| said.borrow_mut().push(one)).await;
        said.into_inner()
    }

    #[tokio::test]
    async fn a_model_nobody_has_heard_of_fails_rather_than_hangs() {
        let stream = everything(&asking("nobody/nothing")).await;
        assert_eq!(stream.len(), 1, "{stream:?}");
        assert!(
            matches!(&stream[0], Said::Failed { message, .. } if message.contains("no model")),
            "{stream:?}"
        );
    }

    #[tokio::test]
    async fn every_stream_ends_with_something_that_ends_it() {
        // The property a reader depends on: silence is never an answer.
        let stream = everything(&asking("nobody/nothing")).await;
        assert!(
            stream.last().is_some_and(Said::is_last),
            "a stream must end in stop or failed: {stream:?}"
        );
    }

    #[tokio::test]
    async fn a_real_model_that_has_no_credential_says_so_rather_than_calling() {
        // anthropic is in the shipped catalog and wants a key. Without one this must refuse
        // before any HTTP happens, and it must say which stream it is ending.
        let stream = everything(&asking("anthropic/claude-haiku-4-5")).await;
        assert!(stream.last().is_some_and(Said::is_last), "{stream:?}");
    }

    #[test]
    fn every_delta_becomes_the_said_that_means_the_same() {
        assert!(matches!(
            carried(Delta::Text("a".into())),
            Said::Text { .. }
        ));
        assert!(matches!(
            carried(Delta::Thinking("a".into())),
            Said::Thinking { .. }
        ));
        assert!(matches!(
            carried(Delta::Signature("a".into())),
            Said::Signature { .. }
        ));
        assert!(matches!(
            carried(Delta::ToolCallArgs("{".into())),
            Said::ToolCallArgs { .. }
        ));
        assert!(matches!(
            carried(Delta::Usage(Usage::default())),
            Said::Spent { .. }
        ));
        assert!(matches!(
            carried(Delta::Stop(StopReason::EndTurn)),
            Said::Stop { .. }
        ));
    }
}

#[cfg(test)]
mod ending_tests {
    use super::*;
    use crate::mind::model::StopReason;

    /// What a protocol actually produces, folded the way [`run`] folds it.
    ///
    /// The bug this is for: openrouter reported the stop, then the token counts, then the stop
    /// again. A reader that stops at the first terminal — which the contract tells it to do —
    /// dropped the usage, so every turn through melchior billed zero.
    fn folded(deltas: Vec<Delta>) -> Vec<Said> {
        let mut said = Vec::new();
        let mut ended = None;
        for delta in deltas {
            match carried(delta) {
                Said::Stop { reason } => ended = Some(reason),
                one => said.push(one),
            }
        }
        said.push(Said::Stop {
            reason: ended.unwrap_or(StopReason::EndTurn),
        });
        said
    }

    #[test]
    fn a_stop_reported_twice_ends_the_stream_once() {
        let said = folded(vec![
            Delta::Text("hi".into()),
            Delta::Stop(StopReason::EndTurn),
            Delta::Usage(crate::mind::model::Usage::default()),
            Delta::Stop(StopReason::EndTurn),
        ]);
        assert_eq!(
            said.iter().filter(|s| s.is_last()).count(),
            1,
            "one terminal: {said:?}"
        );
    }

    #[test]
    fn usage_reported_after_the_stop_still_reaches_the_caller() {
        let said = folded(vec![
            Delta::Stop(StopReason::EndTurn),
            Delta::Usage(crate::mind::model::Usage {
                input: 14,
                output: 6,
                ..Default::default()
            }),
        ]);
        assert!(
            said.iter()
                .any(|s| matches!(s, Said::Spent { usage } if usage.input == 14)),
            "the counts were dropped: {said:?}"
        );
        assert!(said.last().is_some_and(Said::is_last), "{said:?}");
    }

    #[test]
    fn the_reason_the_protocol_gave_is_the_one_reported() {
        let said = folded(vec![Delta::Stop(StopReason::Length)]);
        assert!(matches!(
            said.last(),
            Some(Said::Stop {
                reason: StopReason::Length
            })
        ));
    }

    #[test]
    fn a_stream_that_never_said_why_still_ends() {
        let said = folded(vec![Delta::Text("hi".into())]);
        assert!(said.last().is_some_and(Said::is_last), "{said:?}");
    }
}

/// A provider's classification, as the wire names it.
///
/// The seven line up one for one, and they are written out rather than derived so that adding a
/// class on either side is a compile error here rather than a silent `Unknown` a broker cannot
/// act on. `Overflow` in particular is answered by compacting and asking again.
fn refused(class: crate::mind::provider::retry::RetryClass) -> Refusal {
    use crate::mind::provider::retry::RetryClass as R;
    match class {
        R::Transport => Refusal::Transport,
        R::Overload => Refusal::Overload,
        R::Throttle => Refusal::Throttle,
        R::Auth => Refusal::Auth,
        R::Invalid => Refusal::Invalid,
        R::Overflow => Refusal::Overflow,
        R::Unknown => Refusal::Unknown,
    }
}
