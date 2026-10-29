//! Configured rates and versioned cached-rate presence.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pricing {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

/// Dollars per million tokens, with separately tracked cached-rate presence.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(from = "DeclaredCost")]
pub struct Cost {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
    pub presence: PricePresence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PricePresence {
    pub version: u8,
    #[serde(default)]
    pub cache_read: bool,
    #[serde(default)]
    pub cache_write: bool,
}

impl Default for PricePresence {
    fn default() -> Self {
        Self {
            version: 1,
            cache_read: false,
            cache_write: false,
        }
    }
}

#[derive(Deserialize)]
struct DeclaredCost {
    #[serde(default)]
    input: f64,
    #[serde(default)]
    output: f64,
    #[serde(default)]
    cache_read: Option<f64>,
    #[serde(default)]
    cache_write: Option<f64>,
    #[serde(default)]
    presence: Option<PricePresence>,
}

impl From<DeclaredCost> for Cost {
    fn from(raw: DeclaredCost) -> Self {
        Self {
            input: raw.input,
            output: raw.output,
            cache_read: raw.cache_read.unwrap_or_default(),
            cache_write: raw.cache_write.unwrap_or_default(),
            presence: raw.presence.unwrap_or(PricePresence {
                cache_read: raw.cache_read.is_some(),
                cache_write: raw.cache_write.is_some(),
                ..PricePresence::default()
            }),
        }
    }
}

impl Cost {
    #[must_use]
    pub fn cache_read_rate(self) -> f64 {
        self.cached_rate(self.cache_read, self.presence.cache_read)
    }

    #[must_use]
    pub fn cache_write_rate(self) -> f64 {
        self.cached_rate(self.cache_write, self.presence.cache_write)
    }

    fn cached_rate(self, rate: f64, present: bool) -> f64 {
        if rate.is_finite()
            && rate >= 0.0
            && (rate > 0.0 || (self.presence.version == 1 && present))
        {
            rate
        } else {
            self.input
        }
    }
}
