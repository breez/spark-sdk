//! Test-only [`ExternalSparkSigner`] decorator that records the
//! `prepare_transfer` requests the SDK forwards to the signer, so a test can
//! assert exactly what the SDK asked the signer to sign. Every other method is
//! forwarded unchanged to the wrapped signer.

use std::sync::{Arc, Mutex};

use breez_sdk_spark::SignerError;
use breez_sdk_spark::signer::*;

/// Shared, ordered log of the `prepare_transfer` requests the SDK issued.
pub type RecordedPrepareTransfers = Arc<Mutex<Vec<ExternalPrepareTransferRequest>>>;

/// Wraps an [`ExternalSparkSigner`], forwarding every call to the inner signer
/// and recording each `prepare_transfer` request it sees.
pub struct RecordingSparkSigner {
    inner: Arc<dyn ExternalSparkSigner>,
    recorded: RecordedPrepareTransfers,
}

impl RecordingSparkSigner {
    /// Wraps `inner`, returning the decorating signer and a handle to the log of
    /// recorded `prepare_transfer` requests.
    pub fn new(inner: Arc<dyn ExternalSparkSigner>) -> (Arc<Self>, RecordedPrepareTransfers) {
        let recorded: RecordedPrepareTransfers = Arc::new(Mutex::new(Vec::new()));
        let signer = Arc::new(Self {
            inner,
            recorded: recorded.clone(),
        });
        (signer, recorded)
    }
}

#[macros::async_trait]
impl ExternalSparkSigner for RecordingSparkSigner {
    async fn get_identity_public_key(&self) -> Result<PublicKeyBytes, SignerError> {
        self.inner.get_identity_public_key().await
    }

    async fn get_public_key_for_leaf(
        &self,
        leaf_id: ExternalTreeNodeId,
    ) -> Result<PublicKeyBytes, SignerError> {
        self.inner.get_public_key_for_leaf(leaf_id).await
    }

    async fn get_static_deposit_public_key(
        &self,
        index: u32,
    ) -> Result<PublicKeyBytes, SignerError> {
        self.inner.get_static_deposit_public_key(index).await
    }

    async fn sign_authentication_challenge(
        &self,
        challenge: Vec<u8>,
    ) -> Result<EcdsaSignatureBytes, SignerError> {
        self.inner.sign_authentication_challenge(challenge).await
    }

    async fn sign_message(&self, message: Vec<u8>) -> Result<EcdsaSignatureBytes, SignerError> {
        self.inner.sign_message(message).await
    }

    async fn sign_frost(
        &self,
        jobs: Vec<ExternalFrostJob>,
    ) -> Result<Vec<ExternalFrostShareResult>, SignerError> {
        self.inner.sign_frost(jobs).await
    }

    async fn prepare_transfer(
        &self,
        request: ExternalPrepareTransferRequest,
    ) -> Result<ExternalPreparedTransfer, SignerError> {
        self.recorded
            .lock()
            .expect("recorded prepare_transfer lock poisoned")
            .push(request.clone());
        self.inner.prepare_transfer(request).await
    }

    async fn prepare_claim(
        &self,
        request: ExternalPrepareClaimRequest,
    ) -> Result<ExternalPreparedClaim, SignerError> {
        self.inner.prepare_claim(request).await
    }

    async fn prepare_lightning_receive(
        &self,
        request: ExternalPrepareLightningReceiveRequest,
    ) -> Result<ExternalPreparedLightningReceive, SignerError> {
        self.inner.prepare_lightning_receive(request).await
    }

    async fn prepare_static_deposit(
        &self,
        request: ExternalPrepareStaticDepositRequest,
    ) -> Result<ExternalPreparedStaticDeposit, SignerError> {
        self.inner.prepare_static_deposit(request).await
    }

    async fn start_static_deposit_refund(
        &self,
        request: ExternalStartStaticDepositRefundRequest,
    ) -> Result<ExternalStartedStaticDepositRefund, SignerError> {
        self.inner.start_static_deposit_refund(request).await
    }

    async fn sign_static_deposit_refund(
        &self,
        request: ExternalSignStaticDepositRefundRequest,
    ) -> Result<ExternalFrostSignature, SignerError> {
        self.inner.sign_static_deposit_refund(request).await
    }

    async fn sign_spark_invoice(
        &self,
        request: ExternalSignSparkInvoiceRequest,
    ) -> Result<ExternalSignedSparkInvoice, SignerError> {
        self.inner.sign_spark_invoice(request).await
    }

    async fn prepare_token_transaction(
        &self,
        request: ExternalPrepareTokenTransactionRequest,
    ) -> Result<ExternalPreparedTokenTransaction, SignerError> {
        self.inner.prepare_token_transaction(request).await
    }

    async fn prepare_static_deposit_claim(
        &self,
        request: ExternalPrepareStaticDepositClaimRequest,
    ) -> Result<ExternalPreparedStaticDepositClaim, SignerError> {
        self.inner.prepare_static_deposit_claim(request).await
    }
}
