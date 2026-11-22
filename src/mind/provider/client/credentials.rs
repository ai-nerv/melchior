use super::{ProviderError, RetryClass};
use crate::mind::provider::{
    Http,
    endpoint::{Auth, Provider},
    identity, oauth,
};
use reqwest::header::HeaderMap;

pub(super) struct Resolved {
    pub key: Option<String>,
    pub endpoint: Option<String>,
    pub models: Option<std::collections::BTreeSet<String>>,
    principal: String,
    rotating: bool,
}

impl Resolved {
    fn oauth(tokens: &oauth::Tokens) -> Self {
        Self {
            key: Some(tokens.access.clone()),
            endpoint: None,
            models: None,
            principal: tokens.identity(),
            rotating: true,
        }
    }

    pub fn binding(
        &self,
        provider: &Provider,
        headers: &HeaderMap,
    ) -> Result<String, ProviderError> {
        let auth = serde_json::to_vec(&provider.auth).map_err(|_| {
            ProviderError::new(RetryClass::Invalid, "invalid authentication configuration")
                .finally()
        })?;
        let mut fields = vec![auth, self.principal.as_bytes().to_vec()];
        let mut headers: Vec<_> = headers.iter().collect();
        headers.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
        for (name, value) in headers {
            fields.push(name.as_str().as_bytes().to_vec());
            let rotating = self.rotating
                && self.key.as_ref().is_some_and(|key| {
                    if name.as_str() == "authorization" {
                        value.as_bytes() == format!("Bearer {key}").as_bytes()
                    } else {
                        matches!(name.as_str(), "api-key" | "x-api-key" | "x-goog-api-key")
                            && value.as_bytes() == key.as_bytes()
                    }
                });
            fields.push(if rotating {
                b"oauth-access-v1".to_vec()
            } else {
                value.as_bytes().to_vec()
            });
        }
        let fields: Vec<_> = fields.iter().map(Vec::as_slice).collect();
        Ok(identity::digest("responses-credential-v1", &fields))
    }
}

fn auth(error: oauth::Error) -> ProviderError {
    ProviderError::new(RetryClass::Auth, error.to_string())
}

fn held(store: &oauth::Store, provider: &Provider) -> Result<oauth::Tokens, ProviderError> {
    provider
        .saved_tokens(store)
        .cloned()
        .ok_or_else(|| auth(oauth::Error::NotSignedIn(provider.id.clone())))
}

pub(super) async fn with(
    http: &Http,
    provider: &Provider,
    mut validate: impl FnMut(&Resolved) -> Result<(), ProviderError>,
) -> Result<(), ProviderError> {
    if matches!(provider.auth, Auth::GitHubCopilot { .. }) {
        return crate::mind::provider::copilot::resolve(http, provider, |tokens, account| {
            let mut resolved = Resolved::oauth(tokens);
            resolved.endpoint = Some(
                account
                    .as_ref()
                    .map_or_else(
                        || crate::mind::provider::copilot::endpoint(provider, &tokens.access),
                        |a| Ok(a.base_url.clone()),
                    )
                    .map_err(auth)?,
            );
            resolved.models = account.map(|a| a.models.clone());
            validate(&resolved)
        })
        .await
        .map_err(|error| match error {
            crate::mind::provider::copilot::ResolveError::Auth(error) => auth(error),
            crate::mind::provider::copilot::ResolveError::Check(error) => error,
        });
    }
    if !matches!(provider.auth, Auth::OAuth { .. }) {
        let owner = provider.clone();
        let key = off_thread(move || owner.resolve_key())
            .await
            .map_err(auth)?;
        if key.is_none() && !matches!(provider.auth, Auth::None) {
            return Err(ProviderError::new(
                RetryClass::Auth,
                format!("{}: {}", provider.id, provider.auth.requirement()),
            ));
        }
        let principal = key.as_ref().map_or_else(String::new, |key| {
            identity::digest("api-key-v1", &[key.as_bytes()])
        });
        return validate(&Resolved {
            key,
            endpoint: None,
            models: None,
            principal,
            rotating: false,
        });
    }
    let store = off_thread(oauth::Store::load).await.map_err(auth)?;
    let tokens = held(&store, provider)?;
    validate(&Resolved::oauth(&tokens))?;
    if tokens.has_fresh_access(oauth::now()) {
        return Ok(());
    }
    let (token_url, client_id) = provider.auth.refresh_config().ok_or_else(|| {
        auth(oauth::Error::Refused(format!(
            "OAuth refresh is not configured for {}",
            provider.id
        )))
    })?;
    let id = provider.id.clone();
    let _held = off_thread(move || oauth::hold(&id)).await.map_err(auth)?;
    let store = off_thread(oauth::Store::load).await.map_err(auth)?;
    let tokens = held(&store, provider)?;
    validate(&Resolved::oauth(&tokens))?;
    if tokens.has_fresh_access(oauth::now()) {
        return Ok(());
    }
    let refresh = tokens
        .refresh
        .clone()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| auth(oauth::Error::Expired(provider.id.clone())))?;
    let renewed = oauth::exchange(
        http,
        token_url,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh),
            ("client_id", client_id),
        ],
    )
    .await
    .map_err(auth)?;
    let id = provider.id.clone();
    let store = off_thread(move || oauth::Store::amend(|store| store.renew(&id, renewed)))
        .await
        .map_err(auth)?;
    validate(&Resolved::oauth(&held(&store, provider)?))
}

async fn off_thread<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, oauth::Error> + Send + 'static,
) -> Result<T, oauth::Error> {
    tokio::task::spawn_blocking(work).await.unwrap_or_else(|_| {
        Err(oauth::Error::Refused(
            "credentials could not be reached".into(),
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn binding_tracks_effective_headers_and_issuer_but_not_oauth_rotation() {
        let mut provider: Provider = serde_json::from_value(json!({"id":"p","name":"P",
            "api":"openai-responses","base_url":"https://fixture.invalid",
            "auth":{"kind":"oauth","service":"Fixture","token_url":"https://fixture.invalid/token","client_id":"client"}})).expect("provider");
        let mut token = oauth::Tokens {
            access: "first-secret".into(),
            refresh: None,
            expires_at: u64::MAX,
            binding: Some("a".repeat(64)),
        };
        let headers = |access: &str, organization: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(
                "authorization",
                format!("Bearer {access}").parse().expect("header"),
            );
            headers.insert("openai-organization", organization.parse().expect("header"));
            headers
        };
        let binding = Resolved::oauth(&token)
            .binding(&provider, &headers(&token.access, "org-a"))
            .expect("binding");
        token.access = "rotated-secret".into();
        assert_eq!(
            Resolved::oauth(&token)
                .binding(&provider, &headers(&token.access, "org-a"))
                .expect("same login"),
            binding
        );
        assert_ne!(
            Resolved::oauth(&token)
                .binding(&provider, &headers(&token.access, "org-b"))
                .expect("other org"),
            binding
        );
        let Auth::OAuth { client_id, .. } = &mut provider.auth else {
            panic!("oauth");
        };
        *client_id = Some("another-client".into());
        assert_ne!(
            Resolved::oauth(&token)
                .binding(&provider, &headers(&token.access, "org-a"))
                .expect("other client"),
            binding
        );
        assert_eq!(binding.len(), 64);
        assert!(!binding.contains("secret"));
    }
}
