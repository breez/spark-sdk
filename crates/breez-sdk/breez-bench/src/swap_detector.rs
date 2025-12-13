//! Tracing-based swap detection to distinguish payment-time swaps from background optimization swaps.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::Subscriber;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

/// Trace messages that indicate different swap types.
///
/// Payment-time swap (in select_leaves):
///   "Swapped leaves to match target amount"
///
/// Background optimization swap (in optimize_leaves):
///   "Optimizing {} leaves"
const PAYMENT_SWAP_MESSAGE: &str = "Swapped leaves to match target amount";

/// A tracing layer that detects payment-time swaps.
///
/// This layer watches for the specific trace message emitted when
/// `select_leaves()` performs a swap to match target amounts.
/// It ignores background optimization swaps.
pub struct SwapDetectorLayer {
    /// Flag that gets set to true when a payment-time swap is detected.
    swap_detected: Arc<AtomicBool>,
}

impl SwapDetectorLayer {
    pub fn new(swap_detected: Arc<AtomicBool>) -> Self {
        Self { swap_detected }
    }
}

impl<S> Layer<S> for SwapDetectorLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        // Check if this event contains the payment swap message
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        if let Some(message) = visitor.message
            && message.contains(PAYMENT_SWAP_MESSAGE)
        {
            self.swap_detected.store(true, Ordering::SeqCst);
        }
    }
}

/// Visitor to extract the message field from a tracing event.
#[derive(Default)]
struct MessageVisitor {
    message: Option<String>,
}

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{:?}", value));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        }
    }
}

/// Guard for tracking swap detection during a single payment.
///
/// Create this before starting a payment, then check `had_swap()` after completion.
pub struct SwapDetectionGuard {
    swap_detected: Arc<AtomicBool>,
}

impl SwapDetectionGuard {
    /// Create a new guard and reset the detection flag.
    pub fn new(swap_detected: Arc<AtomicBool>) -> Self {
        swap_detected.store(false, Ordering::SeqCst);
        Self { swap_detected }
    }

    /// Check if a payment-time swap was detected during this guard's lifetime.
    pub fn had_swap(&self) -> bool {
        self.swap_detected.load(Ordering::SeqCst)
    }

    /// Reset the flag for reuse.
    #[allow(dead_code)]
    pub fn reset(&self) {
        self.swap_detected.store(false, Ordering::SeqCst);
    }
}

/// Creates a shared swap detection flag.
pub fn create_swap_flag() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_swap_detection_guard() {
        let flag = create_swap_flag();
        let guard = SwapDetectionGuard::new(flag.clone());

        assert!(!guard.had_swap());

        flag.store(true, Ordering::SeqCst);
        assert!(guard.had_swap());

        guard.reset();
        assert!(!guard.had_swap());
    }
}
