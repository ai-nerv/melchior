use super::*;
use crate::mind::lua::adapter::{
    instructions,
    support::{adapter, plain_model},
};
use crate::mind::model::{Message, ToolStrictness};
use crate::mind::provider::{
    api::{Adapter, Options},
    compat::{ChatStrictDialect, Compat},
};
use serde_json::{Value, json};

fn tool(name: &str) -> Tool {
    Tool {
        name: name.into(),
        description: format!("Use {name}"),
        parameters: json!({"type":"object","properties":{"value":{"type":"string"}},"required":["value"],"additionalProperties":false}),
        strict: None,
    }
}

fn context() -> Context {
    let mut change = Message::tool_changes(vec![tool("b")], vec![]);
    change.content.push(Content::Text {
        text: "update α".into(),
        signature: None,
    });
    Context {
        system: Some("base".into()),
        tools: vec![tool("a")],
        messages: vec![Message::user("first"), change, Message::user("later")],
    }
}

fn model(system: bool, additions: bool) -> crate::mind::provider::model::Model {
    let mut model = plain_model();
    model.compat = Some(Compat {
        supports_mid_convo_system_messages: Some(system),
        supports_mid_convo_tool_additions: Some(additions),
        ..Compat::default()
    });
    model
}

fn prepared(ctx: &Context, model: &crate::mind::provider::model::Model) -> Value {
    let saved = ctx.clone();
    let body = adapter("openai-completions")
        .prepare(model, ctx, &Options::default())
        .expect("prepare")
        .0;
    assert_eq!(ctx, &saved);
    body
}

fn names(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .map(|tool| tool["function"]["name"].as_str().expect("name").into())
        .collect()
}

#[test]
fn neutral_tool_changes_round_trip_lossless_schemas_without_changing_legacy_messages() {
    let mut added = tool("lookup");
    added.parameters = json!({"const":{"empty":{},"array":[],"null":null,"max":u64::MAX}});
    added.strict = Some(ToolStrictness::Require);
    let message = Message::tool_changes(vec![added.clone()], vec!["old".into()]);
    let value = serde_json::to_value(&message).expect("encode");
    assert_eq!(
        value,
        json!({"role":"system","content":[{"type":"tool_changes","added":[added],"removed":["old"]}]})
    );
    assert_eq!(
        serde_json::from_value::<Message>(value).expect("decode"),
        message
    );
    assert_eq!(
        serde_json::to_value(Message::user("hi")).expect("legacy"),
        json!({"role":"user","content":[{"type":"text","text":"hi"}]})
    );
    assert!(message.text().is_empty());
    assert_eq!(message.tool_calls().count(), 0);
}

#[test]
fn inline_schemas_preserve_empty_shapes_nulls_and_full_width_integers_across_lua() {
    let parameters = json!({"type":"object","properties":{"value":{"const":{"array":[],"object":{},"null":null,"integer":u64::MAX,"__metatable":null}}}});
    for native in [false, true] {
        let mut ctx = context();
        if let Content::ToolChanges { added, .. } = &mut ctx.messages[1].content[0] {
            added[0].parameters = parameters.clone();
        }
        let body = prepared(&ctx, &model(true, native));
        let declaration = if native {
            &body["messages"][2]["tools"][0]
        } else {
            &body["tools"][1]
        };
        assert_eq!(declaration["function"]["parameters"], parameters);
    }
}

#[test]
fn additions_stay_anchored_only_with_both_chat_flags_and_never_share_content() {
    for positional in [false, true] {
        for additions in [false, true] {
            for developer in [false, true] {
                let mut model = model(positional, additions);
                model
                    .compat
                    .as_mut()
                    .expect("compat")
                    .supports_developer_role = Some(developer);
                let ctx = context();
                let body = prepared(&ctx, &model);
                let messages = body["messages"].as_array().expect("messages");
                let native = positional && additions;
                assert_eq!(
                    names(&body["tools"]),
                    if native { vec!["a"] } else { vec!["a", "b"] }
                );
                let inline: Vec<_> = messages
                    .iter()
                    .filter(|m| m.get("tools").is_some())
                    .collect();
                assert_eq!(inline.len(), usize::from(native));
                if native {
                    assert_eq!(inline[0]["role"], "system");
                    assert!(inline[0].get("content").is_none());
                    assert_eq!(names(&inline[0]["tools"]), ["b"]);
                    assert_eq!(messages[1]["content"], "first");
                    assert_eq!(messages[3]["content"], "update α");
                    assert_eq!(
                        messages[3]["role"],
                        if developer { "developer" } else { "system" }
                    );
                }
                assert_eq!(
                    messages[0]["content"],
                    if positional {
                        "base"
                    } else {
                        "base\n\nupdate α"
                    }
                );
                assert!(!body.to_string().contains("tool_changes"));
            }
        }
    }
}

