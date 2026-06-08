//! `SparkSigner` implementation backed by Turnkey.
//!
//! Spark keys are hardened children of `m/8797555'/{account}'`: identity at
//! `/0'`, signing leaf at `/1'/{u32_be(sha256(leaf_id)[..4]) % 2^31}'`, static
//! deposit at `/3'/{index}'`. The signing flows pass Turnkey opaque derivation
//! tags rather than paths; the pure-pubkey methods materialize the account at
//! the path (deterministic from the wallet seed, so recoverable without local
//! state) and read its compressed key.
//!
//! The pubkey maps are a performance memoization only. Each derivation's key is
//! deterministic and immutable, so caching avoids repeat Turnkey round-trips
//! (and re-issuing a `CREATE_WALLET_ACCOUNTS` activity for a path already
//! materialized), and never needs invalidation. They are not persistence:
//! in-memory, lost on restart, rebuilt from Turnkey on demand. Correctness does
//! not depend on them.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use bitcoin::bip32::DerivationPath;
use bitcoin::hashes::{Hash, sha256};
use bitcoin::secp256k1::{PublicKey, SecretKey, ecdsa, schnorr};
use frost_secp256k1_tr::Identifier;
use frost_secp256k1_tr::round1::{NonceCommitment, SigningCommitments};
use frost_secp256k1_tr::round2::SignatureShare;
use spark_wallet::{
    AggregateFrostRequest, DefaultSigner, FrostDerivation, FrostJob, FrostShareResult,
    FrostSigningCommitmentsWithNonces, NewLeafKey, OperatorPackage, OperatorRecipient,
    PrepareClaimRequest, PrepareLightningReceiveRequest, PrepareStaticDepositClaimRequest,
    PrepareStaticDepositRequest, PrepareTokenTransactionRequest, PrepareTransferRequest,
    PreparedClaim, PreparedLightningReceive, PreparedStaticDeposit, PreparedStaticDepositClaim,
    PreparedTokenTransaction, PreparedTransfer, SecretSource, SignFrostRequest,
    SignSparkInvoiceRequest, SignStaticDepositRefundRequest, SignedSparkInvoice, Signer,
    SignerError, SparkSigner, StartStaticDepositRefundRequest, StartedStaticDepositRefund,
    TreeNodeId, aggregate_frost,
};

use crate::Network;

use turnkey_enclave_encrypt::{ExportClient, QuorumPublicKey};

use super::accounts::{
    decode_scalar_32, ecdsa_from_rs, schnorr_from_rs, spark_address_format, xpriv_from_secret,
};
use super::error::TurnkeyError;
use super::transport::TurnkeyClient;
use super::types::{
    ADDRESS_FORMAT_COMPRESSED, EXPORT_WALLET_ACCOUNT_PATH, EXPORT_WALLET_ACCOUNT_RESULT,
    EXPORT_WALLET_ACCOUNT_TYPE, ExportWalletAccountIntent, ExportWalletAccountResult,
    HASH_FUNCTION_NO_OP, HASH_FUNCTION_SHA256, SPARK_CLAIM_TRANSFER_PATH,
    SPARK_CLAIM_TRANSFER_RESULT, SPARK_CLAIM_TRANSFER_TYPE, SPARK_PREPARE_LIGHTNING_RECEIVE_PATH,
    SPARK_PREPARE_LIGHTNING_RECEIVE_RESULT, SPARK_PREPARE_LIGHTNING_RECEIVE_TYPE,
    SPARK_PREPARE_TRANSFER_PATH, SPARK_PREPARE_TRANSFER_RESULT, SPARK_PREPARE_TRANSFER_TYPE,
    SPARK_SIGN_FROST_PATH, SPARK_SIGN_FROST_RESULT, SPARK_SIGN_FROST_TYPE, SparkClaimLeaf,
    SparkClaimPackage, SparkClaimTransferIntent, SparkClaimTransferResult,
    SparkEncryptedOperatorPackage, SparkFrostCommitment, SparkKeyDerivation, SparkLeafPublicKey,
    SparkLightningReceivePackage, SparkOperatorRecipient, SparkPartialSignature,
    SparkPrepareLightningReceiveIntent, SparkPrepareLightningReceiveResult,
    SparkPrepareTransferIntent, SparkPrepareTransferResult, SparkSignFrostIntent,
    SparkSignFrostResult, SparkSignatureRequest, SparkTransferLeaf, SparkTransferPackage,
};

