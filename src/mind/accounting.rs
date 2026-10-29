use crate::events::{self, Correlation, Fact, Id, LedgerOutcome};
use crate::mind::{
    model::{Cost, MeasuredUsage, Usage},
    spending::{self, Settlement},
};
use std::{cell::RefCell, num::NonZeroU64, rc::Rc};

#[derive(Clone)]
pub(crate) struct Trace {
    ask: Option<Id>,
    usage: Option<Rc<RefCell<State>>>,
}

impl Trace {
    pub(crate) fn attempts() -> Self {
        Self {
            ask: events::identity(),
            usage: None,
        }
    }

    pub(crate) fn accounted(price: Cost) -> Self {
        let ask = events::identity();
        Self {
            ask,
            usage: ask.map(|_| Rc::new(RefCell::new(State::new(price)))),
        }
    }

    pub(crate) fn ask(&self) -> Option<Id> {
        self.ask
    }

    fn update(&self, change: impl FnOnce(&mut State), ledger: LedgerOutcome) {
        if let (Some(ask), Some(state)) = (self.ask, &self.usage) {
            let mut state = state.borrow_mut();
            change(&mut state);
            let Some(revision) = state.advance() else {
                events::exhausted();
                return;
            };
            let total = state.total();
            events::emit(|| {
                (
                    Correlation {
                        ask_id: Some(ask),
                        ..Correlation::default()
                    },
                    Fact::UsageSnapshot {
                        revision,
                        input_tokens: total.measured.input,
                        output_tokens: total.measured.output,
                        cache_read_tokens: total.measured.cache_read,
                        cache_write_tokens: total.measured.cache_write,
                        reported_cost_micros: total.measured.cost_micros,
                        estimated_cost_micros: total.estimate,
                        ledger,
                    },
                )
            });
        }
    }

    pub(crate) fn begin(&self) {
        self.update(
            |state| {
                state.active = true;
                state.current = Totals::default();
            },
            LedgerOutcome::Unknown,
        );
    }

    pub(crate) fn measured(&self, native: Usage, usage: MeasuredUsage) {
        if let Some(state) = &self.usage {
            let mut state = state.borrow_mut();
            state.current = Totals {
                measured: usage,
                estimate: estimate(native, usage, state.price),
            };
        }
    }

    pub(crate) fn settled(&self, result: Settlement) {
        self.update(
            |_| {},
            match result {
                Settlement::Replaced => LedgerOutcome::Persisted,
                Settlement::Unchanged => LedgerOutcome::Unchanged,
                Settlement::Untracked => LedgerOutcome::Untracked,
                Settlement::Failed => LedgerOutcome::Failed,
            },
        );
    }

    pub(crate) fn end(&self) {
        self.update(
            |state| {
                if state.active {
                    state.paid = state.total();
                    state.active = false;
                }
            },
            LedgerOutcome::Unknown,
        );
    }
}

#[derive(Clone, Copy, Default)]
struct Totals {
    measured: MeasuredUsage,
    estimate: Option<u64>,
}

fn plus(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    a?.checked_add(b?)
}

impl Totals {
    fn zero() -> Self {
        Self {
            measured: MeasuredUsage {
                input: Some(0),
                output: Some(0),
                cache_read: Some(0),
                cache_write: Some(0),
                cost_micros: Some(0),
            },
            estimate: Some(0),
        }
    }

    fn add(self, other: Self) -> Self {
        Self {
            measured: MeasuredUsage {
                input: plus(self.measured.input, other.measured.input),
                output: plus(self.measured.output, other.measured.output),
                cache_read: plus(self.measured.cache_read, other.measured.cache_read),
                cache_write: plus(self.measured.cache_write, other.measured.cache_write),
                cost_micros: plus(self.measured.cost_micros, other.measured.cost_micros),
            },
            estimate: plus(self.estimate, other.estimate),
        }
    }
}

struct State {
    paid: Totals,
    current: Totals,
    active: bool,
    revision: u64,
    price: Cost,
}

