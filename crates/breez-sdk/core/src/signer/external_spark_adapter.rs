//! Adapter from a foreign [`ExternalSparkSigner`] to a native
//! `spark_wallet::SparkSigner`.
//!
//! Converts each native request into its FFI mirror, calls the foreign signer,
//! and converts the FFI response back into the native type.

use std::sync::Arc;

use bitcoin::secp256k1::{PublicKey, ecdsa};
use spark_wallet::{
    FrostJob, FrostShareResult, PrepareClaimRequest, PrepareLightningReceiveRequest,
    PrepareStaticDepositClaimRequest, PrepareStaticDepositRequest, PrepareTokenTransactionRequest,
    PrepareTransferRequest, PreparedClaim, PreparedLightningReceive, PreparedStaticDeposit,
    PreparedStaticDepositClaim, PreparedTokenTransaction, PreparedTransfer,
    SignSparkInvoiceRequest, SignStaticDepositRefundRequest, SignedSparkInvoice, SignerError,
    StartStaticDepositRefundRequest, StartedStaticDepositRefund, TreeNodeId,
};

use super::ExternalSparkSigner;
use super::external_spark_types::{
    ExternalFrostJob, ExternalFrostShareResult, ExternalPrepareClaimRequest,
    ExternalPrepareLightningReceiveRequest, ExternalPrepareStaticDepositClaimRequest,
    ExternalPrepareStaticDepositRequest, ExternalPrepareTokenTransactionRequest,
    ExternalPrepareTransferRequest, ExternalSignSparkInvoiceRequest,
    ExternalSignStaticDepositRefundRequest, ExternalStartStaticDepositRefundRequest,
};
use super::external_types::ExternalTreeNodeId;

/// Wraps a foreign [`ExternalSparkSigner`] and implements the native
/// `spark_wallet::SparkSigner` over it.
pub struct ExternalSparkSignerAdapter {
    inner: Arc<dyn ExternalSparkSigner>,
}

impl ExternalSparkSignerAdapter {
    pub fn new(inner: Arc<dyn ExternalSparkSigner>) -> Self {
        Self { inner }
    }
}

fn to_spark_err<E: std::fmt::Display>(e: E) -> SignerError {
    SignerError::Generic(e.to_string())
}

#[macros::async_trait]
impl spark_wallet::SparkSigner for ExternalSparkSignerAdapter {
    async fn get_identity_public_key(&self) -> Result<PublicKey, SignerError> {
        self.inner
            .get_identity_public_key()
            .await
            .map_err(to_spark_err)?
            .to_public_key()
            .map_err(to_spark_err)
    }

    async fn get_public_key_for_leaf(
        &self,
        leaf_id: &TreeNodeId,
    ) -> Result<PublicKey, SignerError> {
        let ext = ExternalTreeNodeId::from_tree_node_id(leaf_id).map_err(to_spark_err)?;
        self.inner
            .get_public_key_for_leaf(ext)
            .await
            .map_err(to_spark_err)?
            .to_public_key()
            .map_err(to_spark_err)
    }

    async fn get_static_deposit_public_key(&self, index: u32) -> Result<PublicKey, SignerError> {
        self.inner
            .get_static_deposit_public_key(index)
            .await
            .map_err(to_spark_err)?
            .to_public_key()
            .map_err(to_spark_err)
    }

    async fn sign_authentication_challenge(
        &self,
        challenge: &[u8],
    ) -> Result<ecdsa::Signature, SignerError> {
        self.inner
            .sign_authentication_challenge(challenge.to_vec())
            .await
            .map_err(to_spark_err)?
            .to_signature()
            .map_err(to_spark_err)
    }

    async fn sign_message(&self, message: &[u8]) -> Result<ecdsa::Signature, SignerError> {
        self.inner
            .sign_message(message.to_vec())
            .await
            .map_err(to_spark_err)?
            .to_signature()
            .map_err(to_spark_err)
    }

    async fn sign_frost(&self, jobs: Vec<FrostJob>) -> Result<Vec<FrostShareResult>, SignerError> {
        let ext_jobs = jobs
            .iter()
            .map(ExternalFrostJob::from_frost_job)
            .collect::<Result<Vec<_>, _>>()
            .map_err(to_spark_err)?;
        let results = self
            .inner
            .sign_frost(ext_jobs)
            .await
            .map_err(to_spark_err)?;
        results
            .iter()
            .map(ExternalFrostShareResult::to_frost_share_result)
            .collect::<Result<Vec<_>, _>>()
            .map_err(to_spark_err)
    }

