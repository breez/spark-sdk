//! Test helpers for code that signs through a [`SparkSigner`].

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use bitcoin::secp256k1::{PublicKey, ecdsa, schnorr};
use frost_secp256k1_tr::Identifier;
use frost_secp256k1_tr::round1::SigningCommitments;

use super::{
    FrostDerivation, FrostJob, FrostShareResult, PrepareClaimRequest,
    PrepareLightningReceiveRequest, PrepareStaticDepositClaimRequest, PrepareStaticDepositRequest,
    PrepareTokenTransactionRequest, PrepareTransferRequest, PreparedClaim,
    PreparedLightningReceive, PreparedStaticDeposit, PreparedStaticDepositClaim,
    PreparedTokenTransaction, PreparedTransfer, SignSparkInvoiceRequest,
    SignStaticDepositRefundRequest, SignedSparkInvoice, Signer, SignerError, SparkSigner,
    SparkSignerAdapter, StartStaticDepositRefundRequest, StartedStaticDepositRefund,
    create_test_signer,
};
use crate::tree::TreeNodeId;

/// Signs like the default adapter over the test seed, and records the key every
/// FROST job asks for.
pub(crate) struct RecordingSparkSigner {
    inner: SparkSignerAdapter,
    derivations: Mutex<Vec<FrostDerivation>>,
}

impl RecordingSparkSigner {
    pub(crate) fn new() -> Self {
        Self {
            inner: SparkSignerAdapter::new(Arc::new(create_test_signer())),
            derivations: Mutex::default(),
        }
    }

    /// The key each FROST job signed with, in the order they were signed.
    pub(crate) fn frost_derivations(&self) -> Vec<FrostDerivation> {
        self.derivations.lock().unwrap().clone()
    }
}

/// Round-1 commitments for `operators` operators, standing in for the ones the
/// coordinator hands out.
pub(crate) async fn operator_commitments(
    operators: u16,
) -> BTreeMap<Identifier, SigningCommitments> {
    let signer = create_test_signer();
    let mut commitments = BTreeMap::new();
    for id in 1..=operators {
        let commitment = signer.generate_random_signing_commitment().await.unwrap();
        commitments.insert(Identifier::try_from(id).unwrap(), commitment.commitments);
    }
    commitments
}

#[macros::async_trait]
impl SparkSigner for RecordingSparkSigner {
    async fn get_identity_public_key(&self) -> Result<PublicKey, SignerError> {
        self.inner.get_identity_public_key().await
    }

    async fn get_public_key_for_leaf(
        &self,
        leaf_id: &TreeNodeId,
    ) -> Result<PublicKey, SignerError> {
        self.inner.get_public_key_for_leaf(leaf_id).await
    }

    async fn get_static_deposit_public_key(&self, index: u32) -> Result<PublicKey, SignerError> {
        self.inner.get_static_deposit_public_key(index).await
    }

    async fn sign_authentication_challenge(
        &self,
        challenge: &[u8],
    ) -> Result<ecdsa::Signature, SignerError> {
        self.inner.sign_authentication_challenge(challenge).await
    }

    async fn sign_message(&self, message: &[u8]) -> Result<ecdsa::Signature, SignerError> {
        self.inner.sign_message(message).await
    }

    async fn sign_leaf_refund_spend(
        &self,
        leaf_id: &TreeNodeId,
        sighash: &[u8],
    ) -> Result<schnorr::Signature, SignerError> {
        self.inner.sign_leaf_refund_spend(leaf_id, sighash).await
    }

    async fn sign_frost(&self, jobs: Vec<FrostJob>) -> Result<Vec<FrostShareResult>, SignerError> {
        self.derivations
            .lock()
            .unwrap()
            .extend(jobs.iter().map(|job| job.derivation.clone()));
        self.inner.sign_frost(jobs).await
    }

    async fn prepare_transfer(
        &self,
        request: PrepareTransferRequest,
    ) -> Result<PreparedTransfer, SignerError> {
        self.inner.prepare_transfer(request).await
    }

    async fn prepare_claim(
        &self,
        request: PrepareClaimRequest,
    ) -> Result<PreparedClaim, SignerError> {
        self.inner.prepare_claim(request).await
    }

    async fn prepare_lightning_receive(
        &self,
        request: PrepareLightningReceiveRequest,
    ) -> Result<PreparedLightningReceive, SignerError> {
        self.inner.prepare_lightning_receive(request).await
    }

    async fn prepare_static_deposit(
        &self,
        request: PrepareStaticDepositRequest,
    ) -> Result<PreparedStaticDeposit, SignerError> {
        self.inner.prepare_static_deposit(request).await
    }

    async fn start_static_deposit_refund(
        &self,
        request: StartStaticDepositRefundRequest,
    ) -> Result<StartedStaticDepositRefund, SignerError> {
        self.inner.start_static_deposit_refund(request).await
    }

    async fn sign_static_deposit_refund(
        &self,
        request: SignStaticDepositRefundRequest,
    ) -> Result<frost_secp256k1_tr::Signature, SignerError> {
        self.inner.sign_static_deposit_refund(request).await
    }

    async fn prepare_static_deposit_claim(
        &self,
        request: PrepareStaticDepositClaimRequest,
    ) -> Result<PreparedStaticDepositClaim, SignerError> {
        self.inner.prepare_static_deposit_claim(request).await
    }

    async fn sign_spark_invoice(
        &self,
        request: SignSparkInvoiceRequest,
    ) -> Result<SignedSparkInvoice, SignerError> {
        self.inner.sign_spark_invoice(request).await
    }

    async fn prepare_token_transaction(
        &self,
        request: PrepareTokenTransactionRequest,
    ) -> Result<PreparedTokenTransaction, SignerError> {
        self.inner.prepare_token_transaction(request).await
    }
}