impl State {
    fn advance(&mut self) -> Option<NonZeroU64> {
        let revision = NonZeroU64::new(self.revision.checked_add(1)?)?;
        self.revision = revision.get();
        Some(revision)
    }

    fn new(price: Cost) -> Self {
        Self {
            paid: Totals::zero(),
            current: Totals::default(),
            active: false,
            revision: 0,
            price,
        }
    }

    fn total(&self) -> Totals {
        if self.active {
            self.paid.add(self.current)
        } else {
            self.paid
        }
    }
}

fn estimate(native: Usage, measured: MeasuredUsage, price: Cost) -> Option<u64> {
    if native.cost_micros > 0 {
        return Some(0);
    }
    let usage = Usage {
        input: measured.input?,
        output: measured.output?,
        cache_read: measured.cache_read?,
        cache_write: measured.cache_write?,
        ..Usage::default()
    };
    for (tokens, rate, explicit_zero) in [
        (usage.input, price.input, false),
        (usage.output, price.output, false),
        (
            usage.cache_read,
            price.cache_read_rate(),
            price.presence.version == 1 && price.presence.cache_read && price.cache_read == 0.0,
        ),
        (
            usage.cache_write,
            price.cache_write_rate(),
            price.presence.version == 1 && price.presence.cache_write && price.cache_write == 0.0,
        ),
    ] {
        if tokens > 0 && (!rate.is_finite() || rate < 0.0 || (rate == 0.0 && !explicit_zero)) {
            return None;
        }
    }
    let cost = spending::cost(usage, price);
    (cost != u64::MAX).then_some(cost)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revisions_exhaust_without_wrapping() {
        let mut state = State::new(Cost::default());
        assert_eq!(state.advance().map(NonZeroU64::get), Some(1));
        state.revision = u64::MAX - 1;
        assert_eq!(state.advance().map(NonZeroU64::get), Some(u64::MAX));
        assert_eq!(state.advance(), None);
        assert_eq!(state.advance(), None);
        assert_eq!(state.revision, u64::MAX);
    }

    #[test]
    fn unknown_attempts_and_overflow_do_not_turn_into_zero_or_wrapped_totals() {
        let zero = Totals::zero();
        assert_eq!(
            zero.add(Totals::default()).measured,
            MeasuredUsage::default()
        );
        assert_eq!(plus(Some(u64::MAX), Some(1)), None);
        assert_eq!(plus(Some(0), Some(0)), Some(0));
        let mut state = State::new(Cost::default());
        state.active = true;
        state.current = Totals {
            measured: MeasuredUsage {
                input: Some(8),
                ..Default::default()
            },
            estimate: None,
        };
        state.paid = state.total();
        state.current = Totals::default();
        assert_eq!(state.total().measured.input, None);
        state.current.measured.input = Some(5);
        assert_eq!(state.total().measured.input, Some(13));
        assert_eq!(state.total().measured.output, None);
    }

    #[test]
    fn estimate_uses_native_cost_selection_and_requires_known_counts_and_rates() {
        let mut measured = Totals::zero().measured;
        measured.input = Some(1_000_000);
        let price = Cost {
            input: 3.0,
            ..Default::default()
        };
        assert_eq!(estimate(Usage::default(), measured, price), Some(3_000_000));
        assert_eq!(estimate(Usage::default(), measured, Cost::default()), None);
        measured.output = None;
        assert_eq!(estimate(Usage::default(), measured, price), None);
        assert_eq!(
            estimate(
                Usage {
                    cost_micros: 7,
                    ..Default::default()
                },
                measured,
                price
            ),
            Some(0)
        );
        let mut cached = Totals::zero().measured;
        cached.cache_read = Some(100);
        let free: Cost = serde_json::from_value(serde_json::json!({"cache_read":0})).expect("cost");
        assert_eq!(estimate(Usage::default(), cached, free), Some(0));
        assert_eq!(
            estimate(
                Usage::default(),
                cached,
                Cost {
                    cache_read: -1.0,
                    ..free
                }
            ),
            None
        );
    }
}
