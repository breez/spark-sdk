//! `ExternalSparkSigner` implementation backed by Turnkey.
//!
//! Spark keys are hardened children of `m/8797555'/{account}'`: identity at
//! `/0'`, signing leaf at `/1'/{u32_be(sha256(leaf_id)[..4]) % 2^31}'`, static
//! deposit at `/3'/{index}'`. The signing flows pass Turnkey opaque derivation
//! tags rather than paths; the pure-pubkey methods materialize the account at
//! the path (deterministic from the wallet seed, so recoverable without local
//! state) and read its compressed key.
//!
//! The trait surface is the FFI [`ExternalSparkSigner`] (so the signer is
//! exposable over uniffi and consumed via `ExternalSparkSignerAdapter`). It
//! carries exactly the signer-relevant subset of each request (leaf ids, not
//! full tree nodes), which is all this signer ever reads; the native-typed
//! inherent helpers below do the Turnkey work, and the trait methods are thin
//! conversions to and from the FFI types.
//!
//! The pubkey maps are a performance memoization only. Each derivation's key is
//! deterministic and immutable, so caching avoids repeat Turnkey round-trips
//! (and re-issuing a `CREATE_WALLET_ACCOUNTS` activity for a path already
//! materialized), and never needs invalidation. They are not persistence:
//! in-memory, lost on restart, rebuilt from Turnkey on demand. Correctness does
//! not depend on them.

use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;
use std::sync::Arc;

use tokio::sync::Mutex;

use bitcoin::bip32::DerivationPath;
use bitcoin::hashes::{Hash, sha256};
use bitcoin::secp256k1::{PublicKey, SecretKey, ecdsa, schnorr};
use frost_secp256k1_tr::Identifier;
use frost_secp256k1_tr::round1::{NonceCommitment, SigningCommitments};
use frost_secp256k1_tr::round2::SignatureShare;
use spark_wallet::{
    AggregateFrostRequest, DefaultSigner, FrostDerivation, FrostJob, FrostShareResult,
    FrostSigningCommitmentsWithNonces, NewLeafKey, OperatorPackage, OperatorRecipient,
    SecretSource, SignFrostRequest, Signer, SparkAddress, TreeNodeId, aggregate_frost,
};

use crate::Network;
use crate::error::SignerError;
use crate::signer::{
    EcdsaSignatureBytes, ExternalFrostCommitments, ExternalFrostJob, ExternalFrostShareResult,
    ExternalFrostSignature, ExternalNewLeafKey, ExternalOperatorPackage, ExternalOperatorRecipient,
    ExternalPrepareClaimRequest, ExternalPrepareLightningReceiveRequest,
    ExternalPrepareStaticDepositClaimRequest, ExternalPrepareStaticDepositRequest,
    ExternalPrepareTokenTransactionRequest, ExternalPrepareTransferRequest, ExternalPreparedClaim,
    ExternalPreparedLightningReceive, ExternalPreparedStaticDeposit,
    ExternalPreparedStaticDepositClaim, ExternalPreparedTokenTransaction, ExternalPreparedTransfer,
    ExternalSignSparkInvoiceRequest, ExternalSignStaticDepositRefundRequest,
    ExternalSignedSparkInvoice, ExternalSparkSigner, ExternalStartStaticDepositRefundRequest,
    ExternalStartedStaticDepositRefund, ExternalTreeNodeId, IdentifierCommitmentPair,
    IdentifierPublicKeyPair, IdentifierSignaturePair, PublicKeyBytes, SchnorrSignatureBytes,
    SecretBytes,
};

