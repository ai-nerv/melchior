use crate::mind::model::{ImageInputLimits, InputLimits};
use crate::mind::provider::model::Api;
use serde_json::Value;
use std::io::{self, Write};
use std::num::NonZeroU64;

pub(super) fn check(api: Api, limits: &InputLimits, body: &Value) -> Result<(), &'static str> {
    if let Some(maximum) = limits.max_request_bytes {
        let mut remaining = Remaining(maximum.get());
        serde_json::to_writer(&mut remaining, body)
            .map_err(|_| "serialized request exceeds model input byte limit")?;
    }
    if let Some(images) = &limits.images {
        images_in(api, images, body)?;
    }
    Ok(())
}

struct Remaining(u64);

impl Write for Remaining {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let size = u64::try_from(bytes.len()).map_err(|_| io::ErrorKind::FileTooLarge)?;
        self.0 = self
            .0
            .checked_sub(size)
            .ok_or(io::ErrorKind::FileTooLarge)?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn images_in(api: Api, limits: &ImageInputLimits, body: &Value) -> Result<(), &'static str> {
    let payload_limit = limits.resize.as_ref().and_then(|resize| resize.max_bytes);
    if limits.max_per_message.is_none()
        && limits.max_per_request.is_none()
        && payload_limit.is_none()
    {
        return Ok(());
    }
    let field = match api {
        Api::AnthropicMessages
        | Api::PiMessages
        | Api::OpenAiCompletions
        | Api::MistralConversations => "messages",
        Api::OpenAiResponses | Api::AzureOpenAiResponses | Api::OpenAiCodexResponses => "input",
        Api::GoogleGenerativeAi | Api::GoogleVertex => "contents",
        Api::BedrockConverseStream | Api::Decisions => {
            return Err("image input-limit validation is unavailable for this protocol");
        }
    };
    let messages = body
        .get(field)
        .and_then(Value::as_array)
        .ok_or("provider messages are unavailable for image input-limit validation")?;
    let mut total = 0_u64;
    for message in messages {
        let mut count = Count {
            images: 0,
            payload_limit,
        };
        match api {
            Api::AnthropicMessages | Api::PiMessages => {
                let shape = if api == Api::PiMessages {
                    ("/data", "toolResult")
                } else {
                    ("/source/data", "tool_result")
                };
                message_blocks(message.get("content"), &mut count, shape, true)?;
            }
            Api::OpenAiCompletions | Api::MistralConversations => {
                for block in array(message.get("content")) {
                    if block.get("type").and_then(Value::as_str) == Some("image_url") {
                        count.image(data_url(if api == Api::MistralConversations {
                            block.get("image_url")
                        } else {
                            block.pointer("/image_url/url")
                        }))?;
                    }
                }
            }
            Api::OpenAiResponses | Api::AzureOpenAiResponses | Api::OpenAiCodexResponses => {
                let field = if message.get("type").and_then(Value::as_str)
                    == Some("function_call_output")
                {
                    "output"
                } else {
                    "content"
                };
                for block in array(message.get(field)) {
                    if block.get("type").and_then(Value::as_str) == Some("input_image") {
                        count.image(data_url(block.get("image_url")))?;
                    }
                }
            }
            Api::GoogleGenerativeAi | Api::GoogleVertex => {
                for part in array(message.get("parts")) {
                    for field in ["inlineData", "fileData"] {
                        if let Some(data) = part.get(field).filter(|data| {
                            data.get("mimeType")
                                .and_then(Value::as_str)
                                .is_some_and(|mime| mime.starts_with("image/"))
                        }) {
                            count.image(data.get("data").and_then(Value::as_str))?;
                        }
                    }
                }
            }
            Api::BedrockConverseStream | Api::Decisions => unreachable!(),
        }
        if exceeds(limits.max_per_message, count.images) {
            return Err("provider message exceeds model image count limit");
        }
        total = total
            .checked_add(count.images)
            .ok_or("image count overflow")?;
        if exceeds(limits.max_per_request, total) {
            return Err("provider request exceeds model image count limit");
        }
    }
    Ok(())
}

fn array(value: Option<&Value>) -> &[Value] {
    value.and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

fn message_blocks(
    blocks: Option<&Value>,
    count: &mut Count,
    shape: (&str, &str),
    nested: bool,
) -> Result<(), &'static str> {
    for block in array(blocks) {
        match block.get("type").and_then(Value::as_str) {
            Some("image") => count.image(block.pointer(shape.0).and_then(Value::as_str))?,
            Some(kind) if nested && kind == shape.1 => {
                message_blocks(block.get("content"), count, shape, false)?
            }
            _ => {}
        }
    }
    Ok(())
}

fn data_url(value: Option<&Value>) -> Option<&str> {
    let (prefix, payload) = value?.as_str()?.split_once(',')?;
    (prefix.starts_with("data:") && prefix.ends_with(";base64")).then_some(payload)
}

fn exceeds(maximum: Option<NonZeroU64>, actual: u64) -> bool {
    maximum.is_some_and(|maximum| actual > maximum.get())
}

struct Count {
    images: u64,
    payload_limit: Option<NonZeroU64>,
}

impl Count {
    fn image(&mut self, payload: Option<&str>) -> Result<(), &'static str> {
        self.images = self.images.checked_add(1).ok_or("image count overflow")?;
        if let Some(maximum) = self.payload_limit {
            let payload =
                payload.ok_or("image payload is unavailable for input-limit validation")?;
            let size = u64::try_from(payload.len()).map_err(|_| "image payload size overflow")?;
            if size > maximum.get() {
                return Err("base64 image payload exceeds model input byte limit");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
