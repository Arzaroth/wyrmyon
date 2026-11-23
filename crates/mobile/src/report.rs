use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use wyrmyon_transfer::{Events, Pipe, Settled};

use crate::Cancel;
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

pub fn ended(report: &Report, result: anyhow::Result<()>) {
    if let Err(e) = result {
        tell(report, Stage::Failed, format!("{e:#}"));
    }
}

pub(crate) struct Reporter {
    report: Report,
    cancel: Cancel,
    total: AtomicU64,
    last: Mutex<Option<Instant>>,
}

impl Reporter {
    pub(crate) fn new(report: Report, cancel: Cancel) -> Self {
        Self {
            report,
            cancel,
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

    fn cancelled(&self) -> Settled<'_> {
        Box::pin(self.cancel.cancelled())
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
    fn only_a_failure_is_told_at_the_end_with_its_causes() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let kept = Arc::clone(&seen);
        let report: Report = Arc::new(move |update| kept.lock().unwrap().push(update));
        ended(&report, Ok(()));
        let failed = Err(anyhow::anyhow!("the disk is full").context("writing the data"));
        ended(&report, failed);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].stage, Stage::Failed);
        assert_eq!(seen[0].detail, "writing the data: the disk is full");
    }

    #[test]
    fn progress_is_told_at_most_every_tenth_of_a_second_and_always_at_the_end() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let kept = Arc::clone(&seen);
        let reporter = Reporter::new(
            Arc::new(move |update| kept.lock().unwrap().push(update)),
            Cancel::new(),
        );
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
