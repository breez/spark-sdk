use std::sync::Arc;

use tokio::sync::Notify;

#[derive(Clone, Default)]
pub struct Wakeup(Arc<Notify>);

impl Wakeup {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Wakes one waiter. With none waiting, it leaves a wake for the next `waited`
    /// to take, and further wakes before then add nothing.
    pub fn wake(&self) {
        self.0.notify_one();
    }

    pub async fn waited(&self) {
        self.0.notified().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_nudge_raised_before_the_wait_is_kept() {
        let wakeup = Wakeup::new();
        wakeup.wake();
        wakeup.waited().await;
    }

    #[tokio::test]
    async fn nudges_coalesce() {
        let wakeup = Wakeup::new();
        wakeup.wake();
        wakeup.wake();
        wakeup.waited().await;
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), wakeup.waited())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn a_clone_wakes_the_same_worker() {
        let wakeup = Wakeup::new();
        let waiter = wakeup.clone();
        let handle = tokio::spawn(async move { waiter.waited().await });
        for _ in 0..100 {
            wakeup.wake();
            if handle.is_finished() {
                break;
            }
            tokio::task::yield_now().await;
        }
        handle.await.expect("the clone's nudge woke the waiter");
    }
}
