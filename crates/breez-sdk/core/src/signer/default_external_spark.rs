//! Default in-process implementation of [`ExternalSparkSigner`].
//!
//! Backs the foreign Spark-signing trait with the same in-process signer the
//! seed path uses, converting between the FFI mirror types and the native
//! `spark_wallet` types per call. Created via
//! [`default_external_signers`](crate::default_external_signers); also serves
//! as the reference for integrators implementing the trait themselves.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::error::SignerError;
use crate::signer::external_spark::ExternalSparkSigner;
use crate::signer::external_spark_types::{
    ExternalFrostJob, ExternalFrostShareResult, ExternalNewLeafKey, ExternalOperatorPackage,
    ExternalOperatorRecipient, ExternalPrepareClaimRequest, ExternalPrepareLightningReceiveRequest,
    ExternalPrepareStaticDepositClaimRequest, ExternalPrepareStaticDepositRequest,
    ExternalPrepareTokenTransactionRequest, ExternalPrepareTransferRequest, ExternalPreparedClaim,
    ExternalPreparedLightningReceive, ExternalPreparedStaticDeposit,
    ExternalPreparedStaticDepositClaim, ExternalPreparedTokenTransaction, ExternalPreparedTransfer,
    ExternalSignSparkInvoiceRequest, ExternalSignStaticDepositRefundRequest,
    ExternalSignedSparkInvoice, ExternalSparkInvoiceKind, ExternalStartStaticDepositRefundRequest,
    ExternalStartedStaticDepositRefund, ExternalTokenTransactionKind,
};
use crate::signer::external_types::{
    EcdsaSignatureBytes, ExternalFrostCommitments, ExternalFrostSignature, ExternalTreeNodeId,
    PublicKeyBytes, SchnorrSignatureBytes, SecretBytes,
};
use crate::{Network, SdkError, Seed};
use spark_wallet::{
    ClaimLeafInput, DefaultSigner, PrepareClaimRequest, PrepareLightningReceiveRequest,
    PrepareStaticDepositClaimRequest, PrepareStaticDepositRequest, PrepareTokenTransactionRequest,
    PrepareTransferRequest, SignSparkInvoiceRequest, SignStaticDepositRefundRequest,
    SigningKeyshare, SparkInvoiceKind, SparkSigner, SparkSignerAdapter,
    StartStaticDepositRefundRequest, TokenTransactionKind, TransferLeafInput, TreeNode, TreeNodeId,
    TreeNodeStatus,
};

/// Default `ExternalSparkSigner` backed by the in-process `DefaultSigner`.
pub struct DefaultExternalSparkSigner {
    inner: SparkSignerAdapter,
}

impl DefaultExternalSparkSigner {
    /// Creates the signer from a mnemonic, deriving the same keys as the
    /// seed-based connect path.
    pub fn new(
        mnemonic: String,
        passphrase: Option<String>,
        network: Network,
        account_number: Option<u32>,
    ) -> Result<Self, SdkError> {
        let seed_bytes = Seed::Mnemonic {
            mnemonic,
            passphrase,
        }
        .to_bytes()?;
        let master = spark_wallet::account_master_key(&seed_bytes, network.into(), account_number)
            .map_err(|e| SdkError::Generic(e.to_string()))?;
        Ok(Self {
            inner: SparkSignerAdapter::new(Arc::new(DefaultSigner::from_master(master))),
        })
    }
}

fn err(e: impl std::fmt::Display) -> SignerError {
    SignerError::Generic(e.to_string())
}

fn hash_32(bytes: &[u8], what: &str) -> Result<[u8; 32], SignerError> {
    bytes
        .try_into()
        .map_err(|_| SignerError::Generic(format!("{what} must be 32 bytes")))
}

fn public_key(bytes: &[u8]) -> Result<bitcoin::secp256k1::PublicKey, SignerError> {
    bitcoin::secp256k1::PublicKey::from_slice(bytes).map_err(err)
}

