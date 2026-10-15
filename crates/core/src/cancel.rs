//! A way to ask a running download to stop.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::Notify;

/// Cloneable switch that stops a download. Give one clone to the download
/// (in `DownloadRequest::cancel`) and call [`CancelToken::cancel`] on another,
/// for example when the user presses Ctrl-C. The download then stops cleanly
/// and keeps what it has, so it can be resumed.
#[derive(Clone, Debug, Default)]
pub struct CancelToken {
    inner: Arc<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    stopped: AtomicBool,
    wake: Notify,
}

impl CancelToken {
    /// A token that has not been cancelled.
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask the download to stop. Calling it again does nothing.
    pub fn cancel(&self) {
        self.inner.stopped.store(true, Ordering::SeqCst);
        self.inner.wake.notify_waiters();
    }

    /// Whether [`CancelToken::cancel`] has been called.
    pub fn is_cancelled(&self) -> bool {
        self.inner.stopped.load(Ordering::SeqCst)
    }

    /// Completes once the token is cancelled.
    pub async fn cancelled(&self) {
        loop {
            // Register for the wake-up first, then look at the flag, so a
            // cancel that happens in between is not missed.
            let woken = self.inner.wake.notified();
            tokio::pin!(woken);
            woken.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            woken.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CancelToken;
    use std::time::Duration;

    #[tokio::test]
    async fn cancelled_completes_after_cancel() {
        let token = CancelToken::new();
        assert!(!token.is_cancelled());
        let waiter = tokio::spawn({
            let token = token.clone();
            async move { token.cancelled().await }
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!waiter.is_finished());
        token.cancel();
        tokio::time::timeout(Duration::from_secs(2), waiter)
            .await
            .expect("the waiter should finish")
            .unwrap();
        assert!(token.is_cancelled());
    }

    #[tokio::test]
    async fn already_cancelled_returns_at_once() {
        let token = CancelToken::new();
        token.cancel();
        tokio::time::timeout(Duration::from_millis(200), token.cancelled())
            .await
            .expect("should return immediately");
    }
}
