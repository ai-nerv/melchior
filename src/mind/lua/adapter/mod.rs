//! A wire protocol, described in Lua: the only implementation of [`Adapter`] there is. The
//! transport moves the bytes, and every protocol is a table of functions in `lua/apis/*.lua`,
//! registered like anything else, so describing one needs no rebuild.

use crate::mind::lua::engine::Engine;
use crate::mind::model::Context;
use crate::mind::provider::api::{Adapter, Delta, Options, StreamState};
use crate::mind::provider::model::Model;
use crate::mind::provider::sse;
use std::cell::RefCell;

mod frame;
mod instructions;

#[cfg(test)]
mod measured_tests;

#[cfg(test)]
mod cache_policy;

#[cfg(test)]
mod strict_tests;

/// One registered protocol, driven through the VM.
pub struct LuaAdapter {
    /// The VM holding the description, shared with the tools that run in it: one thread, one VM.
    engine: std::rc::Rc<RefCell<Engine>>,
    /// Which protocol, by the name it registered under.
    name: String,
}

impl LuaAdapter {
    /// Speak `name` through an engine somebody else holds.
    pub fn from_shared(
        engine: std::rc::Rc<std::cell::RefCell<Engine>>,
        name: &str,
    ) -> Result<Self, String> {
        let known = engine.borrow_mut().apis();
        if !known.iter().any(|a| a == name) {
            return Err(format!(
                "no protocol named {name:?} is registered; magi knows {}",
                if known.is_empty() {
                    "none".to_owned()
                } else {
                    known.join(", ")
                }
            ));
        }
        Ok(Self {
            engine,
            name: name.to_owned(),
        })
    }

    /// Take ownership of an engine and speak `name` through it.
    pub fn new(mut engine: Engine, name: &str) -> Result<Self, String> {
        let known = engine.apis();
        if !known.iter().any(|a| a == name) {
            return Err(format!(
                "no protocol named {name:?} is registered; magi knows {}",
                if known.is_empty() {
                    "none".to_owned()
                } else {
                    known.join(", ")
                }
            ));
        }
        Ok(Self {
            engine: std::rc::Rc::new(RefCell::new(engine)),
            name: name.to_owned(),
        })
    }

    fn call(&self, method: &str, args: &[serde_json::Value]) -> Option<serde_json::Value> {
        self.engine.borrow_mut().call_api(&self.name, method, args)
    }
}

/// The model as a protocol description sees it, its `compat` replaced by the fully resolved
/// dialect so that every default lives in `crate::mind::provider::compat::Resolved` alone and a
/// description never writes `compat.thinking_format or "openai"`.
fn described(model: &Model) -> serde_json::Value {
    let mut value = serde_json::to_value(model).unwrap_or_default();
    if let Some(object) = value.as_object_mut() {
        let resolved = crate::mind::provider::compat::resolve(model.compat);
        object.insert(
            "compat".to_owned(),
            serde_json::to_value(resolved).unwrap_or_default(),
        );
    }
    value
}

/// The options as this model can actually take them. `thinking` is settled here rather than in a
/// description because Lua cannot tell a key with no value from an absent key, and the catalog's
/// "cannot do this level" and "no opinion" are different answers.
fn asked(model: &Model, options: &Options) -> serde_json::Value {
    let mut value = serde_json::to_value(options).unwrap_or_default();
    let Some(object) = value.as_object_mut() else {
        return value;
    };
    object.remove("session_id");
    if let Some(session) = &options.provider_session {
        object.insert("__session_id".into(), session.clone().into());
    }
    if let Some(binding) = &options.continuation_binding {
        object.insert("__credential_binding".into(), binding.clone().into());
    }
    let effective = effective_thinking(model, options);
    if let Some(control) =
        crate::mind::provider::thinking::google::configuration(model, options.thinking)
    {
        object.insert("__google_control".into(), control);
    }
    if crate::mind::provider::compat::resolve(model.compat).thinking_format
        == crate::mind::provider::compat::ThinkingFormat::Baseten
        && (options.thinking.is_none() || effective.is_some())
    {
        object.insert(
            "__thinking_enabled".into(),
            serde_json::Value::Bool(
                options
                    .thinking
                    .is_some_and(|level| level != crate::mind::model::ThinkingLevel::Off),
            ),
        );
    }
    match effective {
        Some(level) => {
            object.insert("thinking".to_owned(), serde_json::Value::String(level));
        }
        None => {
            object.remove("thinking");
        }
    }
    value
}

