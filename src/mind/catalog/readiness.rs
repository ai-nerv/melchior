use crate::mind::provider::{
    endpoint::{Auth, Provider},
    oauth::{self, Store},
};
use crate::mind::wire::{Setup, SetupState};

pub(super) fn store(providers: &[Provider]) -> Result<Store, oauth::Error> {
    if providers.iter().any(|provider| {
        matches!(
            provider.auth,
            Auth::OAuth { .. } | Auth::ApiKey { .. } | Auth::GitHubCopilot { .. }
        )
    }) {
        Store::load()
    } else {
        Ok(Store::default())
    }
}

pub(super) fn provider(
    provider: &Provider,
    store: &Result<Store, oauth::Error>,
    now: u64,
) -> Setup {
    let mut setup = Setup {
        state: SetupState::Ready,
        methods: Vec::new(),
        variables: Vec::new(),
        fields: Vec::new(),
        login: false,
    };
    match &provider.auth {
        Auth::GitHubCopilot { config } => {
            setup.methods = vec!["oauth-device".into(), "api-key".into()];
            setup.variables.push(provider.auth.vars().to_vec());
            setup.login = config.valid();
            setup.state = if !config.valid() {
                SetupState::ConfigurationIncomplete
            } else if provider.auth.resolve().is_some() {
                SetupState::Ready
            } else {
                match store {
                    Err(_) => SetupState::CredentialUnreadable,
                    Ok(store) if crate::mind::provider::copilot::signed_in(store, provider) => {
                        SetupState::Ready
                    }
                    Ok(_) => SetupState::CredentialMissing,
                }
            };
        }
        Auth::None => setup.methods.push("none".into()),
        Auth::ApiKey { vars } => {
            setup.methods.push("api-key".into());
            if vars.is_empty() {
                setup.fields.push("auth.vars".into());
                setup.state = SetupState::ConfigurationIncomplete;
            } else {
                setup.variables.push(vars.clone());
                if provider.auth.resolve().is_none() {
                    setup.state = match store {
                        Ok(store) if provider.saved_key(store).is_some() => SetupState::Ready,
                        Ok(_) => SetupState::CredentialMissing,
                        Err(_) => SetupState::CredentialUnreadable,
                    };
                }
            }
        }
        Auth::OAuth {
            authorize_url,
            token_url,
            client_id,
            ..
        } => {
            setup.methods.push("oauth".into());
            setup.login = provider.auth.refresh_config().is_some()
                && authorize_url
                    .as_deref()
                    .is_some_and(crate::mind::provider::usable_url);
            setup.state = match store {
                Err(_) => SetupState::CredentialUnreadable,
                Ok(store) => match provider.saved_tokens(store) {
                    None => SetupState::CredentialMissing,
                    Some(tokens) if provider.auth.tokens_ready(tokens, now) => SetupState::Ready,
                    Some(tokens)
                        if tokens
                            .refresh
                            .as_ref()
                            .is_some_and(|value| !value.trim().is_empty()) =>
                    {
                        if !token_url
                            .as_deref()
                            .is_some_and(crate::mind::provider::usable_url)
                        {
                            setup.fields.push("auth.token_url".into());
                        }
                        if client_id
                            .as_ref()
                            .is_none_or(|value| value.trim().is_empty())
                        {
                            setup.fields.push("auth.client_id".into());
                        }
                        SetupState::ConfigurationIncomplete
                    }
                    Some(_) => SetupState::CredentialExpired,
                },
            };
        }
        Auth::AwsSigV4 => {
            setup.methods.push("aws-sig-v4".into());
            setup.state = SetupState::AuthUnavailable;
        }
        Auth::GoogleAdc => {
            setup.methods.push("google-adc".into());
            setup.state = SetupState::AuthUnavailable;
        }
    }
    setup
}

pub(super) fn model(
    provider: &Provider,
    model: &crate::mind::provider::model::Model,
    mut setup: Setup,
) -> Setup {
    let route = crate::mind::provider::routing::base(provider, model)
        .and_then(|base| crate::mind::provider::routing::destination(provider, model, base));
    if let Err(field) = route {
        setup.fields.push(field.into());
        setup.state = SetupState::ConfigurationIncomplete;
    }
    setup
}