/// The native leaf inputs carry a full `TreeNode` so policy-enforcing signers
/// can inspect it, but the external request conveys only the leaf id (which is
/// all the in-process signer consults: keys are derived from it). The
/// remaining fields are placeholders.
fn node_with_id(id: TreeNodeId) -> TreeNode {
    let placeholder_key =
        bitcoin::secp256k1::PublicKey::from_slice(&[2; 33]).expect("valid placeholder public key");
    TreeNode {
        id,
        tree_id: String::new(),
        value: 0,
        parent_node_id: None,
        node_tx: bitcoin::Transaction {
            version: bitcoin::transaction::Version::TWO,
            lock_time: bitcoin::absolute::LockTime::ZERO,
            input: vec![],
            output: vec![],
        },
        refund_tx: None,
        direct_tx: None,
        direct_refund_tx: None,
        direct_from_cpfp_refund_tx: None,
        vout: 0,
        verifying_public_key: placeholder_key,
        owner_identity_public_key: None,
        signing_keyshare: SigningKeyshare {
            owner_identifiers: vec![],
            threshold: 0,
            public_key: placeholder_key,
        },
        status: TreeNodeStatus::Available,
    }
}

fn operator_recipients(
    recipients: &[ExternalOperatorRecipient],
) -> Result<Vec<spark_wallet::OperatorRecipient>, SignerError> {
    recipients
        .iter()
        .map(|r| r.to_operator_recipient().map_err(err))
        .collect()
}

fn operator_packages(
    packages: &[spark_wallet::OperatorPackage],
) -> Result<Vec<ExternalOperatorPackage>, SignerError> {
    packages
        .iter()
        .map(|p| ExternalOperatorPackage::from_operator_package(p).map_err(err))
        .collect()
}

#[macros::async_trait]
impl ExternalSparkSigner for DefaultExternalSparkSigner {
    async fn get_identity_public_key(&self) -> Result<PublicKeyBytes, SignerError> {
        let pk = self.inner.get_identity_public_key().await.map_err(err)?;
        Ok(PublicKeyBytes::from_public_key(&pk))
    }

    async fn get_public_key_for_leaf(
        &self,
        leaf_id: ExternalTreeNodeId,
    ) -> Result<PublicKeyBytes, SignerError> {
        let id = leaf_id.to_tree_node_id().map_err(err)?;
        let pk = self.inner.get_public_key_for_leaf(&id).await.map_err(err)?;
        Ok(PublicKeyBytes::from_public_key(&pk))
    }

    async fn get_static_deposit_public_key(
        &self,
        index: u32,
    ) -> Result<PublicKeyBytes, SignerError> {
        let pk = self
            .inner
            .get_static_deposit_public_key(index)
            .await
            .map_err(err)?;
        Ok(PublicKeyBytes::from_public_key(&pk))
    }

    async fn sign_authentication_challenge(
        &self,
        challenge: Vec<u8>,
    ) -> Result<EcdsaSignatureBytes, SignerError> {
        let sig = self
            .inner
            .sign_authentication_challenge(&challenge)
            .await
            .map_err(err)?;
        Ok(EcdsaSignatureBytes::from_signature(&sig))
    }

    async fn sign_message(&self, message: Vec<u8>) -> Result<EcdsaSignatureBytes, SignerError> {
        let sig = self.inner.sign_message(&message).await.map_err(err)?;
        Ok(EcdsaSignatureBytes::from_signature(&sig))
    }

    async fn sign_frost(
        &self,
        jobs: Vec<ExternalFrostJob>,
    ) -> Result<Vec<ExternalFrostShareResult>, SignerError> {
        let native_jobs = jobs
            .iter()
            .map(|j| j.to_frost_job().map_err(err))
            .collect::<Result<Vec<_>, _>>()?;
        let results = self.inner.sign_frost(native_jobs).await.map_err(err)?;
        results
            .iter()
            .map(|r| ExternalFrostShareResult::from_frost_share_result(r).map_err(err))
            .collect()
    }