fn to_spark_err<E: std::fmt::Display>(e: E) -> SignerError {
    SignerError::Generic(e.to_string())
}

fn frost_derivation(derivation: &FrostDerivation) -> SparkKeyDerivation {
    match derivation {
        FrostDerivation::SigningLeaf { leaf_id } => {
            SparkKeyDerivation::signing_leaf(leaf_id.to_string())
        }
        FrostDerivation::StaticDeposit { index } => SparkKeyDerivation::static_deposit(*index),
        FrostDerivation::HtlcPreimage => SparkKeyDerivation::htlc_preimage(),
        FrostDerivation::Identity => SparkKeyDerivation::identity(),
    }
}

/// Converts a native FROST job into the request shape, hex-encoding the sighash,
/// verifying key, per-operator commitments, and optional adaptor key.
fn frost_job_to_request(job: &FrostJob) -> Result<SparkSignatureRequest, TurnkeyError> {
    let mut operator_commitments = Vec::with_capacity(job.operator_commitments.len());
    for (identifier, commitment) in &job.operator_commitments {
        operator_commitments.push(SparkFrostCommitment {
            id: hex::encode(identifier.serialize()),
            hiding: hex::encode(
                commitment
                    .hiding()
                    .serialize()
                    .map_err(|e| TurnkeyError::Serialize(e.to_string()))?,
            ),
            binding: hex::encode(
                commitment
                    .binding()
                    .serialize()
                    .map_err(|e| TurnkeyError::Serialize(e.to_string()))?,
            ),
        });
    }
    Ok(SparkSignatureRequest {
        derivation: frost_derivation(&job.derivation),
        message: hex::encode(job.sighash),
        verifying_key: hex::encode(job.verifying_key.serialize()),
        operator_commitments,
        adaptor_public_key: job.adaptor_public_key.map(|pk| hex::encode(pk.serialize())),
    })
}

/// Rebuilds a `FrostShareResult` from Turnkey's partial signature. The secret
/// nonces stay in the enclave, so `nonces_ciphertext` is empty: downstream code
/// only reads the public commitment and the share (see module docs).
fn partial_signature_to_share(
    sig: &SparkPartialSignature,
) -> Result<FrostShareResult, TurnkeyError> {
    let decode = |what: &str, value: &str| -> Result<Vec<u8>, TurnkeyError> {
        hex::decode(value).map_err(|e| TurnkeyError::Deserialize(format!("{what}: {e}")))
    };
    let hiding = NonceCommitment::deserialize(&decode("hiding", &sig.hiding)?)
        .map_err(|e| TurnkeyError::Deserialize(e.to_string()))?;
    let binding = NonceCommitment::deserialize(&decode("binding", &sig.binding)?)
        .map_err(|e| TurnkeyError::Deserialize(e.to_string()))?;
    let signature_share =
        SignatureShare::deserialize(&decode("signatureShare", &sig.signature_share)?)
            .map_err(|e| TurnkeyError::Deserialize(e.to_string()))?;
    Ok(FrostShareResult {
        commitment: FrostSigningCommitmentsWithNonces {
            commitments: SigningCommitments::new(hiding, binding),
            nonces_ciphertext: Vec::new(),
        },
        signature_share,
    })
}

fn operator_recipients(recipients: &[OperatorRecipient]) -> Vec<SparkOperatorRecipient> {
    recipients
        .iter()
        .map(|r| SparkOperatorRecipient {
            operator_id: hex::encode(r.identifier.serialize()),
            encryption_public_key: hex::encode(r.public_key.serialize()),
        })
        .collect()
}

