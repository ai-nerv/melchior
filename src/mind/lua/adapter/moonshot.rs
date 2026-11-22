use super::support::{adapter, plain_context, plain_model};
use crate::mind::model::ThinkingLevel;
use crate::mind::provider::{
    api::{Adapter, Options},
    compat::{Compat, ThinkingFormat},
};
use serde_json::json;

#[test]
fn preserved_thinking_is_opt_in_and_explicit_false_overrides_the_provider() {
    assert!(!Compat::default().resolve().moonshot_keep_thinking);
    let base = Some(Compat {
        moonshot_keep_thinking: Some(true),
        supports_store: Some(true),
        ..Compat::default()
    });
    for flag in [None, Some(false), Some(true)] {
        let compat = Compat {
            moonshot_keep_thinking: flag,
            ..Compat::default()
        }
        .over(base);
        assert_eq!(
            compat.resolve().moonshot_keep_thinking,
            flag.unwrap_or(true)
        );
        assert!(compat.resolve().supports_store);
        assert_eq!(
            serde_json::from_value::<Compat>(serde_json::to_value(compat).expect("JSON"))
                .expect("compat"),
            compat
        );
    }
}

#[test]
fn only_reasoning_deepseek_dialects_gain_keep_all_and_off_remains_disabled() {
    for format in [
        ThinkingFormat::DeepSeek,
        ThinkingFormat::OpenAi,
        ThinkingFormat::Zai,
        ThinkingFormat::OpenRouter,
        ThinkingFormat::Qwen,
    ] {
        for reasoning in [false, true] {
            for enabled in [false, true] {
                for level in [None, Some(ThinkingLevel::Off), Some(ThinkingLevel::High)] {
                    let mut model = plain_model();
                    model.reasoning = reasoning;
                    model.compat = Some(Compat {
                        thinking_format: Some(format),
                        moonshot_keep_thinking: Some(enabled),
                        ..Compat::default()
                    });
                    let body = adapter("openai-completions").request(
                        &model,
                        &plain_context(),
                        &Options {
                            thinking: level,
                            ..Options::default()
                        },
                    );
                    let keep = reasoning
                        && enabled
                        && format == ThinkingFormat::DeepSeek
                        && level != Some(ThinkingLevel::Off);
                    assert_eq!(body["thinking"].get("keep"), keep.then_some(&json!("all")));
                    if keep {
                        assert_eq!(body["thinking"], json!({"type":"enabled","keep":"all"}));
                    } else if format == ThinkingFormat::DeepSeek
                        && level == Some(ThinkingLevel::Off)
                    {
                        assert_eq!(body["thinking"], json!({"type":"disabled"}));
                    }
                }
            }
        }
    }
}
