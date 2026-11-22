use anyhow::{Context, Result, ensure};
use melchior::mind::model::ThinkingLevel;
use melchior::mind::provider::compat::Compat;
use melchior::mind::provider::model::Model;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub mod input;
mod limits;
mod pricing;

pub const OUTPUT: &str = "catalog/generated/pi-1.6.0.json";
const MANIFEST: &str = include_str!("../../tests/provider-parity/manifest.json");
const SOURCE: &str = "catalog/reference/pi-1.6.0/data";

pub fn generate(root: &Path) -> Result<Vec<u8>> {
    let manifest: Value = serde_json::from_str(MANIFEST)?;
    let sources = input::read(&root.join(SOURCE), &manifest)?;
    let mut providers = Vec::new();
    let mut requirements = BTreeMap::<String, usize>::new();
    let mut operations = BTreeMap::<String, usize>::new();
    let mut complete_shapes = 0;
    let mut excluded_images = 0;
    for (id, rows) in sources {
        let mut models = Vec::new();
        let mut excluded = Vec::new();
        for row in rows {
            if input::text(&row, "type")? == "image" {
                excluded.push(json!({"operation":"image","id":input::text(&row,"id")?,"api":input::text(&row,"api")?}));
                excluded_images += 1;
                continue;
            }
            let converted = convert(&row).with_context(|| format!("provider {id}"))?;
            *operations
                .entry(input::text(&row, "type")?.to_owned())
                .or_default() += 1;
            let pending = converted
                .get("pending")
                .and_then(Value::as_array)
                .context("pending mappings")?;
            if pending.is_empty() {
                complete_shapes += 1;
            }
            for field in pending {
                *requirements
                    .entry(field.as_str().context("pending field")?.to_owned())
                    .or_default() += 1;
            }
            models.push(converted);
        }
        models.sort_by_key(|model| {
            model
                .get("source_key")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        });
        excluded.sort_by_key(|model| {
            model
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        });
        let source = manifest
            .get("providers")
            .and_then(Value::as_array)
            .and_then(|rows| {
                rows.iter()
                    .find(|row| row.get("id").and_then(Value::as_str) == Some(id.as_str()))
            })
            .and_then(|row| row.pointer("/models/catalog"))
            .context("provider source provenance")?;
        providers.push(json!({"id":id,"source":source,"activation":"not-reviewed","models":models,"excluded":excluded}));
    }
    let mut generated = json!({"schema":1,"kind":"model-candidates-not-runtime-catalog",
        "reference":manifest.get("reference").context("reference")?,
        "pricing_policy":"Published positive rates are retained; zero rates are unknown unless the source explicitly declares the model free.",
        "summary":{"providers":providers.len(),"operations":operations,"excluded_images":excluded_images,
            "complete_model_shapes":complete_shapes,"activated_providers":0,"pending_mappings":requirements},
        "providers":providers});
    generated.sort_all_objects();
    let mut bytes = serde_json::to_vec_pretty(&generated)?;
    bytes.push(b'\n');
    Ok(bytes)
}