fn operator_package_from(
    pkg: &SparkEncryptedOperatorPackage,
) -> Result<OperatorPackage, TurnkeyError> {
    let id_bytes = hex::decode(&pkg.operator_id)
        .map_err(|e| TurnkeyError::Deserialize(format!("operatorId: {e}")))?;
    let operator_identifier =
        Identifier::deserialize(&id_bytes).map_err(|e| TurnkeyError::Deserialize(e.to_string()))?;
    let encrypted_package = hex::decode(&pkg.encrypted_package)
        .map_err(|e| TurnkeyError::Deserialize(format!("encryptedPackage: {e}")))?;
    Ok(OperatorPackage {
        operator_identifier,
        encrypted_package,
    })
}

fn new_leaf_keys_from(leaves: &[SparkLeafPublicKey]) -> Result<Vec<NewLeafKey>, SignerError> {
    leaves
        .iter()
        .map(|lpk| {
            Ok(NewLeafKey {
                node_id: TreeNodeId::from_str(&lpk.leaf_id).map_err(to_spark_err)?,
                new_signing_public_key: PublicKey::from_str(&lpk.public_key)
                    .map_err(to_spark_err)?,
            })
        })
        .collect()
}

pub(crate) struct TurnkeySparkSigner {
    client: Arc<TurnkeyClient>,
    account: u32,
    network: Network,
    // Pubkey/address memoization (see module docs): immutable, in-memory, non-authoritative.
    identity_pubkey: Mutex<Option<PublicKey>>,
    spark_address: Mutex<Option<String>>,
    leaf_pubkeys: Mutex<HashMap<TreeNodeId, PublicKey>>,
    static_deposit_pubkeys: Mutex<HashMap<u32, PublicKey>>,
    // Static-deposit secret keys exported from Turnkey, cached so the refund
    // start/sign pair need not re-export. Exportable by design (the SSP co-signs
    // with them); in-memory only, never persisted.
    static_deposit_secret_keys: Mutex<HashMap<u32, SecretKey>>,
}

impl TurnkeySparkSigner {
    pub(crate) fn new(client: Arc<TurnkeyClient>, network: Network) -> Self {
        Self {
            client,
            account: 0,
            network,
            identity_pubkey: Mutex::new(None),
            spark_address: Mutex::new(None),
            leaf_pubkeys: Mutex::new(HashMap::new()),
            static_deposit_pubkeys: Mutex::new(HashMap::new()),
            static_deposit_secret_keys: Mutex::new(HashMap::new()),
        }
    }

    fn base_path(&self) -> String {
        format!("m/8797555'/{}'", self.account)
    }

    fn leaf_index(leaf_id: &TreeNodeId) -> u32 {
        let hash = sha256::Hash::hash(leaf_id.to_string().as_bytes());
        let bytes: [u8; 4] = hash.as_byte_array()[..4]
            .try_into()
            .expect("sha256 digest is 32 bytes");
        u32::from_be_bytes(bytes) % (1 << 31)
    }

    async fn pubkey_at_path(&self, path: String) -> Result<PublicKey, SignerError> {
        let hex = self
            .client
            .compressed_pubkey_at(path)
            .await
            .map_err(to_spark_err)?;
        PublicKey::from_str(&hex).map_err(to_spark_err)
    }

    /// The Spark-format identity address, used as `signWith` for Spark-protocol
    /// activities and BIP-340 Schnorr signing.
    async fn spark_identity_address(&self) -> Result<String, SignerError> {
        if let Some(addr) = self.spark_address.lock().unwrap().clone() {
            return Ok(addr);
        }
        let format = spark_address_format(self.network);
        let path = format!("{}/0'", self.base_path());
        let addr = self
            .client
            .create_account(path, format)
            .await
            .map_err(to_spark_err)?;
        *self.spark_address.lock().unwrap() = Some(addr.clone());
        Ok(addr)
    }

