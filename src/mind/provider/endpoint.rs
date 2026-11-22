//! A provider: an identity, a base URL, a credential, and a catalog.

use crate::mind::provider::model::{Api, Model};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Where a credential comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum Auth {
    #[serde(rename = "github-copilot")]
    GitHubCopilot {
        #[serde(flatten)]
        config: super::copilot::Config,
    },
    /// An API key read from the first environment variable that is set; several names, because
    /// vendors rename them and users still have the old ones exported.
    ApiKey {
        /// Variables to try, in order.
        vars: Vec<String>,
    },
    /// An OAuth flow against a subscription account.
    #[serde(rename = "oauth")]
    OAuth {
        /// What a person signs in to, for the message that tells them to.
        service: String,
        /// Where the browser is sent. Optional, with the rest of them, so a provider can be in the
        /// catalog before this build knows how to sign in to it, and `magi auth login` says so.
        #[serde(default)]
        authorize_url: Option<String>,
        /// Where a code, or a refresh token, is exchanged.
        #[serde(default)]
        token_url: Option<String>,
        /// The public client this build identifies as.
        #[serde(default)]
        client_id: Option<String>,
        /// What is asked for.
        #[serde(default)]
        scopes: Vec<String>,
    },
    /// AWS SigV4, from the usual credential chain.
    AwsSigV4,
    /// Google Application Default Credentials.
    GoogleAdc,
    /// No credential; a local endpoint.
    None,
}

impl Auth {
    pub(crate) fn refresh_config(&self) -> Option<(&str, &str)> {
        match self {
            Self::OAuth {
                token_url: Some(url),
                client_id: Some(client),
                ..
            } if super::http::usable_url(url) && !client.trim().is_empty() => Some((url, client)),
            _ => None,
        }
    }

    pub(crate) fn tokens_ready(&self, tokens: &super::oauth::Tokens, now: u64) -> bool {
        tokens.has_fresh_access(now)
            || ((self.refresh_config().is_some()
                || matches!(self, Self::GitHubCopilot { config } if config.valid()))
                && tokens
                    .refresh
                    .as_ref()
                    .is_some_and(|value| !value.trim().is_empty()))
    }

    /// What a person would have to do to enable this.
    #[must_use]
    pub fn requirement(&self) -> String {
        match self {
            Self::GitHubCopilot { .. } => "sign in to GitHub Copilot with :login".into(),
            Self::ApiKey { vars } => format!("set {}", vars.join(" or ")),
            Self::OAuth { service, .. } => format!("sign in to {service}"),
            // `resolve` returns `None` for both whatever the machine holds, so these say "not yet"
            // rather than name a credential that would change nothing.
            Self::AwsSigV4 => "not yet: signing requests with AWS credentials".to_owned(),
            Self::GoogleAdc => "not yet: reading Google application default credentials".to_owned(),
            Self::None => String::new(),
        }
    }

    /// An API key from any of these variables.
    #[must_use]
    pub fn env(vars: &[&str]) -> Self {
        Self::ApiKey {
            vars: vars.iter().map(|v| (*v).to_owned()).collect(),
        }
    }

    /// Read the credential, if one is set.
    #[must_use]
    pub fn resolve(&self) -> Option<String> {
        match self {
            Self::GitHubCopilot { .. } => std::env::var("COPILOT_GITHUB_TOKEN")
                .ok()
                .filter(|value| !value.trim().is_empty()),
            Self::ApiKey { vars } => vars
                .iter()
                .filter_map(|v| std::env::var(v).ok())
                .find(|value| !value.trim().is_empty()),
            // Whatever was stored, fresh or not; renewing needs a round trip this cannot make.
            Self::OAuth { .. } => None,
            // SigV4 signs each request and ADC mints a short-lived token; neither is implemented.
            Self::AwsSigV4 | Self::GoogleAdc | Self::None => None,
        }
    }

    /// The variables a person would have to set.
    #[must_use]
    pub fn vars(&self) -> &[String] {
        match self {
            Self::GitHubCopilot { .. } => {
                static VARS: std::sync::LazyLock<[String; 1]> =
                    std::sync::LazyLock::new(|| ["COPILOT_GITHUB_TOKEN".into()]);
                &*VARS
            }
            Self::ApiKey { vars } => vars,
            Self::OAuth { .. } | Self::AwsSigV4 | Self::GoogleAdc | Self::None => &[],
        }
    }
}