pub fn convert(raw: &Value) -> Result<Value> {
    let operation = input::text(raw, "type")?;
    ensure!(
        matches!(operation, "chat" | "classifier"),
        "operation is outside catalog scope"
    );
    let id = input::text(raw, "id")?;
    let mut model = Map::new();
    let mut pending = BTreeSet::new();
    let mut consumed = BTreeSet::from([
        "type",
        "cost",
        "free",
        "thinkingLevelMap",
        "compat",
        "inputLimits",
    ]);
    for (upstream, native) in [
        ("id", "id"),
        ("name", "name"),
        ("provider", "provider"),
        ("api", "api"),
        ("baseUrl", "base_url"),
        ("headers", "headers"),
        ("reasoning", "reasoning"),
        ("input", "input"),
        ("contextWindow", "context_window"),
        ("maxTokens", "max_tokens"),
    ] {
        consumed.insert(upstream);
        if let Some(value) = raw.get(upstream) {
            model.insert(native.into(), value.clone());
        }
    }
    if let Some(value) = raw.get("free") {
        ensure!(value.is_boolean(), "free must be boolean");
    }
    if let Some(cost) = raw.get("cost") {
        model.insert(
            "cost".into(),
            pricing::convert(
                cost,
                raw.get("free") == Some(&Value::Bool(true)),
                &mut pending,
            )?,
        );
    }
    if let Some(thinking) = raw.get("thinkingLevelMap") {
        let mut mapped = Map::new();
        for (level, value) in thinking
            .as_object()
            .context("thinking map must be an object")?
        {
            ensure!(
                value.is_null() || value.is_string(),
                "thinking map value must be text or null"
            );
            if serde_json::from_value::<ThinkingLevel>(json!(level)).is_ok() {
                mapped.insert(level.clone(), value.clone());
            } else {
                pending.insert(format!("thinkingLevelMap.{level}"));
            }
        }
        model.insert("thinking".into(), Value::Object(mapped));
    }
    if let Some(compat) = raw.get("compat") {
        model.insert("compat".into(), compatibility(compat, raw, &mut pending)?);
    } else {
        let defaults = reviewed_defaults(raw);
        if !defaults.is_empty() {
            model.insert("compat".into(), Value::Object(defaults));
        }
    }
    if let Some(limits) = raw.get("inputLimits") {
        model.insert(
            "input_limits".into(),
            limits::convert(limits, &mut pending)?,
        );
    }
    for field in raw.as_object().context("model must be an object")?.keys() {
        if !consumed.contains(field.as_str()) {
            pending.insert(field.clone());
        }
    }
    if operation == "classifier" {
        pending.insert("selection.operation".into());
    }
    match serde_json::from_value::<Model>(Value::Object(model.clone())) {
        Err(_) => {
            pending.insert("native_model_shape".into());
        }
        Ok(native) => {
            if matches!(
                native.api,
                melchior::mind::provider::model::Api::BedrockConverseStream
                    | melchior::mind::provider::model::Api::Decisions
            ) && native
                .input_limits
                .as_ref()
                .and_then(|limits| limits.images.as_ref())
                .is_some_and(|images| {
                    images.max_per_message.is_some()
                        || images.max_per_request.is_some()
                        || images
                            .resize
                            .as_ref()
                            .and_then(|resize| resize.max_bytes)
                            .is_some()
                })
            {
                pending.insert("inputLimits.images:adapter".into());
            }
            let compat = native.compat.unwrap_or_default();
            let strict = melchior::mind::provider::thinking::capabilities(&native)
                .and_then(|caps| caps.strict_tools)
                == Some(true);
            for (field, enabled) in [
                ("supportsStrictMode", compat.supports_strict_mode),
                ("supportsStrictTools", compat.supports_strict_tools),
            ] {
                if enabled == Some(true) && !strict {
                    pending.insert(format!("compat.{field}:adapter"));
                }
            }
            if let Some(caps) = melchior::mind::provider::thinking::capabilities(&native) {
                model.insert("capabilities".into(), serde_json::to_value(caps)?);
            }
            if native
                .thinking
                .get(&ThinkingLevel::XHigh)
                .is_some_and(Option::is_some)
                && melchior::mind::provider::thinking::effective(&native, ThinkingLevel::XHigh)
                    .is_none()
            {
                pending.insert("thinkingLevelMap.xhigh:adapter".into());
            }
            if matches!(
                native.api,
                melchior::mind::provider::model::Api::MistralConversations
                    | melchior::mind::provider::model::Api::GoogleGenerativeAi
                    | melchior::mind::provider::model::Api::GoogleVertex
            ) {
                for (level, mapped) in &native.thinking {
                    if mapped.is_some()
                        && melchior::mind::provider::thinking::effective(&native, *level).is_none()
                    {
                        pending.insert(format!("thinkingLevelMap.{}:adapter", level.as_str()));
                    }
                }
            }
        }
    }
    Ok(
        json!({"id":id,"operation":operation,"api":input::text(raw,"api")?,
        "source_key":format!("{operation}:{id}"),"candidate":model,"pending":pending}),
    )
}

fn reviewed_defaults(model: &Value) -> Map<String, Value> {
    if model.get("api").and_then(Value::as_str) == Some("mistral-conversations") {
        Map::from_iter([
            ("supports_strict_mode".into(), json!(false)),
            ("supports_mid_convo_system_messages".into(), json!(true)),
            ("supports_prompt_cache_key".into(), json!(true)),
            ("send_session_affinity_headers".into(), json!(true)),
        ])
    } else if model.get("provider").and_then(Value::as_str) == Some("huggingface")
        && model.get("api").and_then(Value::as_str) == Some("openai-completions")
        && model.get("baseUrl").and_then(Value::as_str) == Some("https://router.huggingface.co/v1")
    {
        Map::from_iter([
            ("supports_reasoning_effort".into(), json!(true)),
            ("supports_store".into(), json!(true)),
            ("max_tokens_field".into(), json!("max_completion_tokens")),
        ])
    } else {
        Map::new()
    }
}