    /// BIP-340 Schnorr-signs a 32-byte `hash` with the identity key via
    /// `SIGN_RAW_PAYLOAD`. The Spark-format `signWith` selects Schnorr, and the
    /// hash is signed as-is (`NO_OP`), matching the local signer.
    async fn sign_identity_schnorr(&self, hash: &[u8]) -> Result<schnorr::Signature, SignerError> {
        let sign_with = self.spark_identity_address().await?;
        let result = self
            .client
            .sign_raw(sign_with, hex::encode(hash), HASH_FUNCTION_NO_OP)
            .await
            .map_err(to_spark_err)?;
        schnorr_from_rs(&result.r, &result.s).map_err(to_spark_err)
    }

    /// ECDSA-signs `message` with the identity key via `SIGN_RAW_PAYLOAD`.
    /// `signWith` is the compressed identity address, which selects secp256k1
    /// ECDSA (the Spark-format address would instead select BIP-340 Schnorr).
    /// Turnkey applies SHA-256 to the payload, matching the local signer.
    async fn sign_identity_ecdsa(&self, message: &[u8]) -> Result<ecdsa::Signature, SignerError> {
        let identity = self.get_identity_public_key().await?;
        let result = self
            .client
            .sign_raw(
                hex::encode(identity.serialize()),
                hex::encode(message),
                HASH_FUNCTION_SHA256,
            )
            .await
            .map_err(to_spark_err)?;
        ecdsa_from_rs(&result.r, &result.s).map_err(to_spark_err)
    }

    /// Seeds the leaf cache from a claim/transfer result. Each leaf's signing key
    /// is `HD(leaf_id)`, matching what `get_public_key_for_leaf` derives.
    fn cache_new_leaf_keys(&self, leaves: &[SparkLeafPublicKey]) -> Result<(), SignerError> {
        let mut cache = self.leaf_pubkeys.lock().unwrap();
        for lpk in leaves {
            let id = TreeNodeId::from_str(&lpk.leaf_id).map_err(to_spark_err)?;
            let pk = PublicKey::from_str(&lpk.public_key).map_err(to_spark_err)?;
            cache.insert(id, pk);
        }
        Ok(())
    }

    /// Exports the static-deposit secret key at `index` from Turnkey, decrypting
    /// the bundle against the pinned quorum key. Cached in-memory.
    async fn export_static_deposit_key(&self, index: u32) -> Result<SecretKey, SignerError> {
        if let Some(secret) = self
            .static_deposit_secret_keys
            .lock()
            .unwrap()
            .get(&index)
            .copied()
        {
            return Ok(secret);
        }
        let path = format!("{}/3'/{index}'", self.base_path());
        let address = self
            .client
            .create_account(path, ADDRESS_FORMAT_COMPRESSED)
            .await
            .map_err(to_spark_err)?;
        let mut export_client = ExportClient::new(&QuorumPublicKey::production_signer());
        let target_public_key = export_client.target_public_key().map_err(to_spark_err)?;
        let result: ExportWalletAccountResult = self
            .client
            .submit_activity(
                EXPORT_WALLET_ACCOUNT_PATH,
                EXPORT_WALLET_ACCOUNT_TYPE,
                ExportWalletAccountIntent {
                    address,
                    target_public_key,
                },
                EXPORT_WALLET_ACCOUNT_RESULT,
            )
            .await
            .map_err(to_spark_err)?;
        let private_bytes = export_client
            .decrypt_private_key(&result.export_bundle, &self.client.organization_id)
            .map_err(to_spark_err)?;
        let secret = SecretKey::from_slice(&private_bytes).map_err(to_spark_err)?;
        self.static_deposit_secret_keys
            .lock()
            .unwrap()
            .insert(index, secret);
        Ok(secret)
    }

    /// A local in-process signer rooted at the exported static-deposit key, so
    /// the refund FROST and SSP-export ECIES reuse the existing machinery (the
    /// key is addressed via the empty derivation path).
    async fn local_static_deposit_signer(&self, index: u32) -> Result<DefaultSigner, SignerError> {
        let secret = self.export_static_deposit_key(index).await?;
        Ok(DefaultSigner::from_master(xpriv_from_secret(
            secret,
            self.network,
        )))
    }
}

