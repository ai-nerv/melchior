use super::{Account, config, network, refused};
use crate::mind::provider::{
    Http,
    endpoint::{Auth, Provider},
    oauth,
};

pub(crate) fn apply_catalog(providers: &mut [Provider]) {
    let Ok(store) = oauth::Store::load() else {
        return;
    };
    for provider in providers {
        if !matches!(provider.auth, Auth::GitHubCopilot { .. }) || provider.auth.resolve().is_some()
        {
            continue;
        }
        if let Ok((_, account)) = held(&store, provider) {
            provider
                .models
                .retain(|model| account.models.contains(&model.id));
        }
    }
}

pub(super) fn held(
    store: &oauth::Store,
    provider: &Provider,
) -> Result<(oauth::Tokens, Account), oauth::Error> {
    if store.configurations.get(&provider.id) != Some(&provider.credential_scope()) {
        return Err(refused("Sign in to GitHub Copilot with :login"));
    }
    let tokens = provider
        .saved_tokens(store)
        .cloned()
        .ok_or_else(|| refused("Sign in to GitHub Copilot with :login"))?;
    let account = store
        .copilot
        .get(&provider.id)
        .cloned()
        .ok_or_else(|| refused("Copilot account metadata missing; sign in again"))?;
    super::destination(provider, &account.base_url)?;
    Ok((tokens, account))
}

pub(crate) enum ResolveError<E> {
    Auth(oauth::Error),
    Check(E),
}

impl<E> From<oauth::Error> for ResolveError<E> {
    fn from(error: oauth::Error) -> Self {
        Self::Auth(error)
    }
}

pub(crate) async fn resolve<E>(
    http: &Http,
    provider: &Provider,
    mut inspect: impl FnMut(&oauth::Tokens, Option<&Account>) -> Result<(), E>,
) -> Result<(), ResolveError<E>> {
    config(provider)?;
    if let Some(access) = provider.auth.resolve() {
        return inspect(
            &oauth::Tokens {
                access,
                refresh: None,
                expires_at: u64::MAX,
                binding: None,
            },
            None,
        )
        .map_err(ResolveError::Check);
    }
    let (tokens, account) = held(&oauth::Store::load()?, provider)?;
    inspect(&tokens, Some(&account)).map_err(ResolveError::Check)?;
    if tokens.has_fresh_access(oauth::now()) {
        return Ok(());
    }
    let id = provider.id.clone();
    let _held = tokio::task::spawn_blocking(move || oauth::hold(&id))
        .await
        .map_err(|_| refused("Copilot credential lock failed"))??;
    let (tokens, account) = held(&oauth::Store::load()?, provider)?;
    inspect(&tokens, Some(&account)).map_err(ResolveError::Check)?;
    if tokens.has_fresh_access(oauth::now()) {
        return Ok(());
    }
    let github = tokens
        .refresh
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| refused("Copilot login expired; sign in again"))?;
    let (mut renewed, account) = network::exchange(http, provider, github).await?;
    renewed.binding = Some(tokens.identity());
    inspect(&renewed, Some(&account)).map_err(ResolveError::Check)?;
    let mut saved = false;
    oauth::Store::amend(|store| {
        if provider
            .saved_tokens(store)
            .is_some_and(|current| current.identity() == tokens.identity())
        {
            store.put(&provider.id, renewed.clone());
            store.copilot.insert(provider.id.clone(), account.clone());
            saved = true;
        }
    })?;
    if !saved {
        return Err(refused("Copilot login changed during renewal").into());
    }
    Ok(())
}
