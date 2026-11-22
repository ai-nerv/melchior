use super::{endpoint::Provider, model::Model};
use reqwest::{
    Url,
    header::{HeaderMap, HeaderName, HeaderValue},
};
use std::collections::{BTreeMap, BTreeSet};

const HEADER_COUNT: usize = 128;
const HEADER_BYTES: usize = 32 * 1024;

pub(crate) fn base<'a>(provider: &'a Provider, model: &'a Model) -> Result<&'a str, &'static str> {
    let value = model
        .base_url
        .as_deref()
        .or(provider.base_url.as_deref())
        .ok_or("base_url")?;
    if !super::http::usable_url(value) {
        return Err(if model.base_url.is_some() {
            "model.base_url"
        } else {
            "base_url"
        });
    }
    Ok(value)
}

pub(crate) fn destination(
    provider: &Provider,
    model: &Model,
    raw: &str,
) -> Result<Url, &'static str> {
    validate(provider, model)?;
    provider_destination(provider, raw)
}

pub(crate) fn provider_destination(provider: &Provider, raw: &str) -> Result<Url, &'static str> {
    validate_provider(provider)?;
    if !super::http::usable_url(raw) {
        return Err("request endpoint");
    }
    let target = Url::parse(raw).map_err(|_| "request endpoint")?;
    let same = provider
        .base_url
        .as_deref()
        .filter(|url| super::http::usable_url(url))
        .and_then(|url| Url::parse(url).ok())
        .is_some_and(|url| url.origin() == target.origin());
    let bound = provider
        .credential_origins
        .iter()
        .filter_map(|url| Url::parse(url).ok())
        .any(|url| url.origin() == target.origin());
    if !same && !bound {
        return Err("credential_origins");
    }
    Ok(target)
}

pub(crate) fn validate_provider(provider: &Provider) -> Result<(), &'static str> {
    configured(&provider.headers).map_err(|_| "headers")?;
    if provider.credential_origins.iter().any(|raw| {
        !super::http::usable_url(raw)
            || !Url::parse(raw).is_ok_and(|url| {
                url.path() == "/"
                    && url.query().is_none()
                    && url.host_str().is_some_and(|host| !host.contains('*'))
            })
    }) {
        return Err("credential_origins");
    }
    Ok(())
}

pub(crate) fn validate(provider: &Provider, model: &Model) -> Result<(), &'static str> {
    validate_provider(provider)?;
    model.cost.validate().map_err(|_| "model.cost")?;
    configured(&model.headers).map_err(|_| "model.headers")?;
    Ok(())
}

pub(crate) fn headers(
    provider: &Provider,
    model: &Model,
    defaults: Vec<(String, String)>,
) -> Result<HeaderMap, &'static str> {
    merged([&provider.headers, &model.headers], defaults)
}

pub(crate) fn provider_headers(
    provider: &Provider,
    defaults: Vec<(String, String)>,
) -> Result<HeaderMap, &'static str> {
    merged([&provider.headers], defaults)
}

fn merged<const N: usize>(
    fields: [&BTreeMap<String, String>; N],
    defaults: Vec<(String, String)>,
) -> Result<HeaderMap, &'static str> {
    let mut headers = HeaderMap::new();
    let mut bytes = 0_usize;
    for (name, value) in defaults {
        bytes = bytes.saturating_add(name.len()).saturating_add(value.len());
        if bytes > HEADER_BYTES || headers.len() >= HEADER_COUNT {
            return Err("protocol headers exceed their limit");
        }
        let name =
            HeaderName::from_bytes(name.as_bytes()).map_err(|_| "invalid protocol header")?;
        let mut value = HeaderValue::from_str(&value).map_err(|_| "invalid protocol header")?;
        value.set_sensitive(true);
        if headers.insert(name, value).is_some() {
            return Err("duplicate protocol header");
        }
    }
    for fields in fields {
        for (name, mut value) in configured(fields)? {
            value.set_sensitive(true);
            headers.insert(name, value);
        }
    }
    check_headers(&headers)?;
    Ok(headers)
}

pub(crate) fn check_headers(headers: &HeaderMap) -> Result<(), &'static str> {
    let bytes: usize = headers
        .iter()
        .map(|(name, value)| name.as_str().len() + value.as_bytes().len())
        .sum();
    if headers.len() > HEADER_COUNT || bytes > HEADER_BYTES {
        return Err("request headers exceed their limit");
    }
    Ok(())
}

fn configured(
    fields: &BTreeMap<String, String>,
) -> Result<Vec<(HeaderName, HeaderValue)>, &'static str> {
    let mut seen = BTreeSet::new();
    let mut bytes = 0_usize;
    if fields.len() > HEADER_COUNT {
        return Err("too many configured headers");
    }
    fields
        .iter()
        .map(|(name, value)| {
            bytes = bytes.saturating_add(name.len()).saturating_add(value.len());
            if bytes > HEADER_BYTES {
                return Err("configured headers exceed their limit");
            }
            let name =
                HeaderName::from_bytes(name.as_bytes()).map_err(|_| "invalid configured header")?;
            if protected(name.as_str()) || !seen.insert(name.as_str().to_owned()) {
                return Err("protected or duplicate configured header");
            }
            let value = HeaderValue::from_str(value).map_err(|_| "invalid configured header")?;
            Ok((name, value))
        })
        .collect()
}

fn protected(name: &str) -> bool {
    name.starts_with("x-amz-")
        || matches!(
            name,
            "authorization"
                | "proxy-authorization"
                | "api-key"
                | "x-api-key"
                | "x-goog-api-key"
                | "cookie"
                | "set-cookie"
                | "host"
                | "content-length"
                | "transfer-encoding"
                | "connection"
                | "upgrade"
                | "te"
                | "trailer"
                | "content-type"
        )
}

#[cfg(test)]
mod tests;