#[macros::async_trait]
impl spark_wallet::SparkSigner for TurnkeySparkSigner {
    async fn get_identity_public_key(&self) -> Result<PublicKey, SignerError> {
        if let Some(pk) = *self.identity_pubkey.lock().unwrap() {
            return Ok(pk);
        }
        let pk = self
            .pubkey_at_path(format!("{}/0'", self.base_path()))
            .await?;
        *self.identity_pubkey.lock().unwrap() = Some(pk);
        Ok(pk)
    }

    async fn get_public_key_for_leaf(
        &self,
        leaf_id: &TreeNodeId,
    ) -> Result<PublicKey, SignerError> {
        if let Some(pk) = self.leaf_pubkeys.lock().unwrap().get(leaf_id).copied() {
            return Ok(pk);
        }
        let path = format!("{}/1'/{}'", self.base_path(), Self::leaf_index(leaf_id));
        let pk = self.pubkey_at_path(path).await?;
        self.leaf_pubkeys
            .lock()
            .unwrap()
            .insert(leaf_id.clone(), pk);
        Ok(pk)
    }

    async fn get_static_deposit_public_key(&self, index: u32) -> Result<PublicKey, SignerError> {
        if let Some(pk) = self
            .static_deposit_pubkeys
            .lock()
            .unwrap()
            .get(&index)
            .copied()
        {
            return Ok(pk);
        }
        let path = format!("{}/3'/{index}'", self.base_path());
        let pk = self.pubkey_at_path(path).await?;
        self.static_deposit_pubkeys
            .lock()
            .unwrap()
            .insert(index, pk);
        Ok(pk)
    }

    async fn sign_authentication_challenge(
        &self,
        challenge: &[u8],
    ) -> Result<ecdsa::Signature, SignerError> {
        self.sign_identity_ecdsa(challenge).await
    }

    async fn sign_message(&self, message: &[u8]) -> Result<ecdsa::Signature, SignerError> {
        self.sign_identity_ecdsa(message).await
    }

    async fn sign_frost(&self, jobs: Vec<FrostJob>) -> Result<Vec<FrostShareResult>, SignerError> {
        let sign_with = self.spark_identity_address().await?;
        let mut signatures = Vec::with_capacity(jobs.len());
        for job in &jobs {
            signatures.push(frost_job_to_request(job).map_err(to_spark_err)?);
        }
        let result: SparkSignFrostResult = self
            .client
            .submit_activity(
                SPARK_SIGN_FROST_PATH,
                SPARK_SIGN_FROST_TYPE,
                SparkSignFrostIntent {
                    sign_with,
                    signatures,
                },
                SPARK_SIGN_FROST_RESULT,
            )
            .await
            .map_err(to_spark_err)?;
        if result.signatures.len() != jobs.len() {
            return Err(SignerError::Generic(format!(
                "turnkey sign_frost: expected {} shares, got {}",
                jobs.len(),
                result.signatures.len()
            )));
        }
        result
            .signatures
            .iter()
            .map(|sig| partial_signature_to_share(sig).map_err(to_spark_err))
            .collect()
    }

