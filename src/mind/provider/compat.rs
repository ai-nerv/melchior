//! The quirks one adapter absorbs to serve many vendors: each field is a way some provider that
//! claims to speak OpenAI Chat Completions does not.

use serde::{Deserialize, Serialize};

mod template;
pub use template::{ChatTemplateArgs, ThinkingToggle, ThinkingVariable, ThinkingVariableName};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptCache {
    #[default]
    None,
    AnthropicEphemeral,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnthropicThinking {
    #[default]
    Budget,
    Adaptive,
    AdaptiveEffort,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoogleThinking {
    #[default]
    Auto,
    Budget,
    Level,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnthropicSchema {
    #[default]
    ForcedTool,
    OutputConfig,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatStrictDialect {
    #[default]
    OpenAi,
    DeepSeek,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionAffinityFormat {
    #[default]
    Generic,
    OpenAi,
    OpenRouter,
}

/// Which request field caps the response length.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaxTokensField {
    /// The modern OpenAI name.
    MaxCompletionTokens,
    /// The original, still required by most copies.
    MaxTokens,
}

impl MaxTokensField {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MaxCompletionTokens => "max_completion_tokens",
            Self::MaxTokens => "max_tokens",
        }
    }
}

/// How a provider wants reasoning requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThinkingFormat {
    /// `reasoning_effort: "low"`, as OpenAI does it.
    #[serde(rename = "openai")]
    OpenAi,
    /// `reasoning: { effort }`.
    #[serde(rename = "openrouter")]
    OpenRouter,
    /// `thinking: { type }`, plus `reasoning_effort` where supported.
    #[serde(rename = "deepseek")]
    DeepSeek,
    /// `thinking: { type, clear_thinking }`, plus supported `reasoning_effort`.
    Zai,
    /// Top-level `enable_thinking: bool`.
    Qwen,
    /// `reasoning: { enabled }`, plus model-supported `reasoning_effort`.
    Together,
    /// `reasoning: { effort }` from explicit model mappings.
    AntLing,
    Baseten,
}

/// Overrides for protocol quirks; absent fields inherit the provider's value or conservative defaults.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Compat {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_template_args: Option<ChatTemplateArgs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send_session_affinity_headers: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_affinity_format: Option<SessionAffinityFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_prompt_cache_key: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_strict_dialect: Option<ChatStrictDialect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anthropic_thinking: Option<AnthropicThinking>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub google_thinking: Option<GoogleThinking>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anthropic_schema: Option<AnthropicSchema>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anthropic_replay: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_cache_control_on_tools: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_eager_tool_input_streaming: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_empty_signature: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_forced_tool_choice: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_strict_mode: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_strict_tools: Option<bool>,
    /// Whether the JSON-schema answer format is accepted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_json_schema: Option<bool>,
    /// The declared prompt-cache encoding; an explicit `none` disables it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_cache: Option<PromptCache>,
    /// Whether `store` is accepted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_store: Option<bool>,
    /// Whether the `developer` role is accepted in place of `system`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_developer_role: Option<bool>,
    /// Whether instruction messages may remain at their positions in history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_mid_convo_system_messages: Option<bool>,
    /// Whether positional System messages can declare additional tools.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_mid_convo_tool_additions: Option<bool>,
    /// Whether streamed tools require `tool_stream: true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zai_tool_stream: Option<bool>,
    /// Whether DeepSeek-format thinking sends `keep: "all"` for prior reasoning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moonshot_keep_thinking: Option<bool>,
    /// Whether `reasoning_effort` is accepted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_reasoning_effort: Option<bool>,
    /// Whether `stream_options: {include_usage: true}` yields token counts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_usage_in_streaming: Option<bool>,
    /// Whether streamed chunks carry `finish_reason`. Without it the stop reason is inferred from
    /// whether tool calls arrived.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_finish_reason: Option<bool>,
    /// Which field caps the response length.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens_field: Option<MaxTokensField>,
    /// Whether a tool result must repeat the tool's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_tool_result_name: Option<bool>,
    /// Whether thinking blocks must be flattened into text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_thinking_as_text: Option<bool>,
    /// Whether reasoning models require this field on every assistant message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_reasoning_content_on_assistant_messages: Option<bool>,
    /// How reasoning is requested.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_format: Option<ThinkingFormat>,
}

