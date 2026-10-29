//! Token counts and what they cost.

use super::Cost;
use serde::{Deserialize, Serialize};

/// Tokens consumed by one request, with cache reads and writes counted apart from ordinary input
/// because they are priced apart.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// Prompt tokens billed at the input rate.
    #[serde(deserialize_with = "whole")]
    pub input: u64,
    /// Completion tokens.
    #[serde(deserialize_with = "whole")]
    pub output: u64,
    /// Prompt tokens served from cache.
    #[serde(deserialize_with = "whole")]
    pub cache_read: u64,
    /// Prompt tokens written to cache.
    #[serde(deserialize_with = "whole")]
    pub cache_write: u64,
    /// What the provider said the request cost, in millionths of a US dollar; zero where it did
    /// not say. Read as a number of either kind, because it arrives from Lua.
    #[serde(default, deserialize_with = "whole")]
    pub cost_micros: u64,
    /// Configured estimates for attempts with no positive provider cost; absent on legacy totals.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_whole"
    )]
    pub estimated_micros: Option<u64>,
}

fn optional_whole<'de, D: serde::Deserializer<'de>>(said: D) -> Result<Option<u64>, D::Error> {
    let value = Option::<serde_json::Number>::deserialize(said)?;
    value
        .map(|value| {
            count(&value)
                .ok_or_else(|| serde::de::Error::custom("expected a nonnegative integral u64"))
        })
        .transpose()
}

/// A count that may arrive as `1234` or as `1234.0`: Lua does not always tell the two apart.
fn whole<'de, D: serde::Deserializer<'de>>(said: D) -> Result<u64, D::Error> {
    let value = serde_json::Number::deserialize(said)?;
    count(&value).ok_or_else(|| serde::de::Error::custom("expected a nonnegative integral u64"))
}

pub(super) fn count(value: &serde_json::Number) -> Option<u64> {
    value.as_u64().or_else(|| {
        let n = value.as_f64()?;
        (n.is_finite() && n >= 0.0 && n.fract() == 0.0 && n < 18_446_744_073_709_551_616.0)
            .then_some(n as u64)
    })
}

impl Usage {
    #[must_use]
    pub const fn charged_micros(self) -> u64 {
        self.cost_micros
            .saturating_add(match self.estimated_micros {
                Some(n) => n,
                None => 0,
            })
    }

    pub(crate) fn observe(&mut self, other: Self) {
        if other.prompt_tokens() >= self.prompt_tokens() {
            self.input = other.input;
            self.cache_read = other.cache_read;
            self.cache_write = other.cache_write;
        }
        self.output = self.output.max(other.output);
        self.cost_micros = self.cost_micros.max(other.cost_micros);
    }

    /// Every token that counted towards the context window.
    #[must_use]
    pub const fn prompt_tokens(self) -> u64 {
        self.input
            .saturating_add(self.cache_read)
            .saturating_add(self.cache_write)
    }

    pub(crate) fn measured(value: &serde_json::Value, previous: Self) -> (Self, u64) {
        let Some(object) = value.as_object() else {
            return (previous, 1);
        };
        let mut anomalies = 0;
        let mut measured = previous;
        for (name, slot) in [
            ("input", &mut measured.input),
            ("output", &mut measured.output),
            ("cache_read", &mut measured.cache_read),
            ("cache_write", &mut measured.cache_write),
            ("cost_micros", &mut measured.cost_micros),
        ] {
            if let Some(value) = object.get(name) {
                if let Some(n) = value.as_number().and_then(count) {
                    *slot = n;
                } else {
                    anomalies += 1;
                }
            }
        }
        let mut held = previous;
        held.observe(measured);
        (held, anomalies)
    }

    /// What fraction of the prompt came from cache, or `None` when nothing was sent.
    #[must_use]
    pub fn cache_hit_rate(self) -> Option<f64> {
        let prompt = self.prompt_tokens();
        (prompt > 0).then(|| self.cache_read as f64 / prompt as f64 * 100.0)
    }

    /// Add another request's tokens to this total, stopping at the largest count rather than
    /// wrapping.
    pub const fn add(&mut self, other: Self) {
        self.input = self.input.saturating_add(other.input);
        self.output = self.output.saturating_add(other.output);
        self.cache_read = self.cache_read.saturating_add(other.cache_read);
        self.cache_write = self.cache_write.saturating_add(other.cache_write);
        self.cost_micros = self.cost_micros.saturating_add(other.cost_micros);
        self.estimated_micros = match (self.estimated_micros, other.estimated_micros) {
            (None, None) => None,
            (a, b) => Some(
                match a {
                    Some(n) => n,
                    None => 0,
                }
                .saturating_add(match b {
                    Some(n) => n,
                    None => 0,
                }),
            ),
        };
    }

    /// What these tokens cost at `cost`.
    #[must_use]
    pub fn price(self, cost: Cost) -> f64 {
        let per_million = |tokens: u64, rate: f64| tokens as f64 / 1_000_000.0 * rate;
        per_million(self.input, cost.input)
            + per_million(self.output, cost.output)
            + per_million(self.cache_read, cost.cache_read_rate())
            + per_million(self.cache_write, cost.cache_write_rate())
    }
}

#[cfg(test)]
mod accounting_tests;
#[cfg(test)]
mod pricing_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_tokens_count_cache_as_well_as_input() {
        let usage = Usage {
            input: 100,
            output: 50,
            cache_read: 900,
            cache_write: 10,
            cost_micros: 0,
            estimated_micros: None,
        };
        assert_eq!(usage.prompt_tokens(), 1010);
    }

    #[test]
    fn a_cost_arrives_as_either_kind_of_number() {
        let whole: Usage = serde_json::from_str(
            r#"{"input":1,"output":1,"cache_read":0,"cache_write":0,"cost_micros":1234}"#,
        )
        .expect("an integer");
        let float: Usage = serde_json::from_str(
            r#"{"input":1,"output":1,"cache_read":0,"cache_write":0,"cost_micros":1234.0}"#,
        )
        .expect("a float");
        assert_eq!(whole.cost_micros, 1234);
        assert_eq!(float.cost_micros, 1234);
        let unsaid: Usage =
            serde_json::from_str(r#"{"input":1,"output":1,"cache_read":0,"cache_write":0}"#)
                .expect("none at all");
        assert_eq!(unsaid.cost_micros, 0);
    }

    #[test]
    fn the_cache_hit_rate_is_a_percentage_of_the_prompt() {
        let usage = Usage {
            input: 250,
            cache_read: 750,
            ..Usage::default()
        };
        assert_eq!(usage.cache_hit_rate(), Some(75.0));
    }

    #[test]
    fn an_empty_prompt_has_no_hit_rate_rather_than_zero() {
        assert_eq!(Usage::default().cache_hit_rate(), None);
    }

    #[test]
    fn totals_accumulate() {
        let mut total = Usage::default();
        total.add(Usage {
            input: 10,
            output: 5,
            ..Usage::default()
        });
        total.add(Usage {
            input: 1,
            output: 2,
            ..Usage::default()
        });
        assert_eq!(total.input, 11);
        assert_eq!(total.output, 7);
    }

    #[test]
    fn price_is_per_million_tokens() {
        let usage = Usage {
            input: 1_000_000,
            output: 500_000,
            ..Usage::default()
        };
        let cost = Cost {
            input: 3.0,
            output: 15.0,
            ..Cost::default()
        };
        assert!(
            (usage.price(cost) - 10.5).abs() < 1e-9,
            "{}",
            usage.price(cost)
        );
    }
}
