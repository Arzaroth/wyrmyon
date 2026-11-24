use std::sync::Arc;

use tokio::sync::watch;

#[derive(Clone)]
pub struct Cancel(Arc<watch::Sender<bool>>);

impl Cancel {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self(Arc::new(watch::channel(false).0))
    }

    pub fn cancel(&self) {
        self.0.send_replace(true);
    }

    pub fn is_cancelled(&self) -> bool {
        *self.0.borrow()
    }

    pub fn cancelled(&self) -> impl Future<Output = ()> + Send + 'static {
        let sender = Arc::clone(&self.0);
        async move {
            let mut cancelled = sender.subscribe();
            let _ = cancelled.wait_for(|c| *c).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_cancel_reaches_whoever_waits_on_it() {
        let cancel = Cancel::new();
        assert!(!cancel.is_cancelled());
        let waiting = tokio::spawn(cancel.clone().cancelled());
        cancel.cancel();
        waiting.await.unwrap();
        cancel.cancelled().await;
        assert!(cancel.is_cancelled());
    }
}