#[test]
fn removal_redefinition_and_identical_redeclaration_fold_through_the_last_change_only() {
    for (added, removed, expected) in [
        (vec![], vec!["a".into()], vec!["b"]),
        (vec![], vec!["missing".into()], vec!["a", "b"]),
        (vec![tool("a")], vec![], vec!["a", "b"]),
        (vec![tool("b")], vec![], vec!["a", "b"]),
        (vec![tool("a")], vec!["a".into()], vec!["b", "a"]),
    ] {
        let mut ctx = context();
        ctx.messages.push(Message::tool_changes(added, removed));
        ctx.messages.push(Message::user("after change"));
        ctx.messages
            .push(Message::tool_changes(vec![tool("c")], vec![]));
        let body = prepared(&ctx, &model(true, true));
        assert_eq!(names(&body["tools"]), expected);
        let inline: Vec<_> = body["messages"]
            .as_array()
            .expect("messages")
            .iter()
            .filter(|m| m.get("tools").is_some())
            .collect();
        assert_eq!(inline.len(), 1);
        assert_eq!(names(&inline[0]["tools"]), ["c"]);
        let messages = body["messages"].as_array().expect("messages");
        assert_eq!(messages[messages.len() - 2]["content"], "after change");
    }
    let mut ctx = context();
    ctx.messages
        .push(Message::tool_changes(vec![], vec!["b".into()]));
    ctx.messages
        .push(Message::tool_changes(vec![tool("b")], vec![]));
    let body = prepared(&ctx, &model(true, true));
    assert_eq!(names(&body["tools"]), ["a", "b"]);
    assert!(
        !body["messages"]
            .as_array()
            .expect("messages")
            .iter()
            .any(|m| m.get("tools").is_some())
    );
}

#[test]
fn replacements_retain_insertion_order_and_duplicate_definitions_use_the_last_value() {
    let mut one = tool("a");
    one.description = "replacement".into();
    let mut last = tool("b");
    last.parameters = json!({"const":{"n":u64::MAX,"empty":[],"o":{},"null":null}});
    let ctx = Context {
        tools: vec![tool("a")],
        messages: vec![
            Message::user("first"),
            Message::tool_changes(vec![tool("b"), one.clone(), last.clone()], vec![]),
        ],
        ..Context::default()
    };
    for flags in [(false, false), (true, false), (true, true)] {
        let body = prepared(&ctx, &model(flags.0, flags.1));
        assert_eq!(names(&body["tools"]), ["a", "b"]);
        assert_eq!(body["tools"][0]["function"]["description"], one.description);
        assert_eq!(body["tools"][1]["function"]["parameters"], last.parameters);
    }
}

#[test]
fn initial_system_tools_fold_but_first_midconversation_additions_do_not() {
    for system in [None, Some(""), Some("base")] {
        for first in [false, true] {
            let mut ctx = Context {
                system: system.map(str::to_owned),
                messages: vec![Message::tool_changes(vec![tool("a")], vec![])],
                ..Context::default()
            };
            if !first {
                ctx.messages.insert(0, Message::user("first"));
            }
            let body = prepared(&ctx, &model(true, true));
            let folded = first && system.is_none_or(str::is_empty);
            assert_eq!(
                names(&body["tools"]),
                if folded { vec!["a"] } else { vec![] }
            );
            let inline: Vec<_> = body["messages"]
                .as_array()
                .expect("messages")
                .iter()
                .filter(|m| m.get("tools").is_some())
                .collect();
            assert_eq!(inline.len(), usize::from(!folded));
            if !folded {
                assert_eq!(names(&inline[0]["tools"]), ["a"]);
            }
        }
    }
}

#[test]
fn fallback_resolves_the_current_set_for_every_registered_adapter_without_mutation() {
    for api in [
        "openai-completions",
        "openai-responses",
        "azure-openai-responses",
        "openai-codex-responses",
        "anthropic-messages",
        "google-generative-ai",
        "google-vertex",
        "pi-messages",
        "decisions",
    ] {
        let mut ctx = context();
        ctx.messages
            .push(Message::tool_changes(vec![tool("c")], vec!["a".into()]));
        let saved = ctx.clone();
        let rebuilt = instructions::context(api, &model(false, true), &ctx).expect("fallback");
        assert_eq!(
            rebuilt
                .tools
                .iter()
                .map(|t| t.name.as_str())
                .collect::<Vec<_>>(),
            ["b", "c"]
        );
        assert!(
            !serde_json::to_string(&rebuilt)
                .expect("JSON")
                .contains("tool_changes")
        );
        assert_eq!(rebuilt.system.as_deref(), Some("base\n\nupdate α"));
        assert_eq!(ctx, saved);
    }
}