/// A `Compat` with every question answered, which is what a protocol description is handed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct Resolved {
    pub chat_template_args: ChatTemplateArgs,
    pub send_session_affinity_headers: bool,
    pub session_affinity_format: SessionAffinityFormat,
    pub supports_prompt_cache_key: bool,
    pub chat_strict_dialect: ChatStrictDialect,
    pub anthropic_thinking: AnthropicThinking,
    pub google_thinking: GoogleThinking,
    pub anthropic_schema: AnthropicSchema,
    pub anthropic_replay: bool,
    pub supports_cache_control_on_tools: bool,
    pub supports_eager_tool_input_streaming: bool,
    pub allow_empty_signature: bool,
    pub supports_forced_tool_choice: bool,
    pub supports_strict_mode: bool,
    pub supports_strict_tools: bool,
    pub supports_json_schema: bool,
    pub prompt_cache: PromptCache,
    pub supports_store: bool,
    pub supports_developer_role: bool,
    pub supports_mid_convo_system_messages: bool,
    pub supports_mid_convo_tool_additions: bool,
    pub zai_tool_stream: bool,
    pub moonshot_keep_thinking: bool,
    pub supports_reasoning_effort: bool,
    pub supports_usage_in_streaming: bool,
    pub supports_finish_reason: bool,
    pub max_tokens_field: MaxTokensField,
    pub requires_tool_result_name: bool,
    pub requires_thinking_as_text: bool,
    pub requires_reasoning_content_on_assistant_messages: bool,
    pub thinking_format: ThinkingFormat,
}

impl Default for Resolved {
    /// Request defaults; JSON-schema answering is enabled unless explicitly refused.
    fn default() -> Self {
        Self {
            chat_template_args: ChatTemplateArgs::default(),
            send_session_affinity_headers: false,
            session_affinity_format: SessionAffinityFormat::Generic,
            supports_prompt_cache_key: false,
            chat_strict_dialect: ChatStrictDialect::OpenAi,
            anthropic_thinking: AnthropicThinking::Budget,
            google_thinking: GoogleThinking::Auto,
            anthropic_schema: AnthropicSchema::ForcedTool,
            anthropic_replay: false,
            supports_cache_control_on_tools: true,
            supports_eager_tool_input_streaming: false,
            allow_empty_signature: false,
            supports_forced_tool_choice: true,
            prompt_cache: PromptCache::None,
            supports_strict_mode: false,
            supports_strict_tools: false,
            supports_json_schema: true,
            supports_store: false,
            supports_developer_role: false,
            supports_mid_convo_system_messages: false,
            supports_mid_convo_tool_additions: false,
            zai_tool_stream: false,
            moonshot_keep_thinking: false,
            supports_reasoning_effort: false,
            supports_usage_in_streaming: true,
            supports_finish_reason: true,
            max_tokens_field: MaxTokensField::MaxTokens,
            requires_tool_result_name: false,
            requires_thinking_as_text: false,
            requires_reasoning_content_on_assistant_messages: false,
            thinking_format: ThinkingFormat::OpenAi,
        }
    }
}