    async fn prepare_transfer(
        &self,
        request: PrepareTransferRequest,
    ) -> Result<PreparedTransfer, SignerError> {
        let ext = ExternalPrepareTransferRequest::from_prepare_transfer_request(&request)
            .map_err(to_spark_err)?;
        self.inner
            .prepare_transfer(ext)
            .await
            .map_err(to_spark_err)?
            .to_prepared_transfer()
            .map_err(to_spark_err)
    }

    async fn prepare_claim(
        &self,
        request: PrepareClaimRequest,
    ) -> Result<PreparedClaim, SignerError> {
        let ext = ExternalPrepareClaimRequest::from_prepare_claim_request(&request)
            .map_err(to_spark_err)?;
        self.inner
            .prepare_claim(ext)
            .await
            .map_err(to_spark_err)?
            .to_prepared_claim()
            .map_err(to_spark_err)
    }

    async fn prepare_lightning_receive(
        &self,
        request: PrepareLightningReceiveRequest,
    ) -> Result<PreparedLightningReceive, SignerError> {
        let ext = ExternalPrepareLightningReceiveRequest::from_prepare_lightning_receive_request(
            &request,
        )
        .map_err(to_spark_err)?;
        self.inner
            .prepare_lightning_receive(ext)
            .await
            .map_err(to_spark_err)?
            .to_prepared_lightning_receive()
            .map_err(to_spark_err)
    }

    async fn prepare_static_deposit(
        &self,
        request: PrepareStaticDepositRequest,
    ) -> Result<PreparedStaticDeposit, SignerError> {
        let ext =
            ExternalPrepareStaticDepositRequest::from_prepare_static_deposit_request(&request)
                .map_err(to_spark_err)?;
        self.inner
            .prepare_static_deposit(ext)
            .await
            .map_err(to_spark_err)?
            .to_prepared_static_deposit()
            .map_err(to_spark_err)
    }

    async fn start_static_deposit_refund(
        &self,
        request: StartStaticDepositRefundRequest,
    ) -> Result<StartedStaticDepositRefund, SignerError> {
        let ext =
            ExternalStartStaticDepositRefundRequest::from_start_static_deposit_refund_request(
                &request,
            )
            .map_err(to_spark_err)?;
        self.inner
            .start_static_deposit_refund(ext)
            .await
            .map_err(to_spark_err)?
            .to_started_static_deposit_refund()
            .map_err(to_spark_err)
    }

    async fn sign_static_deposit_refund(
        &self,
        request: SignStaticDepositRefundRequest,
    ) -> Result<frost_secp256k1_tr::Signature, SignerError> {
        let ext = ExternalSignStaticDepositRefundRequest::from_sign_static_deposit_refund_request(
            &request,
        )
        .map_err(to_spark_err)?;
        self.inner
            .sign_static_deposit_refund(ext)
            .await
            .map_err(to_spark_err)?
            .to_frost_signature()
            .map_err(to_spark_err)
    }

    async fn sign_spark_invoice(
        &self,
        request: SignSparkInvoiceRequest,
    ) -> Result<SignedSparkInvoice, SignerError> {
        let ext = ExternalSignSparkInvoiceRequest::from_sign_spark_invoice_request(&request)
            .map_err(to_spark_err)?;
        self.inner
            .sign_spark_invoice(ext)
            .await
            .map_err(to_spark_err)?
            .to_signed_spark_invoice()
            .map_err(to_spark_err)
    }

    async fn prepare_token_transaction(
        &self,
        request: PrepareTokenTransactionRequest,
    ) -> Result<PreparedTokenTransaction, SignerError> {
        let ext = ExternalPrepareTokenTransactionRequest::from_prepare_token_transaction_request(
            &request,
        )
        .map_err(to_spark_err)?;
        self.inner
            .prepare_token_transaction(ext)
            .await
            .map_err(to_spark_err)?
            .to_prepared_token_transaction()
            .map_err(to_spark_err)
    }

    async fn prepare_static_deposit_claim(
        &self,
        request: PrepareStaticDepositClaimRequest,
    ) -> Result<PreparedStaticDepositClaim, SignerError> {
        let ext =
            ExternalPrepareStaticDepositClaimRequest::from_prepare_static_deposit_claim_request(
                &request,
            )
            .map_err(to_spark_err)?;
        self.inner
            .prepare_static_deposit_claim(ext)
            .await
            .map_err(to_spark_err)?
            .to_prepared_static_deposit_claim()
            .map_err(to_spark_err)
    }
}
