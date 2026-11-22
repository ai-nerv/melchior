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
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "DeclaredCost")]
pub struct Cost {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
    pub presence: PricePresence,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tiers: Vec<Tier>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PricePresence {
    pub version: u8,
    #[serde(default)]
    pub input: bool,
    #[serde(default)]
    pub output: bool,
    #[serde(default)]
    pub cache_read: bool,
    #[serde(default)]
    pub cache_write: bool,
}

impl Default for PricePresence {
    fn default() -> Self {
        Self {
            version: 1,
            input: false,
            output: false,
            cache_read: false,
            cache_write: false,
        }
    }
}

#[derive(Deserialize)]
struct DeclaredCost {
    #[serde(default)]
    input: Option<f64>,
    #[serde(default)]
    output: Option<f64>,
    #[serde(default)]
    cache_read: Option<f64>,
    #[serde(default)]
    cache_write: Option<f64>,
    #[serde(default)]
    presence: Option<PricePresence>,
    #[serde(default)]
    tiers: Vec<Tier>,
}

impl TryFrom<DeclaredCost> for Cost {
    type Error = &'static str;

    fn try_from(raw: DeclaredCost) -> Result<Self, Self::Error> {
        let cost = Self {
            input: raw.input.unwrap_or_default(),
            output: raw.output.unwrap_or_default(),
            cache_read: raw.cache_read.unwrap_or_default(),
            cache_write: raw.cache_write.unwrap_or_default(),
            presence: raw.presence.unwrap_or(PricePresence {
                input: raw.input.is_some(),
                output: raw.output.is_some(),
                cache_read: raw.cache_read.is_some(),
                cache_write: raw.cache_write.is_some(),
                ..PricePresence::default()
            }),
            tiers: raw.tiers,
        };
        cost.validate()?;
        Ok(cost)
    }
}

impl Cost {
    pub(crate) fn pricing(&self) -> Option<Pricing> {
        let known = |rate: f64, present: bool| {
            (valid(rate) && (rate > 0.0 || (self.presence.version == 1 && present)))
                .then(|| rate.to_string())
        };
        let input = known(self.input, self.presence.input);
        let output = known(self.output, self.presence.output);
        (input.is_some() || output.is_some()).then_some(Pricing { input, output })
    }

    #[must_use]
    pub fn cache_read_rate(&self) -> f64 {
        self.base().cache_read_rate()
    }

    #[must_use]
    pub fn cache_write_rate(&self) -> f64 {
        self.base().cache_write_rate()
    }

    pub(crate) fn at(&self, prompt: u64) -> Rates {
        let mut rates = self.base();
        if let Some(tier) = self
            .tiers
            .iter()
            .filter(|tier| prompt > tier.input_tokens_above)
            .max_by_key(|tier| tier.input_tokens_above)
        {
            for (target, presence, value) in [
                (&mut rates.input, &mut rates.presence.input, tier.input),
                (&mut rates.output, &mut rates.presence.output, tier.output),
                (
                    &mut rates.cache_read,
                    &mut rates.presence.cache_read,
                    tier.cache_read,
                ),
                (
                    &mut rates.cache_write,
                    &mut rates.presence.cache_write,
                    tier.cache_write,
                ),
            ] {
                if let Some(value) = value {
                    *target = value;
                    *presence = true;
                }
            }
        }
        rates
    }

    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        if [self.input, self.output, self.cache_read, self.cache_write]
            .into_iter()
            .any(|rate| !valid(rate))
        {
            return Err("cost rates must be finite and nonnegative");
        }
        if self.tiers.len() > 64 {
            return Err("cost tiers exceed their count limit");
        }
        let mut thresholds = std::collections::BTreeSet::new();
        for tier in &self.tiers {
            if !thresholds.insert(tier.input_tokens_above) {
                return Err("cost tiers must have unique thresholds");
            }
            if [tier.input, tier.output, tier.cache_read, tier.cache_write]
                .into_iter()
                .flatten()
                .any(|rate| !valid(rate))
            {
                return Err("tier rates must be finite and nonnegative");
            }
        }
        Ok(())
    }

    fn base(&self) -> Rates {
        Rates {
            input: self.input,
            output: self.output,
            cache_read: self.cache_read,
            cache_write: self.cache_write,
            presence: self.presence,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tier {
    pub input_tokens_above: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write: Option<f64>,
}

pub(crate) struct Rates {
    pub input: f64,
    pub output: f64,
    cache_read: f64,
    cache_write: f64,
    presence: PricePresence,
}

impl Rates {
    pub(crate) fn cache_read_rate(&self) -> f64 {
        self.cached_rate(self.cache_read, self.presence.cache_read)
    }

    pub(crate) fn cache_write_rate(&self) -> f64 {
        self.cached_rate(self.cache_write, self.presence.cache_write)
    }

    fn cached_rate(&self, rate: f64, present: bool) -> f64 {
        if rate.is_finite()
            && rate >= 0.0
            && (rate > 0.0 || (self.presence.version == 1 && present))
        {
            rate
        } else {
            self.input
        }
    }

    pub(crate) fn known(&self, usage: super::Usage) -> bool {
        [
            (usage.input, self.input, self.presence.input),
            (usage.output, self.output, self.presence.output),
            (
                usage.cache_read,
                self.cache_read_rate(),
                self.presence.cache_read && self.cache_read == 0.0,
            ),
            (
                usage.cache_write,
                self.cache_write_rate(),
                self.presence.cache_write && self.cache_write == 0.0,
            ),
        ]
        .into_iter()
        .all(|(tokens, rate, present)| {
            tokens == 0 || (valid(rate) && (rate > 0.0 || (self.presence.version == 1 && present)))
        })
    }
}

fn valid(rate: f64) -> bool {
    rate.is_finite() && rate >= 0.0
}

#[cfg(test)]
mod tests;
