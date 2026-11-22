use super::{Call, ProviderError, RetryClass, parted};
use crate::mind::model::{RequestFrame, overflow::Kind};
use crate::mind::provider::api::Options;
use serde::Deserialize;
use serde_json::Value;

fn continuation(model: &crate::mind::provider::model::Model) -> bool {
    use crate::mind::provider::{compat, model::Api};
    matches!(
        model.api,
        Api::OpenAiResponses
            | Api::AzureOpenAiResponses
            | Api::OpenAiCodexResponses
            | Api::GoogleGenerativeAi
            | Api::GoogleVertex
    ) || (model.api == Api::AnthropicMessages && compat::resolve(model.compat).anthropic_replay)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Budget {
    version: u32,
    automatic: bool,
    effective: u64,
    minimum: u64,
    path: String,
}

pub(super) struct Prepared {
    pub body: Value,
    pub scratch: Option<Value>,
    pub frame: Option<RequestFrame>,
    budget: Option<Budget>,
    options: Options,
}

impl Prepared {
    pub fn bound(
        &self,
        call: &Call<'_>,
        binding: String,
        endpoint: &str,
    ) -> Result<Option<Self>, ProviderError> {
        let provider_session = super::session::bound(call, &self.options, &binding, endpoint);
        if !continuation(call.model) && provider_session.is_none() {
            return Ok(None);
        }
        let options = Options {
            continuation_binding: continuation(call.model).then_some(binding),
            provider_session,
            ..self.options.clone()
        };
        Self::new(call, &options).map(Some)
    }

    pub fn new(call: &Call<'_>, options: &Options) -> Result<Self, ProviderError> {
        super::session::validate(options)
            .map_err(|why| ProviderError::new(RetryClass::Invalid, why).finally())?;
        let mut model = call.model.clone();
        if continuation(&model) {
            model.base_url = Some(
                crate::mind::provider::routing::base(call.provider, &model)
                    .map_err(|why| ProviderError::new(RetryClass::Invalid, why).finally())?
                    .to_owned(),
            );
        }
        let (mut built, frame) = call
            .adapter
            .prepare(&model, call.context, options)
            .map_err(|why| ProviderError::new(RetryClass::Invalid, why).finally())?;
        let budget = built
            .as_object_mut()
            .and_then(|fields| fields.remove("__budget"))
            .and_then(|value| serde_json::from_value::<Budget>(value).ok())
            .filter(|budget| {
                budget.version == 1
                    && budget.path.starts_with('/')
                    && budget.path.len() <= 256
                    && budget.minimum > 0
                    && budget.effective >= budget.minimum
                    && budget.effective <= call.model.max_tokens
                    && built.pointer(&budget.path).and_then(Value::as_u64) == Some(budget.effective)
            });
        let (body, scratch) = parted(built);
        if let Some(limits) = &call.model.input_limits {
            super::input_limits::check(call.model.api, limits, &body)
                .map_err(|why| ProviderError::new(RetryClass::Invalid, why).finally())?;
        }
        Ok(Self {
            body,
            scratch,
            frame,
            budget,
            options: options.clone(),
        })
    }

    pub fn lowered(&self, call: &Call<'_>, error: &ProviderError) -> Option<Self> {
        if error.class != RetryClass::Overflow
            || (call.options.max_tokens.is_some() && !call.options.max_tokens_automatic)
        {
            return None;
        }
        let budget = self.budget.as_ref().filter(|budget| budget.automatic)?;
        let detail = error.overflow?;
        if detail.kind != Kind::OutputBudget || detail.requested_output_tokens? != budget.effective
        {
            return None;
        }
        let room = detail.limit_tokens?.checked_sub(detail.prompt_tokens?)?;
        if room < budget.minimum || room >= budget.effective {
            return None;
        }
        let options = Options {
            output_limit: Some(room),
            ..self.options.clone()
        };
        let next = Self::new(call, &options).ok()?;
        let reduced = next.budget.as_ref()?;
        if !reduced.automatic
            || reduced.path != budget.path
            || reduced.effective != room
            || reduced.minimum < budget.minimum
            || next.scratch != self.scratch
        {
            return None;
        }
        let mut unchanged = next.body.clone();
        *unchanged.pointer_mut(&budget.path)? = Value::from(budget.effective);
        (unchanged == self.body).then_some(next)
    }

    pub fn session(&self) -> Option<&str> {
        self.options.provider_session.as_deref()
    }

    pub fn without_preferred_strict(&self, call: &Call<'_>, error: &ProviderError) -> Option<Self> {
        if error.class != RetryClass::Invalid
            || !error.strict_tool_limit
            || self.options.disable_preferred_strict
        {
            return None;
        }
        let options = Options {
            disable_preferred_strict: true,
            ..self.options.clone()
        };
        let next = Self::new(call, &options).ok()?;
        (next.body != self.body).then_some(next)
    }
}