impl Compat {
    /// Apply these overrides to the conservative defaults.
    #[must_use]
    pub fn resolve(self) -> Resolved {
        let d = Resolved::default();
        Resolved {
            chat_template_args: self.chat_template_args.unwrap_or(d.chat_template_args),
            send_session_affinity_headers: self
                .send_session_affinity_headers
                .unwrap_or(d.send_session_affinity_headers),
            session_affinity_format: self
                .session_affinity_format
                .unwrap_or(d.session_affinity_format),
            supports_prompt_cache_key: self
                .supports_prompt_cache_key
                .unwrap_or(d.supports_prompt_cache_key),
            chat_strict_dialect: self.chat_strict_dialect.unwrap_or(d.chat_strict_dialect),
            anthropic_thinking: self.anthropic_thinking.unwrap_or(d.anthropic_thinking),
            google_thinking: self.google_thinking.unwrap_or(d.google_thinking),
            anthropic_schema: self.anthropic_schema.unwrap_or(d.anthropic_schema),
            anthropic_replay: self.anthropic_replay.unwrap_or(d.anthropic_replay),
            supports_cache_control_on_tools: self
                .supports_cache_control_on_tools
                .unwrap_or(d.supports_cache_control_on_tools),
            supports_eager_tool_input_streaming: self
                .supports_eager_tool_input_streaming
                .unwrap_or(d.supports_eager_tool_input_streaming),
            allow_empty_signature: self
                .allow_empty_signature
                .unwrap_or(d.allow_empty_signature),
            supports_forced_tool_choice: self
                .supports_forced_tool_choice
                .unwrap_or(d.supports_forced_tool_choice),
            supports_strict_mode: self.supports_strict_mode.unwrap_or(d.supports_strict_mode),
            supports_strict_tools: self
                .supports_strict_tools
                .unwrap_or(d.supports_strict_tools),
            supports_json_schema: self.supports_json_schema.unwrap_or(d.supports_json_schema),
            prompt_cache: self.prompt_cache.unwrap_or(d.prompt_cache),
            supports_store: self.supports_store.unwrap_or(d.supports_store),
            supports_developer_role: self
                .supports_developer_role
                .unwrap_or(d.supports_developer_role),
            supports_mid_convo_system_messages: self
                .supports_mid_convo_system_messages
                .unwrap_or(d.supports_mid_convo_system_messages),
            supports_mid_convo_tool_additions: self
                .supports_mid_convo_tool_additions
                .unwrap_or(d.supports_mid_convo_tool_additions),
            zai_tool_stream: self.zai_tool_stream.unwrap_or(d.zai_tool_stream),
            moonshot_keep_thinking: self
                .moonshot_keep_thinking
                .unwrap_or(d.moonshot_keep_thinking),
            supports_reasoning_effort: self
                .supports_reasoning_effort
                .unwrap_or(d.supports_reasoning_effort),
            supports_usage_in_streaming: self
                .supports_usage_in_streaming
                .unwrap_or(d.supports_usage_in_streaming),
            supports_finish_reason: self
                .supports_finish_reason
                .unwrap_or(d.supports_finish_reason),
            max_tokens_field: self.max_tokens_field.unwrap_or(d.max_tokens_field),
            requires_tool_result_name: self
                .requires_tool_result_name
                .unwrap_or(d.requires_tool_result_name),
            requires_thinking_as_text: self
                .requires_thinking_as_text
                .unwrap_or(d.requires_thinking_as_text),
            requires_reasoning_content_on_assistant_messages: self
                .requires_reasoning_content_on_assistant_messages
                .unwrap_or(d.requires_reasoning_content_on_assistant_messages),
            thinking_format: self.thinking_format.unwrap_or(d.thinking_format),
        }
    }
}

