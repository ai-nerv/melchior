use super::*;
use crate::mind::provider::compat::ChatStrictDialect;

fn model() -> Model {
    let mut model = declared("openai-completions", true);
    model.compat.as_mut().expect("compat").chat_strict_dialect = Some(ChatStrictDialect::DeepSeek);
    model
}

fn required() -> Value {
    json!({"type":"object","properties":{"term":{"type":"string","pattern":"^a"},
        "count":{"type":"integer","minimum":1,"maximum":10}},"required":["term","count"]})
}

#[test]
fn effective_strict_tools_select_beta_without_changing_ordinary_or_other_dialect_routes() {
    let wire = adapter("openai-completions");
    for (base, strict_url, ordinary_url) in [
        (
            "https://fixture.invalid",
            "https://fixture.invalid/beta/chat/completions",
            "https://fixture.invalid/chat/completions",
        ),
        (
            "https://fixture.invalid/v1/",
            "https://fixture.invalid/beta/chat/completions",
            "https://fixture.invalid/v1/chat/completions",
        ),
        (
            "https://fixture.invalid/beta",
            "https://fixture.invalid/beta/chat/completions",
            "https://fixture.invalid/beta/chat/completions",
        ),
        (
            "http://127.0.0.1:4321/gateway/v1",
            "http://127.0.0.1:4321/gateway/beta/chat/completions",
            "http://127.0.0.1:4321/gateway/v1/chat/completions",
        ),
    ] {
        for policy in [
            None,
            Some(ToolStrictness::Prefer),
            Some(ToolStrictness::Require),
        ] {
            let ctx = context(policy, required());
            let saved = ctx.clone();
            let body = wire
                .prepare(&model(), &ctx, &Options::default())
                .expect("prepare")
                .0;
            assert_eq!(body["tools"][0]["function"]["strict"], policy.is_some());
            assert_eq!(
                wire.request_endpoint(base, &model(), &body)
                    .expect("endpoint"),
                if policy.is_some() {
                    strict_url
                } else {
                    ordinary_url
                }
            );
            assert_eq!(
                wire.request_endpoint(base, &plain_model(), &body)
                    .expect("ordinary dialect"),
                ordinary_url
            );
            assert_eq!(ctx, saved);
        }
        let body = wire
            .prepare(&model(), &plain_context(), &Options::default())
            .expect("without tools")
            .0;
        assert_eq!(
            wire.request_endpoint(base, &model(), &body)
                .expect("endpoint"),
            ordinary_url
        );
    }
}

#[test]
fn mixed_tool_sets_never_weaken_require_or_silently_upgrade_legacy_tools() {
    let wire = adapter("openai-completions");
    for first in [ToolStrictness::Prefer, ToolStrictness::Require] {
        for (second, parameters) in [(None, required()), (Some(ToolStrictness::Prefer), schema())] {
            let mut ctx = context(Some(first), required());
            let mut other = context(second, parameters).tools.remove(0);
            other.name = "other".into();
            ctx.tools.push(other);
            let saved = ctx.clone();
            let result = wire.prepare(&model(), &ctx, &Options::default());
            if first == ToolStrictness::Require {
                assert!(
                    result
                        .expect_err("mixed required set")
                        .contains("every tool")
                );
            } else {
                let body = result.expect("whole set fallback").0;
                for (actual, original) in body["tools"]
                    .as_array()
                    .expect("tools")
                    .iter()
                    .zip(&ctx.tools)
                {
                    assert_eq!(actual["function"]["strict"], false);
                    assert_eq!(actual["function"]["parameters"], original.parameters);
                }
                assert_eq!(
                    wire.request_endpoint("https://fixture.invalid", &model(), &body)
                        .expect("endpoint"),
                    "https://fixture.invalid/chat/completions"
                );
            }
            assert_eq!(ctx, saved);
        }
    }
    let mut ctx = context(Some(ToolStrictness::Require), required());
    let mut other = ctx.tools[0].clone();
    other.name = "other".into();
    other.strict = Some(ToolStrictness::Prefer);
    ctx.tools.push(other);
    let body = wire
        .prepare(&model(), &ctx, &Options::default())
        .expect("all strict")
        .0;
    assert!(
        body["tools"]
            .as_array()
            .expect("tools")
            .iter()
            .all(|t| t["function"]["strict"] == true)
    );
    assert!(
        wire.prepare(
            &model(),
            &ctx,
            &Options {
                disable_preferred_strict: true,
                ..Options::default()
            }
        )
        .is_err()
    );
}

#[test]
fn native_constraint_failures_refuse_require_and_preserve_original_preferred_schemas() {
    let wire = adapter("openai-completions");
    for property in [
        json!({"type":"string","minLength":1}),
        json!({"type":"string","maxLength":5}),
        json!({"type":"string","format":"date"}),
        json!({"type":"null"}),
        json!({"type":["string","null"]}),
        json!({"enum":["a",null]}),
        json!({"type":"array","items":{"type":"string"},"minItems":1}),
        json!({"type":"array","items":{"type":"string"},"maxItems":1}),
        json!({"type":"array","items":{"type":"string"},"uniqueItems":true}),
        json!({"type":"array"}),
        json!({"type":"unknown"}),
    ] {
        let parameters = json!({"type":"object","properties":{"v":property},"required":["v"]});
        for policy in [ToolStrictness::Prefer, ToolStrictness::Require] {
            let ctx = context(Some(policy), parameters.clone());
            let result = wire.prepare(&model(), &ctx, &Options::default());
            if policy == ToolStrictness::Require {
                assert!(
                    result
                        .expect_err("unsupported required schema")
                        .contains("DeepSeek")
                );
            } else {
                let body = result.expect("fallback").0;
                assert_eq!(body["tools"][0]["function"]["strict"], false);
                assert_eq!(body["tools"][0]["function"]["parameters"], parameters);
            }
        }
    }
}

#[test]
fn inheritance_and_policy_hash_distinguish_declared_schema_dialects() {
    let deepseek = model().compat.expect("compat");
    assert_eq!(
        Compat::default()
            .over(Some(deepseek))
            .resolve()
            .chat_strict_dialect,
        ChatStrictDialect::DeepSeek
    );
    let normal = Compat {
        chat_strict_dialect: Some(ChatStrictDialect::OpenAi),
        ..Compat::default()
    }
    .over(Some(deepseek));
    assert_eq!(
        normal.resolve().chat_strict_dialect,
        ChatStrictDialect::OpenAi
    );
    let wire = adapter("openai-completions");
    let (_, before) = wire
        .prepare(&model(), &plain_context(), &Options::default())
        .expect("deepseek");
    let mut normal_model = model();
    normal_model.compat = Some(normal);
    let (_, after) = wire
        .prepare(&normal_model, &plain_context(), &Options::default())
        .expect("normal");
    assert_ne!(before.expect("frame").policy, after.expect("frame").policy);
    let body = wire
        .prepare(
            &model(),
            &context(Some(ToolStrictness::Require), required()),
            &Options::default(),
        )
        .expect("prepare")
        .0;
    assert!(
        wire.request_endpoint(
            "https://fixture.invalid/v1?secret=private-sentinel",
            &model(),
            &body
        )
        .expect_err("ambiguous base")
        .contains("endpoint conversion failed")
    );
}
