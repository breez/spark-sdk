//! Tracing-based operation detection for benchmark analysis.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::Subscriber;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

/// Payment-time swap (in select_leaves):
///   "Swapped leaves to match target amount"
const PAYMENT_SWAP_MESSAGE: &str = "Swapped leaves to match target amount";

/// Leaf optimization cancellation (in LeafOptimizer::cancel):
///   "Cancelling leaf optimization and waiting for completion"
pub const OPTIMIZATION_CANCELLATION_MESSAGE: &str =
    "Cancelling leaf optimization and waiting for completion";

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

/// A generic tracing layer that detects operations by message pattern.
///
/// This layer watches for trace messages containing a specific pattern
/// and sets a flag when detected.
pub struct OperationDetectorLayer {
    /// Flag that gets set to true when the target message is detected.
    operation_detected: Arc<AtomicBool>,
    /// The message pattern to match against.
    message_pattern: &'static str,
}

impl OperationDetectorLayer {
    pub fn new(operation_detected: Arc<AtomicBool>, message_pattern: &'static str) -> Self {
        Self {
            operation_detected,
            message_pattern,
        }
    }

    /// Create a swap detector layer for payment-time swaps.
    pub fn new_swap_detector(swap_detected: Arc<AtomicBool>) -> Self {
        Self::new(swap_detected, PAYMENT_SWAP_MESSAGE)
    }

    /// Create a cancellation detector layer for optimization cancellations.
    pub fn new_cancellation_detector(cancellation_detected: Arc<AtomicBool>) -> Self {
        Self::new(cancellation_detected, OPTIMIZATION_CANCELLATION_MESSAGE)
    }
}

impl<S> Layer<S> for OperationDetectorLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        // Check if this event contains the target message pattern
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        if let Some(message) = visitor.message
            && message.contains(self.message_pattern)
        {
            self.operation_detected.store(true, Ordering::SeqCst);
        }
    }
}

/// Guard for tracking operation detection during a single payment.
///
/// Create this before starting a payment, then check `had_operation()` after completion.
pub struct OperationDetectionGuard {
    operation_detected: Arc<AtomicBool>,
}

impl OperationDetectionGuard {
    /// Create a new guard and reset the detection flag.
    pub fn new(operation_detected: Arc<AtomicBool>) -> Self {
        operation_detected.store(false, Ordering::SeqCst);
        Self { operation_detected }
    }

    /// Check if the operation was detected during this guard's lifetime.
    pub fn had_operation(&self) -> bool {
        self.operation_detected.load(Ordering::SeqCst)
    }
}

/// Creates a shared operation detection flag.
pub fn create_operation_flag() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_operation_detection_guard() {
        let flag = create_operation_flag();
        let guard = OperationDetectionGuard::new(flag.clone());

        assert!(!guard.had_operation());

        flag.store(true, Ordering::SeqCst);
        assert!(guard.had_operation());

        let guard = OperationDetectionGuard::new(flag.clone());
        assert!(!guard.had_operation());
    }
}
