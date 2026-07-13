use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use breez_sdk_spark::SignerError;
use breez_sdk_spark::signer::CpfpSigner;
use flutter_rust_bridge::DartFnFuture;
use futures::FutureExt;

/// Extract a human-readable message from a panic payload.
fn panic_message(e: Box<dyn std::any::Any + Send>) -> String {
    e.downcast_ref::<String>()
        .cloned()
        .or_else(|| e.downcast_ref::<&str>().map(|s| (*s).to_string()))
        .unwrap_or_else(|| "Dart signer callback panicked".to_string())
}

/// Wraps a Dart `sign_psbt` callback as a [`CpfpSigner`] so a Flutter app can
/// sign the exit's CPFP inputs with any scheme (custom scripts, multisig, a
/// hardware wallet) rather than the built-in single-key signer. A Dart-side
/// throw surfaces as a [`SignerError`].
pub(crate) struct CallbackCpfpSigner {
    pub(crate) sign_psbt:
        Arc<dyn Fn(Vec<u8>) -> DartFnFuture<anyhow::Result<Vec<u8>>> + Send + Sync>,
}

#[async_trait::async_trait]
impl CpfpSigner for CallbackCpfpSigner {
    async fn sign_psbt(&self, psbt_bytes: Vec<u8>) -> Result<Vec<u8>, SignerError> {
        AssertUnwindSafe((self.sign_psbt)(psbt_bytes))
            .catch_unwind()
            .await
            .map_err(|e| SignerError::Signing(panic_message(e)))?
            .map_err(|e| SignerError::Signing(format!("{e}")))
    }
}
