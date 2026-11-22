use crate::mind::provider::{endpoint::Provider, model::Api};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Strategy {
    Bundled,
    Compatible {
        #[serde(default = "path")]
        path: String,
        #[serde(default)]
        auth: Header,
    },
    Openrouter,
    Ollama,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Header {
    #[default]
    Bearer,
    ApiKey,
    XApiKey,
    GoogleApiKey,
}

fn path() -> String {
    "models".into()
}

impl Strategy {
    pub(super) fn selected(provider: &Provider) -> Self {
        if let Some(strategy) = &provider.discovery {
            return strategy.clone();
        }
        if !provider.discover {
            return Self::Bundled;
        }
        match provider.details.as_deref() {
            Some("openrouter") => Self::Openrouter,
            Some("ollama") => Self::Ollama,
            _ => Self::Compatible {
                path: path(),
                auth: match provider.api {
                    Api::AnthropicMessages => Header::XApiKey,
                    Api::AzureOpenAiResponses => Header::ApiKey,
                    Api::GoogleGenerativeAi => Header::GoogleApiKey,
                    _ => Header::Bearer,
                },
            },
        }
    }
}

impl Header {
    pub(super) fn fields(self, key: Option<&str>) -> Vec<(String, String)> {
        let Some(key) = key else {
            return Vec::new();
        };
        let (name, value) = match self {
            Self::Bearer => ("authorization", format!("Bearer {key}")),
            Self::ApiKey => ("api-key", key.to_owned()),
            Self::XApiKey => ("x-api-key", key.to_owned()),
            Self::GoogleApiKey => ("x-goog-api-key", key.to_owned()),
        };
        vec![(name.into(), value)]
    }
}
