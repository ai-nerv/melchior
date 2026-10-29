use super::{Call, ProviderError, RetryClass, parted};
use crate::mind::model::{RequestFrame, overflow::Kind};
use crate::mind::provider::api::Options;
use serde::Deserialize;
use serde_json::Value;

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
}

impl Prepared {
    pub fn new(call: &Call<'_>, options: &Options) -> Self {
        let (mut built, frame) = call
            .adapter
            .request_observed(call.model, call.context, options);
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
        Self {
            body,
            scratch,
            frame,
            budget,
        }
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
            ..call.options.clone()
        };
        let next = Self::new(call, &options);
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
}