/// One vendor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(from = "ProviderDecl")]
pub struct Provider {
    /// Id used in configuration and in a qualified model name.
    pub id: String,
    /// Name shown to a person.
    pub name: String,
    /// Where its API lives, when that is fixed. `None` for endpoints derived from configuration
    /// such as a Bedrock region, which carry an explicit `compat` instead.
    pub base_url: Option<String>,
    /// Additional origins explicitly permitted to receive this provider's requests and credentials.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub credential_origins: Vec<String>,
    /// Request headers overriding non-authentication protocol headers.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    /// Which protocol it speaks.
    pub api: Api,
    /// How to authenticate.
    pub auth: Auth,
    /// Protocol overrides applied to every model this provider offers. Where there is no fixed
    /// `base_url`, this is the only way its dialect can be stated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compat: Option<crate::mind::provider::compat::Compat>,
    /// What it offers. Empty is allowed, and means "ask": see `discover`.
    #[serde(default)]
    pub models: Vec<Model>,
    /// Whether to ask the provider what it offers rather than be told here. What comes back is
    /// cached, never written into the configuration.
    #[serde(default)]
    pub discover: bool,
    /// Explicit discovery policy, overriding the legacy discover/details fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery: Option<crate::mind::discovering::Strategy>,
    /// Where a discovered model's own details are asked, when its listing leaves them out.
    /// `"ollama"` reads `/api/show` for each: its window, and whether it reasons or sees.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    /// Upstreams a router must never serve this provider's asks from, fallbacks included.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub avoid: Vec<String>,
}

impl Provider {
    pub(crate) fn saved_tokens<'a>(
        &self,
        store: &'a super::oauth::Store,
    ) -> Option<&'a super::oauth::Tokens> {
        if store
            .configurations
            .get(&self.id)
            .is_some_and(|scope| scope != &self.credential_scope())
        {
            return None;
        }
        store.get(&self.id)
    }

    pub(crate) fn credential_scope(&self) -> String {
        let encoded = serde_json::to_vec(&(
            &self.id,
            &self.auth,
            &self.base_url,
            &self.credential_origins,
        ))
        .unwrap_or_default();
        super::identity::digest("saved-provider-key-v1", &[&encoded])
    }

    pub(crate) fn saved_key<'a>(&self, store: &'a super::oauth::Store) -> Option<&'a str> {
        store
            .api_keys
            .get(&self.id)
            .filter(|key| {
                key.configuration == self.credential_scope() && !key.value.trim().is_empty()
            })
            .map(|key| key.value.as_str())
    }

    pub(crate) fn resolve_key(&self) -> Result<Option<String>, super::oauth::Error> {
        if let Some(key) = self.auth.resolve() {
            return Ok(Some(key));
        }
        if !matches!(self.auth, Auth::ApiKey { .. }) {
            return Ok(None);
        }
        let store = super::oauth::Store::load()?;
        Ok(self.saved_key(&store).map(str::to_owned))
    }

    /// Whether credentials are available without an interactive sign-in.
    #[must_use]
    pub fn is_configured(&self) -> bool {
        match &self.auth {
            Auth::None => true,
            Auth::GitHubCopilot { .. } => {
                self.auth.resolve().is_some()
                    || super::oauth::Store::load()
                        .is_ok_and(|store| super::copilot::signed_in(&store, self))
            }
            Auth::OAuth { .. } => crate::mind::provider::oauth::Store::load().is_ok_and(|store| {
                self.saved_tokens(&store)
                    .is_some_and(|tokens| self.auth.tokens_ready(tokens, super::oauth::now()))
            }),
            _ => self.resolve_key().is_ok_and(|key| key.is_some()),
        }
    }

    /// One of its models by id.
    #[must_use]
    pub fn model(&self, id: &str) -> Option<&Model> {
        self.models.iter().find(|m| m.id == id)
    }
}