#[test]
fn tool_history_is_system_only_bounded_and_rejected_before_protocol_serialization() {
    for role in [Role::User, Role::Assistant, Role::Tool] {
        let mut ctx = context();
        ctx.messages[1].role = role;
        assert!(instructions::context("openai-completions", &model(true, true), &ctx).is_err());
    }
    let mut ctx = context();
    ctx.messages[1].content.push(Content::ToolChanges {
        added: vec![],
        removed: vec![],
    });
    assert!(validate(&ctx).is_err());
    for bad in ["".into(), "a\nb".into(), "x".repeat(257)] {
        let mut ctx = context();
        ctx.messages
            .push(Message::tool_changes(vec![tool(&bad)], vec![]));
        assert!(validate(&ctx).is_err());
        ctx.messages.pop();
        ctx.messages.push(Message::tool_changes(vec![], vec![bad]));
        assert!(validate(&ctx).is_err());
    }
    let mut ctx = Context {
        tools: (0..ACTIVE).map(|i| tool(&format!("t{i}"))).collect(),
        messages: vec![Message::tool_changes(vec![tool("new")], vec!["t0".into()])],
        ..Context::default()
    };
    assert!(instructions::context("openai-completions", &model(true, true), &ctx).is_ok());
    ctx.messages = vec![Message::tool_changes(vec![tool("new")], vec![])];
    assert!(instructions::context("openai-completions", &model(true, true), &ctx).is_err());
    ctx.tools.clear();
    ctx.messages = vec![Message::tool_changes(
        vec![],
        vec!["missing".into(); CHANGES],
    )];
    assert!(validate(&ctx).is_ok());
    ctx.tools.push(tool("one too many"));
    assert!(validate(&ctx).is_err());
}

#[test]
fn declaration_bytes_have_an_exact_aggregate_boundary_without_serializing_into_an_unbounded_buffer()
{
    let mut item = tool("a");
    let base = serde_json::to_vec(&Content::ToolChanges {
        added: vec![item.clone()],
        removed: vec![],
    })
    .expect("encode")
    .len()
        + 2;
    item.description.push_str(&"x".repeat(BYTES - base));
    let mut ctx = Context {
        messages: vec![Message::tool_changes(vec![item], vec![])],
        ..Context::default()
    };
    assert!(validate(&ctx).is_ok());
    if let Content::ToolChanges { added, .. } = &mut ctx.messages[0].content[0] {
        added[0].description.push('x');
    }
    assert!(validate(&ctx).is_err());
}

#[test]
fn strict_negotiation_includes_inline_tools_and_deepseek_whole_set_routing() {
    for supported in [false, true] {
        for policy in [ToolStrictness::Prefer, ToolStrictness::Require] {
            let mut ctx = context();
            if let Content::ToolChanges { added, .. } = &mut ctx.messages[1].content[0] {
                added[0].strict = Some(policy);
            }
            let mut model = model(true, true);
            model.compat.as_mut().expect("compat").supports_strict_mode = Some(supported);
            let result = adapter("openai-completions").prepare(&model, &ctx, &Options::default());
            if !supported && policy == ToolStrictness::Require {
                assert!(result.is_err());
                continue;
            }
            let body = result.expect("prepare").0;
            let inline = &body["messages"][2]["tools"][0]["function"];
            assert_eq!(inline.get("strict"), supported.then_some(&json!(true)));
            assert_eq!(inline["parameters"], tool("b").parameters);
        }
    }
    for required in [false, true] {
        let mut ctx = context();
        if let Content::ToolChanges { added, .. } = &mut ctx.messages[1].content[0] {
            added[0].strict = Some(if required {
                ToolStrictness::Require
            } else {
                ToolStrictness::Prefer
            });
        }
        let mut model = model(true, true);
        let compat = model.compat.as_mut().expect("compat");
        compat.supports_strict_mode = Some(true);
        compat.chat_strict_dialect = Some(ChatStrictDialect::DeepSeek);
        let wire = adapter("openai-completions");
        let mixed = wire.prepare(&model, &ctx, &Options::default());
        if required {
            assert!(mixed.is_err());
        } else {
            let body = mixed.expect("fallback").0;
            assert_eq!(body["tools"][0]["function"]["strict"], false);
            assert_eq!(body["messages"][2]["tools"][0]["function"]["strict"], false);
        }
        ctx.tools.clear();
        let body = wire
            .prepare(&model, &ctx, &Options::default())
            .expect("inline strict")
            .0;
        assert!(body.get("tools").is_none());
        assert_eq!(
            wire.request_endpoint("https://fixture.invalid/v1", &model, &body)
                .expect("route"),
            "https://fixture.invalid/beta/chat/completions"
        );
    }
}