fn compatibility(raw: &Value, model: &Value, pending: &mut BTreeSet<String>) -> Result<Value> {
    let api = input::text(model, "api")?;
    let mut mapped = reviewed_defaults(model);
    if matches!(api, "openai-completions" | "anthropic-messages")
        && raw.get("sendSessionAffinityHeaders") == Some(&Value::Bool(true))
        && raw.get("sessionAffinityFormat").is_none()
    {
        let router = model.get("provider").and_then(Value::as_str) == Some("openrouter")
            || model
                .get("baseUrl")
                .and_then(Value::as_str)
                .is_some_and(|url| url.contains("openrouter.ai"));
        mapped.insert(
            "session_affinity_format".into(),
            json!(if router {
                "openrouter"
            } else if api == "anthropic-messages" {
                "generic"
            } else {
                "openai"
            }),
        );
    }
    for (field, value) in raw.as_object().context("compat must be an object")? {
        if field == "chatTemplateArgs" {
            if api == "openai-completions"
                && raw.get("thinkingFormat").and_then(Value::as_str) == Some("baseten")
                && serde_json::from_value::<melchior::mind::provider::compat::ChatTemplateArgs>(
                    value.clone(),
                )
                .is_ok()
            {
                mapped.insert("chat_template_args".into(), value.clone());
            } else {
                pending.insert(format!("compat.{field}"));
            }
            continue;
        }
        if field == "thinkingFormat" && value == "baseten" && api != "openai-completions" {
            pending.insert(format!("compat.{field}"));
            continue;
        }
        if field == "sendSessionAffinityHeaders" {
            if value.is_boolean()
                && (matches!(api, "openai-completions" | "anthropic-messages")
                    || value == &Value::Bool(false))
            {
                mapped.insert("send_session_affinity_headers".into(), value.clone());
            } else {
                pending.insert(format!("compat.{field}"));
            }
            continue;
        }
        if field == "sessionAffinityFormat" {
            if matches!(api, "openai-completions" | "anthropic-messages") && serde_json::from_value::<melchior::mind::provider::compat::SessionAffinityFormat>(value.clone()).is_ok() {
                mapped.insert("session_affinity_format".into(), value.clone());
            } else {
                pending.insert(format!("compat.{field}"));
            }
            continue;
        }
        if matches!(
            field.as_str(),
            "supportsCacheControlOnTools" | "supportsEagerToolInputStreaming"
        ) {
            if api == "anthropic-messages" && value.is_boolean() {
                mapped.insert(
                    if field == "supportsCacheControlOnTools" {
                        "supports_cache_control_on_tools"
                    } else {
                        "supports_eager_tool_input_streaming"
                    }
                    .into(),
                    value.clone(),
                );
            } else {
                pending.insert(format!("compat.{field}"));
            }
            continue;
        }
        if field == "supportsMidConvoToolAdditions" {
            if value.is_boolean() && (api == "openai-completions" || value == &Value::Bool(false)) {
                mapped.insert("supports_mid_convo_tool_additions".into(), value.clone());
            } else {
                pending.insert(format!("compat.{field}"));
            }
            continue;
        }
        if field == "zaiToolStream" {
            if value.is_boolean() && (api == "openai-completions" || value == &Value::Bool(false)) {
                mapped.insert("zai_tool_stream".into(), value.clone());
            } else {
                pending.insert(format!("compat.{field}"));
            }
            continue;
        }
        if field == "supportsMidConvoSystemMessages" {
            if value.is_boolean()
                && (matches!(api, "openai-completions" | "mistral-conversations")
                    || value == &Value::Bool(false))
            {
                mapped.insert("supports_mid_convo_system_messages".into(), value.clone());
            } else {
                pending.insert(format!("compat.{field}"));
            }
            continue;
        }
        if field == "requiresReasoningContentOnAssistantMessages" {
            if api == "openai-completions" && value.is_boolean() {
                mapped.insert(
                    "requires_reasoning_content_on_assistant_messages".into(),
                    value.clone(),
                );
            } else {
                pending.insert(format!("compat.{field}"));
            }
            continue;
        }
        if matches!(
            field.as_str(),
            "forceAdaptiveThinking" | "allowEmptySignature"
        ) {
            if api == "anthropic-messages" && value.is_boolean() {
                if field == "forceAdaptiveThinking" {
                    mapped.insert(
                        "anthropic_thinking".into(),
                        json!(if value.as_bool() == Some(true) {
                            "adaptive_effort"
                        } else {
                            "budget"
                        }),
                    );
                } else {
                    mapped.insert("allow_empty_signature".into(), value.clone());
                }
            } else {
                pending.insert(format!("compat.{field}"));
            }
            continue;
        }
        if field == "supportsLongCacheRetention"
            && value == &Value::Bool(false)
            && matches!(
                api,
                "openai-completions" | "openai-responses" | "anthropic-messages"
            )
        {
            continue;
        }
        let native = match field.as_str() {
            "supportsStrictMode" => "supports_strict_mode",
            "supportsStrictTools" => "supports_strict_tools",
            "supportsStore" => "supports_store",
            "supportsDeveloperRole" => "supports_developer_role",
            "supportsReasoningEffort" => "supports_reasoning_effort",
            "supportsUsageInStreaming" => "supports_usage_in_streaming",
            "supportsFinishReason" => "supports_finish_reason",
            "maxTokensField" => "max_tokens_field",
            "requiresToolResultName" => "requires_tool_result_name",
            "requiresThinkingAsText" => "requires_thinking_as_text",
            "thinkingFormat" => "thinking_format",
            _ => {
                pending.insert(format!("compat.{field}"));
                continue;
            }
        };
        let probe = Value::Object(Map::from_iter([(native.to_owned(), value.clone())]));
        if serde_json::from_value::<Compat>(probe).is_ok() {
            mapped.insert(native.into(), value.clone());
        } else {
            pending.insert(format!("compat.{field}"));
        }
    }
    Ok(Value::Object(mapped))
}