    async fn prepare_transfer(
        &self,
        request: ExternalPrepareTransferRequest,
    ) -> Result<ExternalPreparedTransfer, SignerError> {
        let native_request = PrepareTransferRequest {
            transfer_id: request.transfer_id.parse().map_err(err)?,
            receiver_public_key: public_key(&request.receiver_public_key)?,
            leaves: request
                .leaves
                .iter()
                .map(|l| {
                    Ok(TransferLeafInput {
                        node: node_with_id(l.node_id.to_tree_node_id().map_err(err)?),
                        new_leaf_id: l.new_leaf_id.to_tree_node_id().map_err(err)?,
                    })
                })
                .collect::<Result<Vec<_>, SignerError>>()?,
            operator_recipients: operator_recipients(&request.operator_recipients)?,
            threshold: request.threshold,
        };
        let prepared = self
            .inner
            .prepare_transfer(native_request)
            .await
            .map_err(err)?;
        Ok(ExternalPreparedTransfer {
            operator_packages: operator_packages(&prepared.operator_packages)?,
            new_leaf_keys: prepared
                .new_leaf_keys
                .iter()
                .map(|k| {
                    Ok(ExternalNewLeafKey {
                        node_id: ExternalTreeNodeId::from_tree_node_id(&k.node_id).map_err(err)?,
                        new_signing_public_key: k.new_signing_public_key.serialize().to_vec(),
                    })
                })
                .collect::<Result<Vec<_>, SignerError>>()?,
            transfer_user_signature: EcdsaSignatureBytes::from_signature(
                &prepared.transfer_user_signature,
            ),
        })
    }

    async fn prepare_claim(
        &self,
        request: ExternalPrepareClaimRequest,
    ) -> Result<ExternalPreparedClaim, SignerError> {
        let native_request = PrepareClaimRequest {
            transfer_id: request.transfer_id.parse().map_err(err)?,
            sender_identity_public_key: public_key(&request.sender_identity_public_key)?,
            leaves: request
                .leaves
                .iter()
                .map(|l| {
                    Ok(ClaimLeafInput {
                        node: node_with_id(l.node_id.to_tree_node_id().map_err(err)?),
                        sender_signature: l.sender_signature.clone(),
                        leaf_key_ciphertext: l.leaf_key_ciphertext.clone(),
                    })
                })
                .collect::<Result<Vec<_>, SignerError>>()?,
            operator_recipients: operator_recipients(&request.operator_recipients)?,
            threshold: request.threshold,
        };
        let prepared = self
            .inner
            .prepare_claim(native_request)
            .await
            .map_err(err)?;
        Ok(ExternalPreparedClaim {
            operator_packages: operator_packages(&prepared.operator_packages)?,
        })
    }

    async fn prepare_lightning_receive(
        &self,
        request: ExternalPrepareLightningReceiveRequest,
    ) -> Result<ExternalPreparedLightningReceive, SignerError> {
        let native_request = PrepareLightningReceiveRequest {
            operator_recipients: operator_recipients(&request.operator_recipients)?,
            threshold: request.threshold,
        };
        let prepared = self
            .inner
            .prepare_lightning_receive(native_request)
            .await
            .map_err(err)?;
        Ok(ExternalPreparedLightningReceive {
            payment_hash: prepared.payment_hash.to_vec(),
            operator_preimage_packages: operator_packages(&prepared.operator_preimage_packages)?,
        })
    }

    async fn prepare_static_deposit(
        &self,
        request: ExternalPrepareStaticDepositRequest,
    ) -> Result<ExternalPreparedStaticDeposit, SignerError> {
        let native_request = PrepareStaticDepositRequest {
            index: request.index,
            ssp_public_key: public_key(&request.ssp_public_key)?,
            frost_jobs: request
                .frost_jobs
                .iter()
                .map(|j| j.to_frost_job().map_err(err))
                .collect::<Result<Vec<_>, _>>()?,
        };
        let prepared = self
            .inner
            .prepare_static_deposit(native_request)
            .await
            .map_err(err)?;
        Ok(ExternalPreparedStaticDeposit {
            exported_secret: prepared.exported_secret,
            frost_shares: prepared
                .frost_shares
                .iter()
                .map(|r| ExternalFrostShareResult::from_frost_share_result(r).map_err(err))
                .collect::<Result<Vec<_>, _>>()?,
        })
    }