#[test]
fn explicit_false_disables_inherited_additions_and_true_on_other_protocols_never_drops_history() {
    let parent = Some(Compat {
        supports_mid_convo_system_messages: Some(true),
        supports_mid_convo_tool_additions: Some(true),
        ..Compat::default()
    });
    let mut model = plain_model();
    model.compat = Some(
        Compat {
            supports_mid_convo_tool_additions: Some(false),
            ..Compat::default()
        }
        .over(parent),
    );
    assert!(
        !model
            .compat
            .expect("compat")
            .resolve()
            .supports_mid_convo_tool_additions
    );
    let ctx = context();
    assert_eq!(names(&prepared(&ctx, &model)["tools"]), ["a", "b"]);
    model.compat = parent;
    assert!(instructions::context("anthropic-messages", &model, &ctx).is_err());
}

#[test]
fn every_registered_adapter_serializes_fallback_like_an_explicit_current_tool_set() {
    let apis = crate::mind::lua::adapter::engine_with_builtins()
        .expect("builtins")
        .apis();
    assert_eq!(apis.len(), 10);
    for api in apis {
        let mut model = model(false, false);
        model.api = serde_json::from_value(json!(api)).expect("API");
        let mut ctx = context();
        ctx.messages
            .push(Message::tool_changes(vec![tool("c")], vec!["a".into()]));
        let saved = ctx.clone();
        let explicit = Context {
            system: Some("base\n\nupdate α".into()),
            tools: vec![tool("b"), tool("c")],
            messages: vec![Message::user("first"), Message::user("later")],
        };
        let wire = adapter(&api);
        let actual = wire
            .prepare(&model, &ctx, &Options::default())
            .expect("history");
        let expected = wire
            .prepare(&model, &explicit, &Options::default())
            .expect("current tools");
        assert_eq!(actual, expected, "{api}");
        assert_eq!(ctx, saved);
        assert!(!actual.0.to_string().contains("tool_changes"));
    }
}

#[test]
fn replay_matches_a_vector_oracle_for_all_four_change_sequences() {
    let operations = [
        (vec![tool("b")], vec![]),
        (vec![tool("a")], vec![]),
        (vec![tool("b"), tool("b")], vec![]),
        (vec![], vec!["a".to_owned()]),
        (vec![], vec!["b".to_owned()]),
        (vec![], vec!["absent".to_owned()]),
        (vec![tool("c")], vec![]),
        (vec![tool("b")], vec!["b".to_owned()]),
    ];
    for sequence in 0..8usize.pow(4) {
        let mut ctx = Context {
            tools: vec![tool("a")],
            messages: vec![Message::user("first")],
            ..Context::default()
        };
        let mut oracle = ctx.tools.clone();
        let mut declared = vec!["a".to_owned()];
        let mut folded = None;
        let mut snapshots = Vec::new();
        for step in 0..4 {
            let (added, removed) = &operations[(sequence >> (step * 3)) & 7];
            let mut non_additive = !removed.is_empty();
            oracle.retain(|t| !removed.contains(&t.name));
            for tool in added {
                non_additive |= declared.contains(&tool.name);
                declared.push(tool.name.clone());
                if let Some(previous) = oracle.iter_mut().find(|t| t.name == tool.name) {
                    *previous = tool.clone();
                } else {
                    oracle.push(tool.clone());
                }
            }
            if non_additive {
                folded = Some(step);
            }
            snapshots.push(oracle.clone());
            ctx.messages
                .push(Message::tool_changes(added.clone(), removed.clone()));
        }
        let saved = ctx.clone();
        for native in [false, true] {
            let result = instructions::context("openai-completions", &model(true, native), &ctx)
                .expect("replay");
            let wanted = if native {
                folded.map_or(&ctx.tools, |at| &snapshots[at])
            } else {
                &oracle
            };
            assert_eq!(&result.tools, wanted, "sequence={sequence} native={native}");
            for (step, message) in result.messages.iter().skip(1).enumerate() {
                let keep = native && folded.is_none_or(|at| step > at);
                assert_eq!(
                    message.content,
                    if keep {
                        ctx.messages[step + 1].content.clone()
                    } else {
                        vec![]
                    },
                    "sequence={sequence} native={native} step={step}"
                );
            }
        }
        assert_eq!(ctx, saved);
    }
}