/// Resolve requested reasoning through the model's capabilities and protocol.
fn effective_thinking(model: &Model, options: &Options) -> Option<String> {
    use crate::mind::provider::{compat::AnthropicThinking, model::Api};
    if model.api == Api::MistralConversations {
        return crate::mind::provider::thinking::mistral::requested(model, options.thinking);
    }
    let level = options.thinking?;
    let effective = crate::mind::provider::thinking::effective(model, level)?;
    if model.api == Api::AnthropicMessages
        && level == crate::mind::model::ThinkingLevel::Off
        && matches!(effective.as_str(), "off" | "none")
        && crate::mind::provider::compat::resolve(model.compat).anthropic_thinking
            != AnthropicThinking::Budget
    {
        Some("off".into())
    } else {
        Some(effective)
    }
}

impl Adapter for LuaAdapter {
    fn endpoint(&self, base_url: &str, model: &Model) -> String {
        self.call(
            "endpoint",
            &[
                serde_json::json!(base_url.trim_end_matches('/')),
                described(model),
            ],
        )
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| base_url.to_owned())
    }

    fn headers(&self, key: Option<&str>) -> Vec<(String, String)> {
        // Content type is set here rather than in every protocol, since all of them post JSON.
        let mut out = vec![("content-type".to_owned(), "application/json".to_owned())];
        let Some(fields) = self
            .call("headers", &[serde_json::json!(key)])
            .and_then(|v| v.as_object().cloned())
        else {
            return out;
        };
        for (name, value) in fields {
            if let Some(value) = value.as_str() {
                out.push((name, value.to_owned()));
            }
        }
        out
    }

    fn request_endpoint(
        &self,
        base_url: &str,
        model: &Model,
        body: &serde_json::Value,
    ) -> Result<String, String> {
        let mut projected = body.get("tools").cloned().unwrap_or_default();
        if self.name == "openai-completions" {
            for added in body
                .get("messages")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter(|message| message["role"] == "system")
                .filter_map(|message| message.get("tools").and_then(serde_json::Value::as_array))
            {
                if projected.is_null() {
                    projected = serde_json::json!([]);
                }
                projected
                    .as_array_mut()
                    .ok_or("invalid prepared tool declarations")?
                    .extend(added.iter().cloned());
            }
        }
        self.call(
            "endpoint",
            &[
                serde_json::json!(base_url.trim_end_matches('/')),
                described(model),
                serde_json::json!({ "tools": projected }),
            ],
        )
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or_else(|| format!("{}: request endpoint conversion failed", self.name))
    }

    fn request(&self, model: &Model, context: &Context, options: &Options) -> serde_json::Value {
        let mut body = self.request_observed(model, context, options).0;
        if let Some(fields) = body.as_object_mut() {
            fields.remove("__budget");
        }
        body
    }

    fn request_observed(
        &self,
        model: &Model,
        context: &Context,
        options: &Options,
    ) -> (serde_json::Value, Option<crate::mind::model::RequestFrame>) {
        self.prepare(model, context, options).unwrap_or_default()
    }

    fn prepare(
        &self,
        model: &Model,
        context: &Context,
        options: &Options,
    ) -> Result<(serde_json::Value, Option<crate::mind::model::RequestFrame>), String> {
        if matches!(
            self.name.as_str(),
            "openai-completions"
                | "mistral-conversations"
                | "openai-responses"
                | "azure-openai-responses"
                | "openai-codex-responses"
                | "anthropic-messages"
        ) && options.schema.is_some()
            && !crate::mind::provider::compat::resolve(model.compat).supports_json_schema
        {
            return Err("this model does not support JSON-schema structured answers".into());
        }
        if self.name == "anthropic-messages"
            && options.schema.is_some()
            && crate::mind::provider::compat::resolve(model.compat).anthropic_schema
                == crate::mind::provider::compat::AnthropicSchema::ForcedTool
            && !crate::mind::provider::compat::resolve(model.compat).supports_forced_tool_choice
        {
            return Err(
                "this model does not support forced tool selection for structured answers".into(),
            );
        }
        let mut context = instructions::context(&self.name, model, context)?;
        let mut omitted_images = 0_u64;
        if !model.accepts_images() {
            use crate::mind::model::Content;
            const OMITTED: &str = "[Image omitted: the current model does not support images. Switch to a vision-capable model to view it.]";
            for message in &mut context.messages {
                for content in &mut message.content {
                    match content {
                        Content::Image { .. } => {
                            omitted_images = omitted_images.saturating_add(1);
                            *content = Content::Text {
                                text: OMITTED.into(),
                                signature: None,
                            }
                        }
                        Content::ToolResult {
                            images, content, ..
                        } if !images.is_empty() => {
                            omitted_images = omitted_images
                                .saturating_add(u64::try_from(images.len()).unwrap_or(u64::MAX));
                            images.clear();
                            content.push('\n');
                            content.push_str(OMITTED);
                        }
                        _ => {}
                    }
                }
            }
        }
        let encoded = crate::mind::provider::strict::context(&self.name, model, &context, options)?;
        let mut description = described(model);
        let fields = description
            .as_object_mut()
            .ok_or_else(|| format!("{}: invalid model description", self.name))?;
        if let Some(base) = model.base_url.as_deref() {
            fields.insert("__endpoint".into(), self.endpoint(base, model).into());
        }
        fields.insert("__protocol".into(), self.name.clone().into());
        let body = self
            .call("request", &[description, encoded, asked(model, options)])
            .filter(|value| value.is_object())
            .ok_or_else(|| format!("{}: request conversion failed", self.name))?;
        Ok((
            body,
            frame::observed(&self.name, model, options, omitted_images),
        ))
    }

    fn on_event(&self, state: &mut StreamState, event: &sse::Event) -> Vec<Delta> {
        self.on_event_measured(state, event)
            .into_iter()
            .map(|(delta, _)| delta)
            .collect()
    }

    fn on_event_measured(
        &self,
        state: &mut StreamState,
        event: &sse::Event,
    ) -> Vec<(Delta, crate::mind::model::MeasuredUsage)> {
        let answer = self.call(
            "on_event",
            &[
                serde_json::json!({ "scratch": state.scratch, "usage": state.usage, "measured_usage": state.measured_usage }),
                serde_json::json!({ "name": event.name, "data": event.data }),
            ],
        );
        let Some(answer) = answer else {
            return Vec::new();
        };

        // The protocol hands back what it wants remembered between events, and the deltas.
        if let Some(scratch) = answer.get("scratch") {
            state.scratch = scratch.clone();
        }
        let mut anomalies = answer
            .get("usage_anomalies")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        if let Some(value) = answer.get("usage") {
            let (usage, invalid) = crate::mind::model::Usage::measured(value, state.usage);
            state.usage = usage;
            state.measured_usage.observe(None, usage);
            anomalies = anomalies.saturating_add(invalid);
        }
        let deltas = answer
            .get("deltas")
            .and_then(|d| d.as_array())
            .map(|deltas| {
                deltas
                    .iter()
                    .filter_map(|value| {
                        if value.get("kind").and_then(serde_json::Value::as_str) == Some("usage") {
                            let (usage, invalid) = crate::mind::model::Usage::measured(
                                value.get("usage")?,
                                state.usage,
                            );
                            state.usage = usage;
                            state.measured_usage.observe(value.get("measured"), usage);
                            anomalies = anomalies.saturating_add(invalid);
                            Some((Delta::Usage(usage), state.measured_usage))
                        } else {
                            delta_from_json(value).map(|delta| (delta, state.measured_usage))
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        if anomalies > 0 {
            state.usage_anomalies = state.usage_anomalies.saturating_add(anomalies);
            crate::noted!("usage: {} ignored {anomalies} invalid metric(s)", self.name);
        }
        deltas
    }
}

/// One delta, as a protocol described it. An unrecognised kind is dropped rather than fatal, so
/// a protocol reporting something this build has no word for does not lose the turn.
fn delta_from_json(value: &serde_json::Value) -> Option<Delta> {
    let text = |key: &str| {
        value
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_owned()
    };
    Some(match value.get("kind").and_then(|k| k.as_str())? {
        "text" => Delta::Text(text("text")),
        "thinking" => Delta::Thinking(text("thinking")),
        "signature" => Delta::Signature(text("signature")),
        "tool_call_start" => Delta::ToolCallStart {
            id: text("id"),
            name: text("name"),
        },
        "tool_call_args" => Delta::ToolCallArgs(text("arguments")),
        "stop" => Delta::Stop(serde_json::from_value(value.get("reason")?.clone()).ok()?),
        _ => return None,
    })
}

/// An engine with these protocol descriptions registered. They are read as configuration rather
/// than compiled in, so fixing a wire format is not a rebuild.
pub fn engine_with(sources: &[(String, String)]) -> Result<Engine, crate::mind::lua::LuaError> {
    let mut engine = Engine::new();
    for (name, source) in sources {
        engine.run(source, name)?;
    }
    Ok(engine)
}

/// Protocols magi knows of but does not speak, and why; named rather than simply absent, so a
/// model in the catalog that does nothing can say what is missing.
pub const UNSPOKEN: &[(&str, &str)] = &[(
    "bedrock-converse-stream",
    "Bedrock frames its stream as binary AWS eventstream records rather than server-sent \
     events, so it needs a decoder in the transport before a description can read it. That \
     is Rust work, not a protocol file.",
)];

/// Why a protocol is not spoken, if it is a known gap.
#[must_use]
pub fn why_unspoken(api: &str) -> Option<&'static str> {
    UNSPOKEN
        .iter()
        .find(|(name, _)| *name == api)
        .map(|(_, why)| *why)
}

/// The protocol descriptions in the checkout, read at run time. For tests only: the product
/// reads its configuration from the config directory and carries no copy.
pub fn shipped_apis() -> Result<Vec<(String, String)>, crate::mind::lua::LuaError> {
    const PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/config/apis.lua");
    let source =
        std::fs::read_to_string(PATH).map_err(|source| crate::mind::lua::LuaError::Io {
            file: PATH.to_owned(),
            source,
        })?;
    Ok(vec![("apis".to_owned(), source)])
}

/// An engine with those protocols registered. For tests.
pub fn engine_with_builtins() -> Result<Engine, crate::mind::lua::LuaError> {
    engine_with(&shipped_apis()?)
}

#[cfg(test)]
#[path = "images.rs"]
mod images;

#[cfg(test)]
mod anthropic_tests;
#[cfg(test)]
mod cache_conversation;
#[cfg(test)]
mod cache_markers;
#[cfg(test)]
mod cache_tests;
#[cfg(test)]
mod deciding;
#[cfg(test)]
mod fidelity_tests;
#[cfg(test)]
mod google_calls;
#[cfg(test)]
mod google_replay;
#[cfg(test)]
mod google_thinking;

#[cfg(test)]
mod google_usage;
#[cfg(test)]
mod mistral;

#[cfg(test)]
mod protocols;
#[cfg(test)]
mod reasoning_content_tests;
#[cfg(test)]
mod zai;

#[cfg(test)]
mod moonshot;

#[cfg(test)]
mod baseten;

#[cfg(test)]
mod fireworks;

#[cfg(test)]
mod qwen;

#[cfg(test)]
mod together;

#[cfg(test)]
mod ant_ling;

#[cfg(test)]
mod completions_tests;
#[cfg(test)]
mod responses_tests;
#[cfg(test)]
mod support;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod usage_tests;

#[cfg(test)]
mod thinking_tests {
    use super::*;
    use crate::mind::model::ThinkingLevel;
    use std::collections::BTreeMap;

    fn model(reasoning: bool, thinking: BTreeMap<ThinkingLevel, Option<String>>) -> Model {
        Model {
            id: "m".into(),
            name: "M".into(),
            provider: "p".into(),
            api: crate::mind::provider::model::Api::OpenAiCompletions,
            base_url: None,
            headers: BTreeMap::new(),
            reasoning,
            input: Vec::new(),
            input_limits: None,
            context_window: 1000,
            max_tokens: 100,
            cost: crate::mind::model::Cost::default(),
            pricing: None,
            popularity_rank: None,
            capabilities: None,
            thinking,
            compat: None,
        }
    }

    fn wanting(level: ThinkingLevel) -> Options {
        Options {
            schema: None,
            thinking: Some(level),
            max_tokens: None,
            provider: None,
            avoid: Vec::new(),
            ..Default::default()
        }
    }

    #[test]
    fn a_model_that_does_not_reason_is_never_asked_to() {
        let asked = asked(
            &model(false, BTreeMap::new()),
            &wanting(ThinkingLevel::High),
        );
        assert!(asked.get("thinking").is_none(), "{asked}");
    }

    #[test]
    fn an_unmapped_level_goes_through_as_the_provider_names_it() {
        let asked = asked(&model(true, BTreeMap::new()), &wanting(ThinkingLevel::High));
        assert_eq!(asked["thinking"], "high");
    }

    #[test]
    fn a_model_may_call_a_level_something_else() {
        let mut map = BTreeMap::new();
        map.insert(ThinkingLevel::High, Some("deep".to_owned()));
        let asked = asked(&model(true, map), &wanting(ThinkingLevel::High));
        assert_eq!(asked["thinking"], "deep");
    }

    #[test]
    fn a_level_the_model_refuses_is_not_asked_for() {
        let mut map = BTreeMap::new();
        map.insert(ThinkingLevel::Max, None);
        let asked = asked(&model(true, map), &wanting(ThinkingLevel::Max));
        assert!(asked.get("thinking").is_none(), "{asked}");
    }

    /// Off is said to every model, including one the catalog believes cannot reason.
    ///
    /// That belief is the provider's word copied into a declaration, and it is wrong often
    /// enough to matter: a helper allowed 1000 tokens spent 999 of them reasoning and answered
    /// nothing, on a model whose catalog entry said `reasoning = false`. A field a model
    /// ignores costs nothing; the silence cost the whole answer.
    #[test]
    fn a_model_that_must_reason_is_asked_for_the_least_rather_than_for_nothing() {
        // The one exception to the rule below. Told to stop, it refuses the whole request; told
        // nothing, it reasons as much as it likes and a helper's budget goes on that alone.
        let mut map = BTreeMap::new();
        map.insert(ThinkingLevel::Off, Some("low".to_owned()));
        let asked = asked(&model(true, map), &wanting(ThinkingLevel::Off));
        assert_eq!(asked["thinking"], "low", "{asked}");
    }

    #[test]
    fn a_level_marked_absent_outright_is_still_not_asked_for() {
        let mut map = BTreeMap::new();
        map.insert(ThinkingLevel::Off, None);
        let asked = asked(&model(true, map), &wanting(ThinkingLevel::Off));
        assert!(asked.get("thinking").is_none(), "{asked}");
    }

    #[test]
    fn off_is_said_to_every_model_so_one_that_reasons_can_stop() {
        for reasons in [true, false] {
            let asked = asked(
                &model(reasons, BTreeMap::new()),
                &wanting(ThinkingLevel::Off),
            );
            assert_eq!(asked["thinking"], "off", "reasoning = {reasons}: {asked}");
        }
    }

    #[test]
    fn asking_for_nothing_asks_for_nothing() {
        let asked = asked(&model(true, BTreeMap::new()), &Options::default());
        assert!(asked.get("thinking").is_none(), "{asked}");
    }
}