    async fn prepare_transfer(
        &self,
        request: PrepareTransferRequest,
    ) -> Result<PreparedTransfer, SignerError> {
        let sign_with = self.spark_identity_address().await?;
        let leaves = request
            .leaves
            .iter()
            .map(|leaf| SparkTransferLeaf {
                leaf_id: leaf.node.id.to_string(),
                old_leaf_derivation: SparkKeyDerivation::signing_leaf(leaf.node.id.to_string()),
                new_leaf_derivation: SparkKeyDerivation::signing_leaf(leaf.new_leaf_id.to_string()),
                refund_signature: None,
                direct_refund_signature: None,
                direct_from_cpfp_refund_signature: None,
            })
            .collect();
        let intent = SparkPrepareTransferIntent {
            sign_with,
            transfer: SparkTransferPackage {
                transfer_id: request.transfer_id.to_string(),
                leaves,
                threshold: request.threshold,
                operator_recipients: operator_recipients(&request.operator_recipients),
                receiver_public_key: hex::encode(request.receiver_public_key.serialize()),
            },
        };
        let result: SparkPrepareTransferResult = self
            .client
            .submit_activity(
                SPARK_PREPARE_TRANSFER_PATH,
                SPARK_PREPARE_TRANSFER_TYPE,
                intent,
                SPARK_PREPARE_TRANSFER_RESULT,
            )
            .await
            .map_err(to_spark_err)?;
        let operator_packages = result
            .operator_packages
            .iter()
            .map(|pkg| operator_package_from(pkg).map_err(to_spark_err))
            .collect::<Result<Vec<_>, _>>()?;
        let new_leaf_keys = new_leaf_keys_from(&result.new_leaf_public_keys)?;
        let der = hex::decode(&result.transfer_user_signature)
            .map_err(|e| TurnkeyError::Deserialize(format!("transferUserSignature: {e}")))
            .map_err(to_spark_err)?;
        let transfer_user_signature = ecdsa::Signature::from_der(&der).map_err(to_spark_err)?;
        Ok(PreparedTransfer {
            operator_packages,
            new_leaf_keys,
            transfer_user_signature,
        })
    }

    async fn prepare_claim(
        &self,
        request: PrepareClaimRequest,
    ) -> Result<PreparedClaim, SignerError> {
        let sign_with = self.spark_identity_address().await?;
        let leaves = request
            .leaves
            .iter()
            .map(|leaf| SparkClaimLeaf {
                leaf_id: leaf.node.id.to_string(),
                ciphertext: hex::encode(&leaf.leaf_key_ciphertext),
                sender_signature: hex::encode(&leaf.sender_signature),
            })
            .collect();
        let intent = SparkClaimTransferIntent {
            sign_with,
            claim: SparkClaimPackage {
                leaves,
                threshold: request.threshold,
                transfer_id: request.transfer_id.to_string(),
                operator_recipients: operator_recipients(&request.operator_recipients),
                sender_identity_public_key: hex::encode(
                    request.sender_identity_public_key.serialize(),
                ),
            },
        };
        let result: SparkClaimTransferResult = self
            .client
            .submit_activity(
                SPARK_CLAIM_TRANSFER_PATH,
                SPARK_CLAIM_TRANSFER_TYPE,
                intent,
                SPARK_CLAIM_TRANSFER_RESULT,
            )
            .await
            .map_err(to_spark_err)?;
        self.cache_new_leaf_keys(&result.new_leaf_public_keys)?;
        let operator_packages = result
            .operator_packages
            .iter()
            .map(|pkg| operator_package_from(pkg).map_err(to_spark_err))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(PreparedClaim { operator_packages })
    }

    async fn prepare_lightning_receive(
        &self,
        request: PrepareLightningReceiveRequest,
    ) -> Result<PreparedLightningReceive, SignerError> {
        let sign_with = self.spark_identity_address().await?;
        let intent = SparkPrepareLightningReceiveIntent {
            sign_with,
            lightning_receive: SparkLightningReceivePackage {
                threshold: request.threshold,
                operator_recipients: operator_recipients(&request.operator_recipients),
            },
        };
        let result: SparkPrepareLightningReceiveResult = self
            .client
            .submit_activity(
                SPARK_PREPARE_LIGHTNING_RECEIVE_PATH,
                SPARK_PREPARE_LIGHTNING_RECEIVE_TYPE,
                intent,
                SPARK_PREPARE_LIGHTNING_RECEIVE_RESULT,
            )
            .await
            .map_err(to_spark_err)?;
        let payment_hash = decode_scalar_32(&result.payment_hash).map_err(to_spark_err)?;
        let operator_preimage_packages = result
            .operator_packages
            .iter()
            .map(|pkg| operator_package_from(pkg).map_err(to_spark_err))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(PreparedLightningReceive {
            payment_hash,
            operator_preimage_packages,
        })
    }

