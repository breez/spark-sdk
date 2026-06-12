//! External (foreign) high-level Spark signer trait.
//!
//! Mirrors `spark_wallet::SparkSigner` using FFI-compatible types so an
//! integrator can implement the Spark flow-signing surface directly.
//! The SDK wraps an implementation in
//! [`ExternalSparkSignerAdapter`](super::ExternalSparkSignerAdapter) to obtain a
//! native `spark_wallet::SparkSigner`.

use crate::error::SignerError;

use super::external_spark_types::{
    ExternalFrostJob, ExternalFrostShareResult, ExternalPrepareClaimRequest,
    ExternalPrepareLightningReceiveRequest, ExternalPrepareStaticDepositClaimRequest,
    ExternalPrepareStaticDepositRequest, ExternalPrepareTokenTransactionRequest,
    ExternalPrepareTransferRequest, ExternalPreparedClaim, ExternalPreparedLightningReceive,
    ExternalPreparedStaticDeposit, ExternalPreparedStaticDepositClaim,
    ExternalPreparedTokenTransaction, ExternalPreparedTransfer, ExternalSignSparkInvoiceRequest,
    ExternalSignStaticDepositRefundRequest, ExternalSignedSparkInvoice,
    ExternalStartStaticDepositRefundRequest, ExternalStartedStaticDepositRefund,
};
use super::external_types::{
    EcdsaSignatureBytes, ExternalFrostSignature, ExternalTreeNodeId, PublicKeyBytes,
};

/// FFI-compatible mirror of `spark_wallet::SparkSigner`.
#[cfg_attr(
    feature = "uniffi",
    uniffi::export(with_foreign, async_runtime = "tokio")
)]
#[macros::async_trait]
pub trait ExternalSparkSigner: Send + Sync {
    /// The wallet identity public key (33 bytes compressed).
    async fn get_identity_public_key(&self) -> Result<PublicKeyBytes, SignerError>;

    /// The signing public key for a tree leaf.
    async fn get_public_key_for_leaf(
        &self,
        leaf_id: ExternalTreeNodeId,
    ) -> Result<PublicKeyBytes, SignerError>;

    /// The static-deposit signing public key at `index`.
    async fn get_static_deposit_public_key(
        &self,
        index: u32,
    ) -> Result<PublicKeyBytes, SignerError>;

    /// ECDSA-sign a server authentication challenge with the identity key.
    async fn sign_authentication_challenge(
        &self,
        challenge: Vec<u8>,
    ) -> Result<EcdsaSignatureBytes, SignerError>;

    /// ECDSA-sign an arbitrary user message with the identity key.
    async fn sign_message(&self, message: Vec<u8>) -> Result<EcdsaSignatureBytes, SignerError>;

    /// Produce FROST shares for a batch of jobs.
    async fn sign_frost(
        &self,
        jobs: Vec<ExternalFrostJob>,
    ) -> Result<Vec<ExternalFrostShareResult>, SignerError>;

    /// Prepare an outbound transfer (key-tweak + packages + payload signature).
    async fn prepare_transfer(
        &self,
        request: ExternalPrepareTransferRequest,
    ) -> Result<ExternalPreparedTransfer, SignerError>;

    /// Claim an inbound transfer (key-tweak step).
    async fn prepare_claim(
        &self,
        request: ExternalPrepareClaimRequest,
    ) -> Result<ExternalPreparedClaim, SignerError>;

    /// Prepare a Lightning receive (in-enclave preimage + Feldman split).
    async fn prepare_lightning_receive(
        &self,
        request: ExternalPrepareLightningReceiveRequest,
    ) -> Result<ExternalPreparedLightningReceive, SignerError>;

    /// Prepare a static deposit (export secret to SSP + FROST-sign tree txs).
    async fn prepare_static_deposit(
        &self,
        request: ExternalPrepareStaticDepositRequest,
    ) -> Result<ExternalPreparedStaticDeposit, SignerError>;

    /// Begin a static-deposit refund (user-commits-first).
    async fn start_static_deposit_refund(
        &self,
        request: ExternalStartStaticDepositRefundRequest,
    ) -> Result<ExternalStartedStaticDepositRefund, SignerError>;

    /// Finish a static-deposit refund (aggregate into the final signature).
    async fn sign_static_deposit_refund(
        &self,
        request: ExternalSignStaticDepositRefundRequest,
    ) -> Result<ExternalFrostSignature, SignerError>;

    /// Schnorr-sign a Spark invoice (sats or tokens) with the identity key.
    async fn sign_spark_invoice(
        &self,
        request: ExternalSignSparkInvoiceRequest,
    ) -> Result<ExternalSignedSparkInvoice, SignerError>;

    /// Schnorr-sign a token-transaction digest with the identity key.
    async fn prepare_token_transaction(
        &self,
        request: ExternalPrepareTokenTransactionRequest,
    ) -> Result<ExternalPreparedTokenTransaction, SignerError>;

    /// Prepare a static-deposit claim (export secret in the clear + sign).
    async fn prepare_static_deposit_claim(
        &self,
        request: ExternalPrepareStaticDepositClaimRequest,
    ) -> Result<ExternalPreparedStaticDepositClaim, SignerError>;
}