use super::accounts::{
    decode_scalar_32, ecdsa_from_rs, schnorr_from_rs, spark_address_format, xpriv_from_secret,
};
use super::error::TurnkeyError;
use super::transport::{OnConflict, TurnkeyClient};
use super::types::{
    ADDRESS_FORMAT_COMPRESSED, HASH_FUNCTION_NO_OP, HASH_FUNCTION_SHA256,
    SPARK_CLAIM_TRANSFER_PATH, SPARK_CLAIM_TRANSFER_RESULT, SPARK_CLAIM_TRANSFER_TYPE,
    SPARK_PREPARE_LIGHTNING_RECEIVE_PATH, SPARK_PREPARE_LIGHTNING_RECEIVE_RESULT,
    SPARK_PREPARE_LIGHTNING_RECEIVE_TYPE, SPARK_PREPARE_TRANSFER_PATH,
    SPARK_PREPARE_TRANSFER_RESULT, SPARK_PREPARE_TRANSFER_TYPE, SPARK_SIGN_FROST_PATH,
    SPARK_SIGN_FROST_RESULT, SPARK_SIGN_FROST_TYPE, SparkClaimLeaf, SparkClaimPackage,
    SparkClaimTransferIntent, SparkClaimTransferResult, SparkEncryptedOperatorPackage,
    SparkFrostCommitment, SparkKeyDerivation, SparkLeafPublicKey, SparkLightningReceivePackage,
    SparkOperatorRecipient, SparkPartialSignature, SparkPrepareLightningReceiveIntent,
    SparkPrepareLightningReceiveResult, SparkPrepareTransferIntent, SparkPrepareTransferResult,
    SparkSignFrostIntent, SparkSignFrostResult, SparkSignatureRequest, SparkTransferLeaf,
    SparkTransferPackage,
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

/// Converts the FFI operator recipients into the native shape consumed by
/// [`operator_recipients`].
fn native_recipients(
    recipients: &[ExternalOperatorRecipient],
) -> Result<Vec<OperatorRecipient>, SignerError> {
    recipients
        .iter()
        .map(|r| r.to_operator_recipient().map_err(to_spark_err))
        .collect()
}

/// Decodes Turnkey's encrypted operator packages into FFI packages.
fn external_operator_packages(
    pkgs: &[SparkEncryptedOperatorPackage],
) -> Result<Vec<ExternalOperatorPackage>, SignerError> {
    pkgs.iter()
        .map(|pkg| {
            let native = operator_package_from(pkg).map_err(to_spark_err)?;
            ExternalOperatorPackage::from_operator_package(&native).map_err(to_spark_err)
        })
        .collect()
}

/// Decodes Turnkey's new-leaf public keys into FFI new-leaf keys (validating
/// each id and key via [`new_leaf_keys_from`]).
fn external_new_leaf_keys(
    leaves: &[SparkLeafPublicKey],
) -> Result<Vec<ExternalNewLeafKey>, SignerError> {
    new_leaf_keys_from(leaves)?
        .into_iter()
        .map(|nlk| {
            Ok(ExternalNewLeafKey {
                node_id: ExternalTreeNodeId::from_tree_node_id(&nlk.node_id)
                    .map_err(to_spark_err)?,
                new_signing_public_key: nlk.new_signing_public_key.serialize().to_vec(),
            })
        })
        .collect()
}

fn ffi_commitment_map(
    pairs: &[IdentifierCommitmentPair],
) -> Result<BTreeMap<Identifier, SigningCommitments>, SignerError> {
    pairs
        .iter()
        .map(|p| {
            Ok((
                p.identifier.to_identifier().map_err(to_spark_err)?,
                p.commitment
                    .to_signing_commitments()
                    .map_err(to_spark_err)?,
            ))
        })
        .collect()
}

fn ffi_signature_map(
    pairs: &[IdentifierSignaturePair],
) -> Result<BTreeMap<Identifier, SignatureShare>, SignerError> {
    pairs
        .iter()
        .map(|p| {
            Ok((
                p.identifier.to_identifier().map_err(to_spark_err)?,
                p.signature.to_signature_share().map_err(to_spark_err)?,
            ))
        })
        .collect()
}

fn ffi_public_key_map(
    pairs: &[IdentifierPublicKeyPair],
) -> Result<BTreeMap<Identifier, PublicKey>, SignerError> {
    pairs
        .iter()
        .map(|p| {
            Ok((
                p.identifier.to_identifier().map_err(to_spark_err)?,
                PublicKey::from_slice(&p.public_key).map_err(to_spark_err)?,
            ))
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
    pub(crate) fn new(client: Arc<TurnkeyClient>, network: Network, account: u32) -> Self {
        Self {
            client,
            account,
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

    /// The wallet identity public key, memoized. Native-typed: internal callers
    /// (identity signing) need the `PublicKey`, not its FFI bytes.
    async fn identity_public_key(&self) -> Result<PublicKey, SignerError> {
        if let Some(pk) = *self.identity_pubkey.lock().await {
            return Ok(pk);
        }
        let pk = self
            .pubkey_at_path(format!("{}/0'", self.base_path()))
            .await?;
        *self.identity_pubkey.lock().await = Some(pk);
        Ok(pk)
    }

    /// The signing public key for a tree leaf, memoized.
    async fn leaf_public_key(&self, leaf_id: &TreeNodeId) -> Result<PublicKey, SignerError> {
        if let Some(pk) = self.leaf_pubkeys.lock().await.get(leaf_id).copied() {
            return Ok(pk);
        }
        let path = format!("{}/1'/{}'", self.base_path(), Self::leaf_index(leaf_id));
        let pk = self.pubkey_at_path(path).await?;
        self.leaf_pubkeys.lock().await.insert(leaf_id.clone(), pk);
        Ok(pk)
    }

    /// The static-deposit signing public key at `index`, memoized.
    async fn static_deposit_public_key(&self, index: u32) -> Result<PublicKey, SignerError> {
        if let Some(pk) = self
            .static_deposit_pubkeys
            .lock()
            .await
            .get(&index)
            .copied()
        {
            return Ok(pk);
        }
        let path = format!("{}/3'/{index}'", self.base_path());
        let pk = self.pubkey_at_path(path).await?;
        self.static_deposit_pubkeys.lock().await.insert(index, pk);
        Ok(pk)
    }

    /// The Spark-format identity address, used as `signWith` for Spark-protocol
    /// activities and BIP-340 Schnorr signing.
    ///
    /// The identity path holds two accounts: a compressed one (created by the
    /// factory, for ECDSA) and the Spark one ensured here. Turnkey allows both
    /// formats at one path, but get-by-path is then ambiguous, so the address is
    /// derived locally: it's the canonical Spark address for the identity key,
    /// identical to what Turnkey assigns the Spark account.
    async fn spark_identity_address(&self) -> Result<String, SignerError> {
        if let Some(addr) = self.spark_address.lock().await.clone() {
            return Ok(addr);
        }
        let path = format!("{}/0'", self.base_path());
        self.client
            .create_account(path, spark_address_format(self.network))
            .await
            .map_err(to_spark_err)?;
        let identity = self.identity_public_key().await?;
        let addr = SparkAddress::new(identity, self.network.into(), None)
            .to_address_string()
            .map_err(to_spark_err)?;
        *self.spark_address.lock().await = Some(addr.clone());
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
        let identity = self.identity_public_key().await?;
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

    /// Native FROST signing over a batch of jobs: the shared path for the trait
    /// `sign_frost` and the in-process `prepare_static_deposit` call.
    async fn sign_frost_native(
        &self,
        jobs: &[FrostJob],
    ) -> Result<Vec<FrostShareResult>, SignerError> {
        let sign_with = self.spark_identity_address().await?;
        let mut signatures = Vec::with_capacity(jobs.len());
        for job in jobs {
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
                OnConflict::Retry,
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

    /// Seeds the leaf cache from a claim/transfer result. Each leaf's signing key
    /// is `HD(leaf_id)`, matching what `get_public_key_for_leaf` derives.
    async fn cache_new_leaf_keys(&self, leaves: &[SparkLeafPublicKey]) -> Result<(), SignerError> {
        let mut cache = self.leaf_pubkeys.lock().await;
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
            .await
            .get(&index)
            .copied()
        {
            return Ok(secret);
        }
        let path = format!("{}/3'/{index}'", self.base_path());
        let secret = self
            .client
            .export_secret_key(path, ADDRESS_FORMAT_COMPRESSED)
            .await
            .map_err(to_spark_err)?;
        self.static_deposit_secret_keys
            .lock()
            .await
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
impl ExternalSparkSigner for TurnkeySparkSigner {
    async fn get_identity_public_key(&self) -> Result<PublicKeyBytes, SignerError> {
        Ok(PublicKeyBytes::from_public_key(
            &self.identity_public_key().await?,
        ))
    }

    async fn get_public_key_for_leaf(
        &self,
        leaf_id: ExternalTreeNodeId,
    ) -> Result<PublicKeyBytes, SignerError> {
        let id = leaf_id.to_tree_node_id().map_err(to_spark_err)?;
        Ok(PublicKeyBytes::from_public_key(
            &self.leaf_public_key(&id).await?,
        ))
    }

    async fn get_static_deposit_public_key(
        &self,
        index: u32,
    ) -> Result<PublicKeyBytes, SignerError> {
        Ok(PublicKeyBytes::from_public_key(
            &self.static_deposit_public_key(index).await?,
        ))
    }

    async fn sign_authentication_challenge(
        &self,
        challenge: Vec<u8>,
    ) -> Result<EcdsaSignatureBytes, SignerError> {
        let sig = self.sign_identity_ecdsa(&challenge).await?;
        Ok(EcdsaSignatureBytes::from_signature(&sig))
    }

    async fn sign_message(&self, message: Vec<u8>) -> Result<EcdsaSignatureBytes, SignerError> {
        let sig = self.sign_identity_ecdsa(&message).await?;
        Ok(EcdsaSignatureBytes::from_signature(&sig))
    }

    async fn sign_frost(
        &self,
        jobs: Vec<ExternalFrostJob>,
    ) -> Result<Vec<ExternalFrostShareResult>, SignerError> {
        let native_jobs = jobs
            .iter()
            .map(|j| j.to_frost_job().map_err(to_spark_err))
            .collect::<Result<Vec<_>, _>>()?;
        let shares = self.sign_frost_native(&native_jobs).await?;
        shares
            .iter()
            .map(|s| ExternalFrostShareResult::from_frost_share_result(s).map_err(to_spark_err))
            .collect()
    }

    async fn prepare_transfer(
        &self,
        request: ExternalPrepareTransferRequest,
    ) -> Result<ExternalPreparedTransfer, SignerError> {
        let sign_with = self.spark_identity_address().await?;
        let leaves = request
            .leaves
            .iter()
            .map(|leaf| {
                let node_id = leaf.node_id.to_tree_node_id().map_err(to_spark_err)?;
                let new_leaf_id = leaf.new_leaf_id.to_tree_node_id().map_err(to_spark_err)?;
                Ok(SparkTransferLeaf {
                    leaf_id: node_id.to_string(),
                    old_leaf_derivation: SparkKeyDerivation::signing_leaf(node_id.to_string()),
                    new_leaf_derivation: SparkKeyDerivation::signing_leaf(new_leaf_id.to_string()),
                    refund_signature: None,
                    direct_refund_signature: None,
                    direct_from_cpfp_refund_signature: None,
                })
            })
            .collect::<Result<Vec<_>, SignerError>>()?;
        let intent = SparkPrepareTransferIntent {
            sign_with,
            transfer: SparkTransferPackage {
                transfer_id: request.transfer_id.clone(),
                leaves,
                threshold: request.threshold,
                operator_recipients: operator_recipients(&native_recipients(
                    &request.operator_recipients,
                )?),
                receiver_public_key: hex::encode(&request.receiver_public_key),
            },
        };
        let result: SparkPrepareTransferResult = self
            .client
            .submit_activity(
                SPARK_PREPARE_TRANSFER_PATH,
                SPARK_PREPARE_TRANSFER_TYPE,
                intent,
                SPARK_PREPARE_TRANSFER_RESULT,
                OnConflict::Retry,
            )
            .await
            .map_err(to_spark_err)?;
        let operator_packages = external_operator_packages(&result.operator_packages)?;
        let new_leaf_keys = external_new_leaf_keys(&result.new_leaf_public_keys)?;
        let der = hex::decode(&result.transfer_user_signature)
            .map_err(|e| TurnkeyError::Deserialize(format!("transferUserSignature: {e}")))
            .map_err(to_spark_err)?;
        let signature = ecdsa::Signature::from_der(&der).map_err(to_spark_err)?;
        Ok(ExternalPreparedTransfer {
            operator_packages,
            new_leaf_keys,
            transfer_user_signature: EcdsaSignatureBytes::from_signature(&signature),
        })
    }

    async fn prepare_claim(
        &self,
        request: ExternalPrepareClaimRequest,
    ) -> Result<ExternalPreparedClaim, SignerError> {
        let sign_with = self.spark_identity_address().await?;
        let leaves = request
            .leaves
            .iter()
            .map(|leaf| {
                let node_id = leaf.node_id.to_tree_node_id().map_err(to_spark_err)?;
                Ok(SparkClaimLeaf {
                    leaf_id: node_id.to_string(),
                    ciphertext: hex::encode(&leaf.leaf_key_ciphertext),
                    sender_signature: hex::encode(&leaf.sender_signature),
                })
            })
            .collect::<Result<Vec<_>, SignerError>>()?;
        let intent = SparkClaimTransferIntent {
            sign_with,
            claim: SparkClaimPackage {
                leaves,
                threshold: request.threshold,
                transfer_id: request.transfer_id.clone(),
                operator_recipients: operator_recipients(&native_recipients(
                    &request.operator_recipients,
                )?),
                sender_identity_public_key: hex::encode(&request.sender_identity_public_key),
            },
        };
        let result: SparkClaimTransferResult = self
            .client
            .submit_activity(
                SPARK_CLAIM_TRANSFER_PATH,
                SPARK_CLAIM_TRANSFER_TYPE,
                intent,
                SPARK_CLAIM_TRANSFER_RESULT,
                OnConflict::Retry,
            )
            .await
            .map_err(to_spark_err)?;
        self.cache_new_leaf_keys(&result.new_leaf_public_keys)
            .await?;
        let operator_packages = external_operator_packages(&result.operator_packages)?;
        Ok(ExternalPreparedClaim { operator_packages })
    }

    async fn prepare_lightning_receive(
        &self,
        request: ExternalPrepareLightningReceiveRequest,
    ) -> Result<ExternalPreparedLightningReceive, SignerError> {
        let sign_with = self.spark_identity_address().await?;
        let intent = SparkPrepareLightningReceiveIntent {
            sign_with,
            lightning_receive: SparkLightningReceivePackage {
                threshold: request.threshold,
                operator_recipients: operator_recipients(&native_recipients(
                    &request.operator_recipients,
                )?),
            },
        };
        let result: SparkPrepareLightningReceiveResult = self
            .client
            .submit_activity(
                SPARK_PREPARE_LIGHTNING_RECEIVE_PATH,
                SPARK_PREPARE_LIGHTNING_RECEIVE_TYPE,
                intent,
                SPARK_PREPARE_LIGHTNING_RECEIVE_RESULT,
                OnConflict::Retry,
            )
            .await
            .map_err(to_spark_err)?;
        let payment_hash = decode_scalar_32(&result.payment_hash)
            .map_err(to_spark_err)?
            .to_vec();
        let operator_preimage_packages = external_operator_packages(&result.operator_packages)?;
        Ok(ExternalPreparedLightningReceive {
            payment_hash,
            operator_preimage_packages,
        })
    }

    async fn prepare_static_deposit(
        &self,
        request: ExternalPrepareStaticDepositRequest,
    ) -> Result<ExternalPreparedStaticDeposit, SignerError> {
        let ExternalPrepareStaticDepositRequest {
            index,
            ssp_public_key,
            frost_jobs,
        } = request;
        let ssp_public_key = PublicKey::from_slice(&ssp_public_key).map_err(to_spark_err)?;
        let native_jobs = frost_jobs
            .iter()
            .map(|j| j.to_frost_job().map_err(to_spark_err))
            .collect::<Result<Vec<_>, _>>()?;
        // Export the static-deposit secret and ECIES-encrypt it to the SSP via a
        // local signer rooted at that key (empty path = the key itself).
        let local = self.local_static_deposit_signer(index).await?;
        let exported_secret = local
            .encrypt_secret_for_receiver(
                &SecretSource::Derived(DerivationPath::master()),
                &ssp_public_key,
            )
            .await
            .map_err(to_spark_err)?;
        // The deposit tree-tx FROST stays in Turnkey (SPARK_SIGN_FROST).
        let frost_shares = self
            .sign_frost_native(&native_jobs)
            .await?
            .iter()
            .map(|s| ExternalFrostShareResult::from_frost_share_result(s).map_err(to_spark_err))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ExternalPreparedStaticDeposit {
            exported_secret,
            frost_shares,
        })
    }

    async fn start_static_deposit_refund(
        &self,
        request: ExternalStartStaticDepositRefundRequest,
    ) -> Result<ExternalStartedStaticDepositRefund, SignerError> {
        let ExternalStartStaticDepositRefundRequest {
            index,
            user_statement,
        } = request;
        let signing_public_key = self.static_deposit_public_key(index).await?;
        // User-commits-first: the nonce (encrypted to the local signer) is
        // generated now and reconstructed in sign_static_deposit_refund.
        let nonce_commitment = self
            .local_static_deposit_signer(index)
            .await?
            .generate_random_signing_commitment()
            .await
            .map_err(to_spark_err)?;
        let user_signature = self.sign_identity_ecdsa(&user_statement).await?;
        Ok(ExternalStartedStaticDepositRefund {
            signing_public_key: signing_public_key.serialize().to_vec(),
            nonce_commitment: ExternalFrostCommitments::from_frost_commitments(&nonce_commitment)
                .map_err(to_spark_err)?,
            user_signature: EcdsaSignatureBytes::from_signature(&user_signature),
        })
    }

    async fn sign_static_deposit_refund(
        &self,
        request: ExternalSignStaticDepositRefundRequest,
    ) -> Result<ExternalFrostSignature, SignerError> {
        let index = request.index;
        let sighash = request.sighash;
        let verifying_key = PublicKey::from_slice(&request.verifying_key).map_err(to_spark_err)?;
        let nonce_commitment = request
            .nonce_commitment
            .to_frost_commitments()
            .map_err(to_spark_err)?;
        let statechain_commitments = ffi_commitment_map(&request.statechain_commitments)?;
        let statechain_signatures = ffi_signature_map(&request.statechain_signatures)?;
        let statechain_public_keys = ffi_public_key_map(&request.statechain_public_keys)?;
        let local = self.local_static_deposit_signer(index).await?;
        let aggregating_public_key = self.static_deposit_public_key(index).await?;
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
            .await
            .map_err(to_spark_err)?;
        let signature = aggregate_frost(AggregateFrostRequest {
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
        .map_err(to_spark_err)?;
        ExternalFrostSignature::from_frost_signature(&signature).map_err(to_spark_err)
    }

    async fn sign_spark_invoice(
        &self,
        request: ExternalSignSparkInvoiceRequest,
    ) -> Result<ExternalSignedSparkInvoice, SignerError> {
        let signature = self.sign_identity_schnorr(&request.invoice_hash).await?;
        Ok(ExternalSignedSparkInvoice {
            signature: SchnorrSignatureBytes::from_signature(&signature),
        })
    }

    async fn prepare_token_transaction(
        &self,
        request: ExternalPrepareTokenTransactionRequest,
    ) -> Result<ExternalPreparedTokenTransaction, SignerError> {
        let signature = self.sign_identity_schnorr(&request.digest).await?;
        Ok(ExternalPreparedTokenTransaction {
            signature: SchnorrSignatureBytes::from_signature(&signature),
        })
    }

    async fn prepare_static_deposit_claim(
        &self,
        request: ExternalPrepareStaticDepositClaimRequest,
    ) -> Result<ExternalPreparedStaticDepositClaim, SignerError> {
        let deposit_secret_key = self.export_static_deposit_key(request.index).await?;
        let user_signature = self.sign_identity_ecdsa(&request.user_statement).await?;
        Ok(ExternalPreparedStaticDepositClaim {
            deposit_secret_key: SecretBytes::from_secret_key(&deposit_secret_key),
            user_signature: EcdsaSignatureBytes::from_signature(&user_signature),
        })
    }
}