/// A provider declaration with unresolved model defaults.
#[derive(Debug, Clone, PartialEq, Deserialize)]
struct ProviderDecl {
    id: String,
    name: String,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    credential_origins: Vec<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    api: Api,
    auth: Auth,
    #[serde(default)]
    compat: Option<crate::mind::provider::compat::Compat>,
    #[serde(default)]
    models: Vec<ModelDecl>,
    #[serde(default)]
    discover: bool,
    #[serde(default)]
    discovery: Option<crate::mind::discovering::Strategy>,
    #[serde(default)]
    details: Option<String>,
    #[serde(default)]
    avoid: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct ModelDecl {
    #[serde(default, deserialize_with = "explicit_api")]
    api: Option<Api>,
    #[serde(flatten)]
    model: Model,
}

fn explicit_api<'de, D: serde::Deserializer<'de>>(input: D) -> Result<Option<Api>, D::Error> {
    Api::deserialize(input).map(Some)
}

impl From<ProviderDecl> for Provider {
    fn from(decl: ProviderDecl) -> Self {
        let ProviderDecl {
            id,
            name,
            base_url,
            credential_origins,
            headers,
            api,
            auth,
            compat,
            models,
            discover,
            discovery,
            details,
            avoid,
        } = decl;
        let models = models
            .into_iter()
            .map(|decl| {
                let mut model = decl.model;
                model.provider.clone_from(&id);
                model.api = decl.api.unwrap_or(api);
                model.compat = Some(model.compat.unwrap_or_default().over(compat));
                model
            })
            .collect();
        Self {
            id,
            name,
            base_url,
            credential_origins,
            headers,
            api,
            auth,
            compat,
            models,
            discover,
            discovery,
            details,
            avoid,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A provider as a config would declare it.
    fn declared(json: serde_json::Value) -> Provider {
        serde_json::from_value(json).expect("a provider")
    }

    fn groq() -> serde_json::Value {
        serde_json::json!({
            "id": "groq",
            "name": "Groq",
            "api": "openai-completions",
            "base_url": "https://api.groq.com/openai/v1",
            "auth": { "kind": "api-key", "vars": ["GROQ_API_KEY"] },
            "models": [
                { "id": "a", "name": "A", "context_window": 131072, "max_tokens": 8192 },
                { "id": "b", "name": "B", "context_window": 131072, "max_tokens": 8192,
                  "reasoning": true },
            ]
        })
    }

    #[test]
    fn a_declaration_stamps_the_provider_and_api_onto_every_model() {
        let p = declared(groq());
        assert_eq!(p.models.len(), 2);
        assert!(p.models.iter().all(|m| m.provider == "groq"));
        assert!(p.models.iter().all(|m| m.api == Api::OpenAiCompletions));
    }

    #[test]
    fn omitted_model_api_inherits_and_explicit_api_survives_round_trip() {
        let mut value = groq();
        value["api"] = serde_json::json!("anthropic-messages");
        value["models"][1]["api"] = serde_json::json!("openai-responses");
        value["models"][1]["provider"] = serde_json::json!("incorrect");
        let provider = declared(value);
        assert_eq!(provider.models[0].api, Api::AnthropicMessages);
        assert_eq!(provider.models[1].api, Api::OpenAiResponses);
        assert!(provider.models.iter().all(|model| model.provider == "groq"));
        let encoded = serde_json::to_value(&provider).expect("serialize");
        assert_eq!(declared(encoded), provider);
    }

    #[test]
    fn explicit_default_protocol_is_not_mistaken_for_an_omitted_api() {
        let mut value = groq();
        value["api"] = serde_json::json!("anthropic-messages");
        value["models"][1]["api"] = serde_json::json!("openai-completions");
        let provider = declared(value);
        assert_eq!(provider.models[0].api, Api::AnthropicMessages);
        assert_eq!(provider.models[1].api, Api::OpenAiCompletions);
    }

    #[test]
    fn an_invalid_explicit_model_protocol_is_not_replaced_by_the_default() {
        for api in [
            serde_json::json!("unknown"),
            serde_json::Value::Null,
            serde_json::json!(12),
        ] {
            let mut value = groq();
            value["models"][1]["api"] = api;
            assert!(serde_json::from_value::<Provider>(value).is_err());
        }
    }

    #[test]
    fn omitted_model_fields_take_their_defaults() {
        let p = declared(groq());
        let a = p.model("a").expect("model a");
        assert!(!a.reasoning, "reasoning defaults off");
        assert_eq!(a.input, vec![crate::mind::provider::model::Modality::Text]);
        assert_eq!(a.cost.input, 0.0);
    }

    #[test]
    fn a_provider_with_no_auth_is_always_configured() {
        let p = declared(serde_json::json!({
            "id": "local", "name": "Local", "api": "openai-completions",
            "base_url": "http://localhost:11434/v1",
            "auth": { "kind": "none" },
            "models": [{ "id": "m", "name": "M", "context_window": 8192, "max_tokens": 4096 }]
        }));
        assert!(p.is_configured());
        assert!(p.auth.requirement().is_empty());
    }

    #[test]
    fn a_provider_without_its_variable_set_is_not_configured() {
        let p = declared(serde_json::json!({
            "id": "x", "name": "X", "api": "openai-completions",
            "base_url": "https://example.com/v1",
            "auth": { "kind": "api-key", "vars": ["MAGI_TEST_KEY_DEFINITELY_UNSET"] },
            "models": [{ "id": "m", "name": "M", "context_window": 8192, "max_tokens": 4096 }]
        }));
        assert!(!p.is_configured());
        assert_eq!(p.auth.requirement(), "set MAGI_TEST_KEY_DEFINITELY_UNSET");
    }

    #[test]
    fn a_credential_that_is_not_a_variable_says_what_to_do_instead() {
        let oauth = Auth::OAuth {
            service: "ChatGPT".into(),
            authorize_url: Some("https://example.test/authorize".into()),
            token_url: Some("https://example.test/token".into()),
            client_id: Some("c".into()),
            scopes: Vec::new(),
        };
        assert_eq!(oauth.requirement(), "sign in to ChatGPT");
        assert!(oauth.vars().is_empty(), "there is nothing to export");
        assert!(Auth::AwsSigV4.requirement().starts_with("not yet"));
        assert!(Auth::AwsSigV4.resolve().is_none());
        assert!(Auth::GoogleAdc.requirement().starts_with("not yet"));
    }

    #[test]
    fn a_provider_level_compat_reaches_every_model() {
        let p = declared(serde_json::json!({
            "id": "cf", "name": "Cloudflare", "api": "openai-completions",
            "auth": { "kind": "none" },
            "compat": { "supports_finish_reason": false },
            "models": [{ "id": "m", "name": "M", "context_window": 8192, "max_tokens": 4096 }]
        }));
        let compat = p.model("m").expect("model").compat.expect("compat");
        assert_eq!(compat.supports_finish_reason, Some(false));
    }

    #[test]
    fn a_model_may_override_its_providers_compat() {
        let p = declared(serde_json::json!({
            "id": "x", "name": "X", "api": "openai-completions",
            "auth": { "kind": "none" },
            "compat": { "supports_finish_reason": false },
            "models": [{ "id": "m", "name": "M", "context_window": 8192, "max_tokens": 4096,
                         "compat": { "supports_store": true } }]
        }));
        let compat = p.model("m").expect("model").compat.expect("compat");
        assert_eq!(compat.supports_store, Some(true), "the model's own wins");
    }

    #[test]
    fn a_provider_may_have_no_fixed_base_url() {
        let p = declared(serde_json::json!({
            "id": "bedrock", "name": "Bedrock", "api": "bedrock-converse-stream",
            "auth": { "kind": "aws-sig-v4" },
            "models": [{ "id": "m", "name": "M", "context_window": 8192, "max_tokens": 4096 }]
        }));
        assert!(
            p.base_url.is_none(),
            "the endpoint comes from configuration"
        );
    }

    #[test]
    fn an_unknown_model_is_absent_rather_than_a_panic() {
        assert!(declared(groq()).model("nope").is_none());
    }
}

#[cfg(test)]
mod merge_tests {
    use super::*;

    /// A provider stating its dialect, and one model correcting one flag.
    fn provider() -> Provider {
        serde_json::from_value(serde_json::json!({
            "id": "p", "name": "P", "api": "openai-completions", "auth": { "kind": "none" },
            "compat": {
                "supports_finish_reason": false,
                "max_tokens_field": "max_completion_tokens",
                "requires_tool_result_name": true
            },
            "models": [
                { "id": "plain", "name": "Plain", "context_window": 1000, "max_tokens": 100 },
                { "id": "odd", "name": "Odd", "context_window": 1000, "max_tokens": 100,
                  "compat": { "supports_store": true } }
            ]
        }))
        .expect("provider")
    }

    #[test]
    fn a_model_correcting_one_flag_keeps_the_rest_of_its_providers() {
        let compat = provider()
            .model("odd")
            .expect("model")
            .compat
            .expect("compat");
        assert_eq!(compat.supports_store, Some(true), "the model's own wins");
        assert_eq!(
            compat.supports_finish_reason,
            Some(false),
            "the provider's survives"
        );
        assert_eq!(
            compat.max_tokens_field,
            Some(crate::mind::provider::compat::MaxTokensField::MaxCompletionTokens),
            "and so does this one"
        );
        assert_eq!(compat.requires_tool_result_name, Some(true));
    }

    #[test]
    fn a_model_stating_nothing_gets_all_of_its_providers() {
        let compat = provider()
            .model("plain")
            .expect("model")
            .compat
            .expect("compat");
        assert_eq!(compat.supports_finish_reason, Some(false));
        assert_eq!(compat.requires_tool_result_name, Some(true));
    }

    #[test]
    fn a_model_may_still_contradict_its_provider() {
        let p: Provider = serde_json::from_value(serde_json::json!({
            "id": "p", "name": "P", "api": "openai-completions", "auth": { "kind": "none" },
            "compat": { "supports_finish_reason": false },
            "models": [{ "id": "m", "name": "M", "context_window": 1000, "max_tokens": 100,
                         "compat": { "supports_finish_reason": true } }]
        }))
        .expect("provider");
        let compat = p.model("m").expect("model").compat.expect("compat");
        assert_eq!(compat.supports_finish_reason, Some(true));
    }
}
