use super::Usage;
use serde::Serialize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct MeasuredUsage {
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub cache_read: Option<u64>,
    pub cache_write: Option<u64>,
    pub cost_micros: Option<u64>,
}

impl MeasuredUsage {
    pub(crate) fn observe(&mut self, evidence: Option<&serde_json::Value>, held: Usage) {
        for (key, slot, retained) in [
            ("input", &mut self.input, held.input),
            ("output", &mut self.output, held.output),
            ("cache_read", &mut self.cache_read, held.cache_read),
            ("cache_write", &mut self.cache_write, held.cache_write),
            ("cost_micros", &mut self.cost_micros, held.cost_micros),
        ] {
            let measured = evidence
                .and_then(|value| value.get(key))
                .and_then(serde_json::Value::as_number)
                .and_then(super::usage::count);
            *slot = measured
                .filter(|value| *value == retained)
                .or_else(|| slot.filter(|value| *value == retained));
        }
    }
}
