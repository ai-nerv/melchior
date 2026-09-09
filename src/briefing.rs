//! Telling the model an instance was named.
//!
//! `$iota-mu` in a prompt sends nothing. It appends a bounded note — who they are, how they stand
//! to this session, what has already passed between them, and the name of the tool for reaching
//! them — and the model decides what to do with it. Empty when a prompt named nobody, so there is
//! never anything to strip back out.

use crate::directory::{Address, TOOL};
use crate::policy::Reach;
use crate::policy::{self, Relation};
use crate::verbs::Standing;

/// How many earlier messages with an instance are worth repeating.
const RECALLED: usize = 6;

/// What the model needs in order to act on the instances a prompt names. `named` is what the
/// harness found in the prompt: scanning one means knowing about cursors and sigil tables, so
/// this is handed the names rather than the prompt.
#[must_use]
pub fn about(named: &[String], standing: &Standing) -> String {
    if named.is_empty() {
        return String::new();
    }
    let mut said = Vec::new();
    for name in named {
        let Some(address) = Address::read(name) else {
            said.push(format!(
                "`${name}` is not a name an instance can have, so nothing answers to it. \
                 Names are `id`, `role/id` or `project/role/id`."
            ));
            continue;
        };
        said.push(brief(&address, standing));
    }
    said.push(String::new());
    said.push(format!(
        "Use the `{TOOL}` tool to reach any of them. Call `{TOOL}` with `verb: \"help\"` \
         to see everything it can do."
    ));
    said.join("\n")
}

/// What is known about one instance, as a paragraph the model can act on.
fn brief(address: &Address, app: &Standing) -> String {
    let whole = address.against(&app.identity());
    let me = app.whom();
    // Through the same `Standing` the tool will use, so the briefing and the refusal cannot
    // disagree about where somebody sits.
    let relation = app.stands(&whole);
    let mut said = format!(
        "`{}` is another magi, addressed as `{}`. It is {}.",
        whole.full(),
        address.written(),
        relation.named()
    );
    // Said before anything else about it, so a model does not plan a turn against a refusal.
    if !policy::may(&me, relation, Reach::Ask) {
        said.push_str(" This session cannot reach it: ");
        said.push_str(&policy::refusal(&me, relation, Reach::Ask));
        said.push('.');
        return said;
    }
    if relation == Relation::Child {
        said.push_str(" This session started it, so it can be stopped as well as asked.");
    } else {
        said.push_str(" It can be asked and told things, but not stopped.");
    }
    let passed: Vec<String> = app
        .inbox
        .iter()
        .filter(|message| message.from == whole.full())
        .rev()
        .take(RECALLED)
        .map(|message| format!("  - it said: {}", message.text))
        .collect();
    if !passed.is_empty() {
        said.push_str("\n\n  What has already passed between you:\n");
        // Oldest first, because that is the order it happened in and the order a reply reads.
        said.push_str(&passed.into_iter().rev().collect::<Vec<_>>().join("\n"));
    }
    said
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::Message;

    /// What the harness would have found in a prompt. The real one is `magi_tui::trigger`.
    fn named(text: &str) -> Vec<String> {
        text.split_whitespace()
            .filter_map(|word| word.strip_prefix('$'))
            .map(ToOwned::to_owned)
            .collect()
    }

    fn app() -> Standing {
        Standing {
            me: "magi/main/alpha-rho".to_owned(),
            ..Standing::default()
        }
    }

    #[test]
    fn a_prompt_that_names_nobody_produces_nothing_at_all() {
        let app = app();
        for text in ["fix the parser", "look at @src/main.rs", "it cost me 20$"] {
            assert!(
                about(&named(text), &app).is_empty(),
                "{text:?} produced an aside"
            );
        }
    }

    #[test]
    fn naming_one_says_what_is_known_about_it_and_nothing_of_the_prompt() {
        // Beside the prompt rather than spliced onto it.
        let said = about(&named("tell $beta-nu to stop"), &app());
        assert!(!said.contains("tell $beta-nu to stop"), "{said}");
        assert!(said.contains("magi/main/beta-nu"), "{said}");
    }

    #[test]
    fn it_names_the_tool_rather_than_doing_anything() {
        let said = about(&named("tell $beta-nu to stop"), &app());
        assert!(said.contains(crate::directory::TOOL), "{said}");
    }

    #[test]
    fn it_says_how_the_named_one_stands_to_this_session() {
        let said = about(&named("ask $beta-nu"), &app());
        assert!(said.contains("another instance's main"), "{said}");
        assert!(said.contains("not stopped"), "{said}");
    }

    #[test]
    fn one_it_cannot_reach_says_so_instead_of_its_history() {
        let mut app = app();
        app.me = "somewhere-with-no-runtime-dir/main/alpha-rho".to_owned();
        app.parent = Some("beta-nu".to_owned());
        let said = about(&named("ask $other/tau-chi"), &app);
        assert!(said.contains("cannot reach"), "{said}");
    }

    #[test]
    fn what_has_already_been_said_comes_back_oldest_first() {
        let mut app = app();
        for text in ["first", "second", "third"] {
            app.inbox.push(Message::new("magi/main/beta-nu", text));
        }
        let said = about(&named("what did $beta-nu want"), &app);
        let first = said.find("first").expect("the first is there");
        let third = said.find("third").expect("the third is there");
        assert!(first < third, "they came back backwards: {said}");
    }

    #[test]
    fn only_that_instance_s_messages_are_repeated() {
        let mut app = app();
        app.inbox
            .push(Message::new("magi/main/beta-nu", "from beta"));
        app.inbox
            .push(Message::new("magi/main/gamma-xi", "from gamma"));
        let said = about(&named("what did $beta-nu want"), &app);
        assert!(said.contains("from beta"), "{said}");
        assert!(!said.contains("from gamma"), "it leaked another's: {said}");
    }

    #[test]
    fn a_long_exchange_is_cut_rather_than_pasted_whole() {
        let mut app = app();
        for at in 0..50 {
            app.inbox
                .push(Message::new("magi/main/beta-nu", &format!("message {at}")));
        }
        let said = about(&named("what did $beta-nu want"), &app);
        assert!(!said.contains("message 0"), "it pasted the whole exchange");
        assert!(said.contains("message 49"), "it dropped the newest");
    }

    #[test]
    fn a_name_nothing_can_have_is_said_to_be_one() {
        let said = about(&named("ask $a/b/c/d about it"), &app());
        assert!(said.contains("not a name"), "{said}");
    }
}
