use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use wyrmyon_transfer::{Events, Pipe};

use crate::api::types::{Stage, Update};

pub type Report = Arc<dyn Fn(Update) + Send + Sync>;

const EVERY: Duration = Duration::from_millis(100);

pub(crate) fn tell(report: &Report, stage: Stage, detail: String) {
    report(Update {
        stage,
        bytes: 0,
        total: 0,
        detail,
    });
}

pub(crate) struct Reporter {
    report: Report,
    total: AtomicU64,
    last: Mutex<Option<Instant>>,
}

impl Reporter {
    pub(crate) fn new(report: Report) -> Self {
        Self {
            report,
            total: AtomicU64::new(0),
            last: Mutex::new(None),
        }
    }

    fn start(&self, stage: Stage, total: u64, detail: String) {
        self.total.store(total, Ordering::Relaxed);
        *self.last.lock().unwrap_or_else(PoisonError::into_inner) = None;
        (self.report)(Update {
            stage,
            bytes: 0,
            total,
            detail,
        });
    }

    fn progress(&self, stage: Stage, bytes: u64) {
        let total = self.total.load(Ordering::Relaxed);
        let now = Instant::now();
        {
            let mut last = self.last.lock().unwrap_or_else(PoisonError::into_inner);
            if bytes < total && last.is_some_and(|last| now - last < EVERY) {
                return;
            }
            *last = Some(now);
        }
        (self.report)(Update {
            stage,
            bytes,
            total,
            detail: String::new(),
        });
    }
}

impl Events for Reporter {
    fn hashing(&self, total: u64) {
        self.start(Stage::Hashing, total, String::new());
    }

    fn hashed(&self, bytes: u64) {
        self.progress(Stage::Hashing, bytes);
    }

    fn connected(&self, pipe: &Pipe, size: u64) {
        self.start(Stage::Connected, size, pipe.describe());
    }

    fn moved(&self, bytes: u64) {
        self.progress(Stage::Moving, bytes);
    }

    fn awaiting_ack(&self) {
        let total = self.total.load(Ordering::Relaxed);
        (self.report)(Update {
            stage: Stage::AwaitingAck,
            bytes: total,
            total,
            detail: String::new(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_is_told_at_most_every_tenth_of_a_second_and_always_at_the_end() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let kept = Arc::clone(&seen);
        let reporter = Reporter::new(Arc::new(move |update| kept.lock().unwrap().push(update)));
        reporter.hashing(100);
        for bytes in 1..=100 {
            reporter.hashed(bytes);
        }
        reporter.awaiting_ack();
        let seen: Vec<_> = seen
            .lock()
            .unwrap()
            .iter()
            .map(|u| (u.stage, u.bytes, u.total))
            .collect();
        assert_eq!(
            seen,
            [
                (Stage::Hashing, 0, 100),
                (Stage::Hashing, 1, 100),
                (Stage::Hashing, 100, 100),
                (Stage::AwaitingAck, 100, 100),
            ]
        );
    }
}
