use crate::events::{self, AttemptOutcome, Correlation, Fact, Id, RetryOutcome};
use crate::mind::accounting::Trace;
use std::time::Instant;

fn correlation(ask: Option<Id>) -> Option<Correlation> {
    Some(Correlation {
        ask_id: Some(ask?),
        attempt_id: Some(events::identity()?),
        ..Correlation::default()
    })
}

pub(super) struct Attempt {
    trace: Trace,
    correlation: Option<Correlation>,
    began: Option<Instant>,
    dispatched: bool,
    finished: bool,
}

impl Attempt {
    pub(super) fn new(trace: Trace) -> Self {
        Self::from(correlation(trace.ask()), trace)
    }

    fn from(correlation: Option<Correlation>, trace: Trace) -> Self {
        Self {
            trace,
            began: correlation.map(|_| Instant::now()),
            correlation,
            dispatched: false,
            finished: false,
        }
    }

    fn emit(&self, outcome: AttemptOutcome, duration: bool) {
        if let Some(correlation) = self.correlation {
            events::emit(|| {
                (
                    correlation,
                    Fact::ProviderAttempt {
                        outcome,
                        duration_ms: if duration {
                            self.began.and_then(events::elapsed)
                        } else {
                            None
                        },
                    },
                )
            });
        }
    }

    pub(super) fn dispatched(&mut self) {
        self.dispatched = true;
        self.emit(AttemptOutcome::Dispatched, false);
        self.trace.begin();
    }

    pub(super) fn finish(&mut self, succeeded: bool) {
        self.finished = true;
        if self.dispatched {
            self.trace.end();
        }
        self.emit(
            if succeeded {
                AttemptOutcome::Succeeded
            } else if self.dispatched {
                AttemptOutcome::Failed
            } else {
                AttemptOutcome::Refused
            },
            true,
        );
    }
}

impl Drop for Attempt {
    fn drop(&mut self) {
        if !self.finished {
            if self.dispatched {
                self.trace.end();
            }
            self.emit(
                if std::thread::panicking() {
                    AttemptOutcome::Failed
                } else {
                    AttemptOutcome::Cancelled
                },
                true,
            );
        }
    }
}

pub(super) struct Waiting {
    trace: Trace,
    correlation: Option<Correlation>,
    dispatched: bool,
}

impl Waiting {
    pub(super) fn new(trace: Trace) -> Self {
        let waiting = Self {
            correlation: correlation(trace.ask()),
            trace,
            dispatched: false,
        };
        waiting.emit(RetryOutcome::Scheduled);
        waiting
    }

    fn emit(&self, outcome: RetryOutcome) {
        if let Some(correlation) = self.correlation {
            events::emit(|| (correlation, Fact::Retry { outcome }));
        }
    }

    pub(super) fn dispatch(mut self) -> Attempt {
        self.dispatched = true;
        self.emit(RetryOutcome::Dispatched);
        Attempt::from(self.correlation, self.trace.clone())
    }
}

impl Drop for Waiting {
    fn drop(&mut self) {
        if !self.dispatched {
            self.emit(if std::thread::panicking() {
                RetryOutcome::Failed
            } else {
                RetryOutcome::Cancelled
            });
        }
    }
}
