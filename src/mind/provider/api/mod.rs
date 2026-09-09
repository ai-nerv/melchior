//! One adapter per wire protocol, implemented in Lua under `lua/apis/*.lua` and read through this
//! contract. Adapters are pure: `request` builds JSON and `on_event` folds one server-sent event
//! into a stream's state, with no HTTP, no sockets and no clock.

use crate::mind::model::{Context, StopReason, ThinkingLevel, Usage};
use crate::mind::provider::model::Model;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Options {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    /// A JSON Schema the answer must satisfy; each protocol names the field differently.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<Schema>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Schema {
    /// Required by some providers, and shown to nobody.
    pub name: String,
    pub schema: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Delta {
    Text(String),
    Thinking(String),
    /// Opaque provider state for the block being streamed, to be replayed verbatim.
    Signature(String),
    ToolCallStart {
        id: String,
        name: String,
    },
    /// Arguments for the tool call in progress, as raw JSON text.
    ToolCallArgs(String),
    Stop(StopReason),
    Usage(Usage),
}

#[derive(Debug, Default)]
pub struct StreamState {
    /// Whatever the adapter is keeping between events, shaped as the protocol needs.
    pub scratch: serde_json::Value,
    pub usage: Usage,
}

/// A wire protocol magi can speak, implemented in Lua rather than here. `Send + Sync` is
/// deliberately not required, since a Lua VM is neither.
pub trait Adapter {
    fn endpoint(&self, base_url: &str, model: &Model) -> String;

    fn headers(&self, key: Option<&str>) -> Vec<(String, String)>;

    fn request(&self, model: &Model, context: &Context, options: &Options) -> serde_json::Value;

    fn on_event(
        &self,
        state: &mut StreamState,
        event: &crate::mind::provider::sse::Event,
    ) -> Vec<Delta>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_default_to_asking_for_nothing_extra() {
        let options = Options::default();
        assert!(options.thinking.is_none());
        assert!(options.max_tokens.is_none());
    }

    #[test]
    fn a_fresh_stream_remembers_nothing() {
        assert!(StreamState::default().scratch.is_null());
    }
}
