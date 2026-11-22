use crate::mind::lua::LuaError;
use crate::mind::provider::endpoint::Provider;
use crate::mind::provider::model::Api;
use serde_json::Value;

pub(super) fn assemble(name: &str, value: &Value) -> Result<Provider, LuaError> {
    let refused = |message: &str| LuaError::Shape {
        what: format!("provider {name:?}"),
        message: message.to_owned(),
    };
    let mut value = value.clone();
    let object = value
        .as_object_mut()
        .ok_or_else(|| refused("expected a declaration table"))?;
    object.insert("id".into(), Value::String(name.to_owned()));
    if !object.get("api").is_some_and(known_api) {
        return Err(refused("api must name a known protocol"));
    }
    if let Some(strategy) = object.get("discovery") {
        serde_json::from_value::<crate::mind::discovering::Strategy>(strategy.clone())
            .map_err(|_| refused("discovery must name a valid strategy and header scheme"))?;
    }
    super::imports::apply(object).map_err(refused)?;
    if let Some(Value::Array(models)) = object.get("models") {
        for (index, model) in models.iter().enumerate() {
            if let Some(limits) = model.get("input_limits").filter(|value| !value.is_null()) {
                serde_json::from_value::<crate::mind::model::InputLimits>(limits.clone()).map_err(
                    |_| {
                        refused(&format!(
                            "models[{}].input_limits must contain known fields and positive limits",
                            index + 1
                        ))
                    },
                )?;
            }
            if let Some(cost) = model.get("cost") {
                serde_json::from_value::<crate::mind::model::Cost>(cost.clone()).map_err(|_| {
                    refused(&format!(
                        "models[{}].cost must contain valid rates and bounded unique tiers",
                        index + 1
                    ))
                })?;
            }
            if model.get("api").is_some_and(|api| !known_api(api)) {
                return Err(refused(&format!(
                    "models[{}].api must name a known protocol",
                    index + 1
                )));
            }
        }
    }
    let provider: Provider = serde_json::from_value(value).map_err(|_| {
        refused("invalid declaration; check name, api, auth, models and compatibility field types")
    })?;
    crate::mind::provider::routing::validate_provider(&provider)
        .map_err(|field| refused(&format!("invalid {field}")))?;
    for (index, model) in provider.models.iter().enumerate() {
        if let Some(advertised) = model
            .capabilities
            .as_ref()
            .and_then(|caps| caps.input_limits.as_ref())
            && Some(advertised) != model.input_limits.as_ref()
        {
            return Err(refused(&format!(
                "models[{}]: capabilities.input_limits must match enforced input_limits",
                index + 1
            )));
        }
        crate::mind::provider::routing::validate(&provider, model)
            .map_err(|field| refused(&format!("models[{}]: invalid {field}", index + 1)))?;
    }
    Ok(provider)
}

fn known_api(value: &Value) -> bool {
    serde_json::from_value::<Api>(value.clone()).is_ok()
}