impl Compat {
    /// Layer these overrides on top of `base`, field by field rather than per struct, so a model
    /// correcting one flag keeps everything else its provider said.
    #[must_use]
    pub fn over(self, base: Option<Self>) -> Self {
        let Some(base) = base else { return self };
        Self {
            chat_template_args: self.chat_template_args.or(base.chat_template_args),
            send_session_affinity_headers: self
                .send_session_affinity_headers
                .or(base.send_session_affinity_headers),
            session_affinity_format: self
                .session_affinity_format
                .or(base.session_affinity_format),
            supports_prompt_cache_key: self
                .supports_prompt_cache_key
                .or(base.supports_prompt_cache_key),
            chat_strict_dialect: self.chat_strict_dialect.or(base.chat_strict_dialect),
            anthropic_thinking: self.anthropic_thinking.or(base.anthropic_thinking),
            google_thinking: self.google_thinking.or(base.google_thinking),
            anthropic_schema: self.anthropic_schema.or(base.anthropic_schema),
            anthropic_replay: self.anthropic_replay.or(base.anthropic_replay),
            supports_cache_control_on_tools: self
                .supports_cache_control_on_tools
                .or(base.supports_cache_control_on_tools),
            supports_eager_tool_input_streaming: self
                .supports_eager_tool_input_streaming
                .or(base.supports_eager_tool_input_streaming),
            allow_empty_signature: self.allow_empty_signature.or(base.allow_empty_signature),
            supports_forced_tool_choice: self
                .supports_forced_tool_choice
                .or(base.supports_forced_tool_choice),
            prompt_cache: self.prompt_cache.or(base.prompt_cache),
            supports_strict_mode: self.supports_strict_mode.or(base.supports_strict_mode),
            supports_strict_tools: self.supports_strict_tools.or(base.supports_strict_tools),
            supports_json_schema: self.supports_json_schema.or(base.supports_json_schema),
            supports_store: self.supports_store.or(base.supports_store),
            supports_developer_role: self
                .supports_developer_role
                .or(base.supports_developer_role),
            supports_mid_convo_system_messages: self
                .supports_mid_convo_system_messages
                .or(base.supports_mid_convo_system_messages),
            supports_mid_convo_tool_additions: self
                .supports_mid_convo_tool_additions
                .or(base.supports_mid_convo_tool_additions),
            zai_tool_stream: self.zai_tool_stream.or(base.zai_tool_stream),
            moonshot_keep_thinking: self.moonshot_keep_thinking.or(base.moonshot_keep_thinking),
            supports_reasoning_effort: self
                .supports_reasoning_effort
                .or(base.supports_reasoning_effort),
            supports_usage_in_streaming: self
                .supports_usage_in_streaming
                .or(base.supports_usage_in_streaming),
            supports_finish_reason: self.supports_finish_reason.or(base.supports_finish_reason),
            max_tokens_field: self.max_tokens_field.or(base.max_tokens_field),
            requires_tool_result_name: self
                .requires_tool_result_name
                .or(base.requires_tool_result_name),
            requires_thinking_as_text: self
                .requires_thinking_as_text
                .or(base.requires_thinking_as_text),
            requires_reasoning_content_on_assistant_messages: self
                .requires_reasoning_content_on_assistant_messages
                .or(base.requires_reasoning_content_on_assistant_messages),
            thinking_format: self.thinking_format.or(base.thinking_format),
        }
    }
}

/// The dialect a model speaks, taking its declaration or the conservative default.
#[must_use]
pub fn resolve(compat: Option<Compat>) -> Resolved {
    compat.unwrap_or_default().resolve()
}

#[cfg(test)]
mod cache_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_the_conservative_reading() {
        let d = Resolved::default();
        assert_eq!(d.max_tokens_field, MaxTokensField::MaxTokens);
        assert!(
            !d.supports_store,
            "an unknown field is a 400, a missing one degrades"
        );
        assert!(!d.supports_developer_role);
        assert_eq!(d.thinking_format, ThinkingFormat::OpenAi);
    }

    #[test]
    fn an_undeclared_model_gets_the_default() {
        assert_eq!(resolve(None), Resolved::default());
    }

    #[test]
    fn a_declaration_overrides_only_what_it_names() {
        let compat = Compat {
            max_tokens_field: Some(MaxTokensField::MaxCompletionTokens),
            supports_store: Some(true),
            ..Compat::default()
        };
        let r = resolve(Some(compat));
        assert_eq!(r.max_tokens_field, MaxTokensField::MaxCompletionTokens);
        assert!(r.supports_store);
        assert!(
            !r.supports_developer_role,
            "an unnamed field keeps its default"
        );
    }

    #[test]
    fn an_empty_declaration_changes_nothing() {
        assert_eq!(resolve(Some(Compat::default())), Resolved::default());
    }

    #[test]
    fn every_thinking_dialect_can_be_declared() {
        for format in [
            ThinkingFormat::OpenAi,
            ThinkingFormat::OpenRouter,
            ThinkingFormat::DeepSeek,
            ThinkingFormat::Zai,
            ThinkingFormat::Qwen,
            ThinkingFormat::Together,
            ThinkingFormat::AntLing,
            ThinkingFormat::Baseten,
        ] {
            let r = resolve(Some(Compat {
                thinking_format: Some(format),
                ..Compat::default()
            }));
            assert_eq!(r.thinking_format, format);
        }
    }

    #[test]
    fn max_tokens_fields_name_their_wire_keys() {
        assert_eq!(MaxTokensField::MaxTokens.as_str(), "max_tokens");
        assert_eq!(
            MaxTokensField::MaxCompletionTokens.as_str(),
            "max_completion_tokens"
        );
    }
}
