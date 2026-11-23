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

    pub fn cancelled(&self) -> impl Future<Output = ()> + Send + 'static {
        let sender = Arc::clone(&self.0);
        async move {
            let mut cancelled = sender.subscribe();
            let _ = cancelled.wait_for(|c| *c).await;
        }
    }
}