    async fn start_static_deposit_refund(
        &self,
        request: ExternalStartStaticDepositRefundRequest,
    ) -> Result<ExternalStartedStaticDepositRefund, SignerError> {
        let started = self
            .inner
            .start_static_deposit_refund(StartStaticDepositRefundRequest {
                index: request.index,
                user_statement: request.user_statement,
            })
            .await
            .map_err(err)?;
        Ok(ExternalStartedStaticDepositRefund {
            signing_public_key: started.signing_public_key.serialize().to_vec(),
            nonce_commitment: ExternalFrostCommitments::from_frost_commitments(
                &started.nonce_commitment,
            )
            .map_err(err)?,
            user_signature: EcdsaSignatureBytes::from_signature(&started.user_signature),
        })
    }

    async fn sign_static_deposit_refund(
        &self,
        request: ExternalSignStaticDepositRefundRequest,
    ) -> Result<ExternalFrostSignature, SignerError> {
        let statechain_commitments = request
            .statechain_commitments
            .iter()
            .map(|p| {
                Ok((
                    p.identifier.to_identifier().map_err(err)?,
                    p.commitment.to_signing_commitments().map_err(err)?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>, SignerError>>()?;
        let statechain_signatures = request
            .statechain_signatures
            .iter()
            .map(|p| {
                Ok((
                    p.identifier.to_identifier().map_err(err)?,
                    p.signature.to_signature_share().map_err(err)?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>, SignerError>>()?;
        let statechain_public_keys = request
            .statechain_public_keys
            .iter()
            .map(|p| {
                Ok((
                    p.identifier.to_identifier().map_err(err)?,
                    public_key(&p.public_key)?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>, SignerError>>()?;
        let signature = self
            .inner
            .sign_static_deposit_refund(SignStaticDepositRefundRequest {
                index: request.index,
                sighash: hash_32(&request.sighash, "refund sighash")?,
                verifying_key: public_key(&request.verifying_key)?,
                nonce_commitment: request
                    .nonce_commitment
                    .to_frost_commitments()
                    .map_err(err)?,
                statechain_commitments,
                statechain_signatures,
                statechain_public_keys,
            })
            .await
            .map_err(err)?;
        ExternalFrostSignature::from_frost_signature(&signature).map_err(err)
    }

    async fn sign_spark_invoice(
        &self,
        request: ExternalSignSparkInvoiceRequest,
    ) -> Result<ExternalSignedSparkInvoice, SignerError> {
        let signed = self
            .inner
            .sign_spark_invoice(SignSparkInvoiceRequest {
                kind: match request.kind {
                    ExternalSparkInvoiceKind::Sats => SparkInvoiceKind::Sats,
                    ExternalSparkInvoiceKind::Tokens => SparkInvoiceKind::Tokens,
                },
                invoice_hash: hash_32(&request.invoice_hash, "invoice hash")?,
            })
            .await
            .map_err(err)?;
        Ok(ExternalSignedSparkInvoice {
            signature: SchnorrSignatureBytes::from_signature(&signed.signature),
        })
    }

    async fn prepare_token_transaction(
        &self,
        request: ExternalPrepareTokenTransactionRequest,
    ) -> Result<ExternalPreparedTokenTransaction, SignerError> {
        let prepared = self
            .inner
            .prepare_token_transaction(PrepareTokenTransactionRequest {
                kind: match request.kind {
                    ExternalTokenTransactionKind::Freeze => TokenTransactionKind::Freeze,
                    ExternalTokenTransactionKind::Partial => TokenTransactionKind::Partial,
                    ExternalTokenTransactionKind::Final => TokenTransactionKind::Final,
                },
                digest: hash_32(&request.digest, "token transaction digest")?,
            })
            .await
            .map_err(err)?;
        Ok(ExternalPreparedTokenTransaction {
            signature: SchnorrSignatureBytes::from_signature(&prepared.signature),
        })
    }

    async fn prepare_static_deposit_claim(
        &self,
        request: ExternalPrepareStaticDepositClaimRequest,
    ) -> Result<ExternalPreparedStaticDepositClaim, SignerError> {
        let prepared = self
            .inner
            .prepare_static_deposit_claim(PrepareStaticDepositClaimRequest {
                index: request.index,
                user_statement: request.user_statement,
            })
            .await
            .map_err(err)?;
        Ok(ExternalPreparedStaticDepositClaim {
            deposit_secret_key: SecretBytes::from_secret_key(&prepared.deposit_secret_key),
            user_signature: EcdsaSignatureBytes::from_signature(&prepared.user_signature),
        })
    }
}