    async fn prepare_static_deposit(
        &self,
        request: PrepareStaticDepositRequest,
    ) -> Result<PreparedStaticDeposit, SignerError> {
        let PrepareStaticDepositRequest {
            index,
            ssp_public_key,
            frost_jobs,
        } = request;
        // Export the static-deposit secret and ECIES-encrypt it to the SSP via a
        // local signer rooted at that key (empty path = the key itself).
        let local = self.local_static_deposit_signer(index).await?;
        let exported_secret = local
            .encrypt_secret_for_receiver(
                &SecretSource::Derived(DerivationPath::master()),
                &ssp_public_key,
            )
            .await?;
        // The deposit tree-tx FROST stays in Turnkey (SPARK_SIGN_FROST).
        let frost_shares = self.sign_frost(frost_jobs).await?;
        Ok(PreparedStaticDeposit {
            exported_secret,
            frost_shares,
        })
    }

    async fn start_static_deposit_refund(
        &self,
        request: StartStaticDepositRefundRequest,
    ) -> Result<StartedStaticDepositRefund, SignerError> {
        let StartStaticDepositRefundRequest {
            index,
            user_statement,
        } = request;
        let signing_public_key = self.get_static_deposit_public_key(index).await?;
        // User-commits-first: the nonce (encrypted to the local signer) is
        // generated now and reconstructed in sign_static_deposit_refund.
        let nonce_commitment = self
            .local_static_deposit_signer(index)
            .await?
            .generate_random_signing_commitment()
            .await?;
        let user_signature = self.sign_identity_ecdsa(&user_statement).await?;
        Ok(StartedStaticDepositRefund {
            signing_public_key,
            nonce_commitment,
            user_signature,
        })
    }

    async fn sign_static_deposit_refund(
        &self,
        request: SignStaticDepositRefundRequest,
    ) -> Result<frost_secp256k1_tr::Signature, SignerError> {
        let SignStaticDepositRefundRequest {
            index,
            sighash,
            verifying_key,
            nonce_commitment,
            statechain_commitments,
            statechain_signatures,
            statechain_public_keys,
        } = request;
        let local = self.local_static_deposit_signer(index).await?;
        let aggregating_public_key = self.get_static_deposit_public_key(index).await?;
        // Sign with the pre-committed nonce, then aggregate (pure public math).
        let user_signature = local
            .sign_frost(SignFrostRequest {
                message: &sighash,
                public_key: &verifying_key,
                private_key: &SecretSource::Derived(DerivationPath::master()),
                verifying_key: &verifying_key,
                self_nonce_commitment: &nonce_commitment,
                statechain_commitments: statechain_commitments.clone(),
                adaptor_public_key: None,
            })
            .await?;
        aggregate_frost(AggregateFrostRequest {
            message: &sighash,
            statechain_signatures,
            statechain_public_keys,
            verifying_key: &verifying_key,
            statechain_commitments,
            self_commitment: &nonce_commitment.commitments,
            public_key: &aggregating_public_key,
            self_signature: &user_signature,
            adaptor_public_key: None,
        })
    }

    async fn prepare_static_deposit_claim(
        &self,
        request: PrepareStaticDepositClaimRequest,
    ) -> Result<PreparedStaticDepositClaim, SignerError> {
        let deposit_secret_key = self.export_static_deposit_key(request.index).await?;
        let user_signature = self.sign_identity_ecdsa(&request.user_statement).await?;
        Ok(PreparedStaticDepositClaim {
            deposit_secret_key,
            user_signature,
        })
    }

    async fn sign_spark_invoice(
        &self,
        request: SignSparkInvoiceRequest,
    ) -> Result<SignedSparkInvoice, SignerError> {
        let signature = self.sign_identity_schnorr(&request.invoice_hash).await?;
        Ok(SignedSparkInvoice { signature })
    }

    async fn prepare_token_transaction(
        &self,
        request: PrepareTokenTransactionRequest,
    ) -> Result<PreparedTokenTransaction, SignerError> {
        let signature = self.sign_identity_schnorr(&request.digest).await?;
        Ok(PreparedTokenTransaction { signature })
    }
}
