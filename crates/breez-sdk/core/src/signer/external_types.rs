//! FFI-compatible types for the `ExternalSigner` trait
//!
//! These types are designed to be simpler and FFI-safe, using basic types like
//! Vec<u8> and String instead of complex Rust types.
use bitcoin::bip32::DerivationPath;
use bitcoin::secp256k1;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

use crate::SdkError;

/// FFI-safe representation of a secp256k1 public key (33 bytes compressed)
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PublicKeyBytes {
    pub bytes: Vec<u8>,
}

impl PublicKeyBytes {
    pub fn from_public_key(pk: &secp256k1::PublicKey) -> Self {
        Self {
            bytes: pk.serialize().to_vec(),
        }
    }

    pub fn to_public_key(&self) -> Result<secp256k1::PublicKey, SdkError> {
        secp256k1::PublicKey::from_slice(&self.bytes)
            .map_err(|e| SdkError::Generic(format!("Invalid public key bytes: {e}")))
    }
}

/// FFI-safe representation of an ECDSA signature (64 bytes)
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct EcdsaSignatureBytes {
    pub bytes: Vec<u8>,
}

impl EcdsaSignatureBytes {
    pub fn from_signature(sig: &secp256k1::ecdsa::Signature) -> Self {
        Self {
            bytes: sig.serialize_compact().to_vec(),
        }
    }

    pub fn to_signature(&self) -> Result<secp256k1::ecdsa::Signature, SdkError> {
        secp256k1::ecdsa::Signature::from_compact(&self.bytes)
            .map_err(|e| SdkError::Generic(format!("Invalid ECDSA signature bytes: {e}")))
    }
}

/// FFI-safe representation of a Schnorr signature (64 bytes)
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SchnorrSignatureBytes {
    pub bytes: Vec<u8>,
}

impl SchnorrSignatureBytes {
    pub fn from_signature(sig: &secp256k1::schnorr::Signature) -> Self {
        Self {
            bytes: sig.as_ref().to_vec(),
        }
    }

    pub fn to_signature(&self) -> Result<secp256k1::schnorr::Signature, SdkError> {
        secp256k1::schnorr::Signature::from_slice(&self.bytes)
            .map_err(|e| SdkError::Generic(format!("Invalid Schnorr signature bytes: {e}")))
    }
}

/// FFI-safe representation of a recoverable ECDSA signature (65 bytes: 1 recovery byte + 64 signature bytes)
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RecoverableEcdsaSignatureBytes {
    pub bytes: Vec<u8>,
}

impl RecoverableEcdsaSignatureBytes {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }
}

/// FFI-safe representation of a private key (32 bytes)
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PrivateKeyBytes {
    pub bytes: Vec<u8>,
}

impl PrivateKeyBytes {
    pub fn from_secret_key(sk: &secp256k1::SecretKey) -> Self {
        Self {
            bytes: sk.secret_bytes().to_vec(),
        }
    }

    pub fn to_secret_key(&self) -> Result<secp256k1::SecretKey, SdkError> {
        secp256k1::SecretKey::from_slice(&self.bytes)
            .map_err(|e| SdkError::Generic(format!("Invalid private key bytes: {e}")))
    }
}

/// Helper functions for `DerivationPath` string conversion
pub fn derivation_path_to_string(path: &DerivationPath) -> String {
    path.to_string()
}

pub fn string_to_derivation_path(s: &str) -> Result<DerivationPath, SdkError> {
    DerivationPath::from_str(s)
        .map_err(|e| SdkError::Generic(format!("Invalid derivation path '{s}': {e}")))
}

/// FFI-safe representation of `spark_wallet::TreeNodeId`
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalTreeNodeId {
    /// The tree node identifier as a string
    pub id: String,
}

impl ExternalTreeNodeId {
    pub fn from_tree_node_id(id: &spark_wallet::TreeNodeId) -> Result<Self, SdkError> {
        Ok(Self { id: id.to_string() })
    }

    pub fn to_tree_node_id(&self) -> Result<spark_wallet::TreeNodeId, SdkError> {
        spark_wallet::TreeNodeId::from_str(&self.id)
            .map_err(|e| SdkError::Generic(format!("Invalid TreeNodeId: {e}")))
    }
}

/// FFI-safe representation of `spark_wallet::EncryptedPrivateKey`
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalEncryptedPrivateKey {
    /// The encrypted ciphertext
    pub ciphertext: Vec<u8>,
}

impl ExternalEncryptedPrivateKey {
    pub fn from_encrypted_private_key(
        key: &spark_wallet::EncryptedPrivateKey,
    ) -> Result<Self, SdkError> {
        Ok(Self {
            ciphertext: key.as_slice().to_vec(),
        })
    }

    pub fn to_encrypted_private_key(&self) -> Result<spark_wallet::EncryptedPrivateKey, SdkError> {
        Ok(spark_wallet::EncryptedPrivateKey::new(
            self.ciphertext.clone(),
        ))
    }
}

/// FFI-safe representation of `spark_wallet::PrivateKeySource`
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ExternalPrivateKeySource {
    /// Private key derived from a tree node
    Derived { node_id: ExternalTreeNodeId },
    /// Encrypted private key
    Encrypted { key: ExternalEncryptedPrivateKey },
}

impl ExternalPrivateKeySource {
    pub fn from_private_key_source(
        source: &spark_wallet::PrivateKeySource,
    ) -> Result<Self, SdkError> {
        match source {
            spark_wallet::PrivateKeySource::Derived(node_id) => Ok(Self::Derived {
                node_id: ExternalTreeNodeId::from_tree_node_id(node_id)?,
            }),
            spark_wallet::PrivateKeySource::Encrypted(key) => Ok(Self::Encrypted {
                key: ExternalEncryptedPrivateKey::from_encrypted_private_key(key)?,
            }),
        }
    }

    pub fn to_private_key_source(&self) -> Result<spark_wallet::PrivateKeySource, SdkError> {
        match self {
            Self::Derived { node_id } => Ok(spark_wallet::PrivateKeySource::Derived(
                node_id.to_tree_node_id()?,
            )),
            Self::Encrypted { key } => Ok(spark_wallet::PrivateKeySource::Encrypted(
                key.to_encrypted_private_key()?,
            )),
        }
    }
}

/// FFI-safe representation of `spark_wallet::SecretToSplit`
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ExternalSecretToSplit {
    /// A private key to split
    PrivateKey { source: ExternalPrivateKeySource },
    /// A preimage to split (32 bytes)
    Preimage { data: Vec<u8> },
}

impl ExternalSecretToSplit {
    pub fn from_secret_to_split(secret: &spark_wallet::SecretToSplit) -> Result<Self, SdkError> {
        match secret {
            spark_wallet::SecretToSplit::PrivateKey(source) => Ok(Self::PrivateKey {
                source: ExternalPrivateKeySource::from_private_key_source(source)?,
            }),
            spark_wallet::SecretToSplit::Preimage(data) => {
                Ok(Self::Preimage { data: data.clone() })
            }
        }
    }

    pub fn to_secret_to_split(&self) -> Result<spark_wallet::SecretToSplit, SdkError> {
        match self {
            Self::PrivateKey { source } => Ok(spark_wallet::SecretToSplit::PrivateKey(
                source.to_private_key_source()?,
            )),
            Self::Preimage { data } => Ok(spark_wallet::SecretToSplit::Preimage(data.clone())),
        }
    }
}

/// FFI-safe representation of `k256::Scalar` (32 bytes)
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalScalar {
    /// The 32-byte scalar value
    pub bytes: Vec<u8>,
}

/// FFI-safe representation of `spark_wallet::SecretShare`
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalSecretShare {
    /// Number of shares required to recover the secret
    pub threshold: u32,
    /// Index (x-coordinate) of the share as 32 bytes
    pub index: ExternalScalar,
    /// Share value (y-coordinate) as 32 bytes
    pub share: ExternalScalar,
}

/// FFI-safe representation of `spark_wallet::VerifiableSecretShare`
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalVerifiableSecretShare {
    /// Base secret share containing threshold, index, and share value
    pub secret_share: ExternalSecretShare,
    /// Cryptographic proofs for share verification (each proof is 33 bytes compressed public key)
    pub proofs: Vec<Vec<u8>>,
}

impl ExternalVerifiableSecretShare {
    pub fn from_verifiable_secret_share(
        share: &spark_wallet::VerifiableSecretShare,
    ) -> Result<Self, SdkError> {
        use k256::elliptic_curve::sec1::ToEncodedPoint;

        let secret_share = ExternalSecretShare {
            threshold: share
                .secret_share
                .threshold
                .try_into()
                .map_err(|_| SdkError::Generic("Threshold value too large".to_string()))?,
            index: ExternalScalar {
                bytes: share.secret_share.index.to_bytes().to_vec(),
            },
            share: ExternalScalar {
                bytes: share.secret_share.share.to_bytes().to_vec(),
            },
        };

        let proofs = share
            .proofs
            .iter()
            .map(|pk| pk.to_encoded_point(true).as_bytes().to_vec())
            .collect();

        Ok(Self {
            secret_share,
            proofs,
        })
    }

    pub fn to_verifiable_secret_share(
        &self,
    ) -> Result<spark_wallet::VerifiableSecretShare, SdkError> {
        use k256::elliptic_curve::PrimeField;
        use k256::{FieldBytes, PublicKey as k256PublicKey, Scalar};

        let index_bytes: [u8; 32] = self.secret_share.index.bytes[..]
            .try_into()
            .map_err(|_| SdkError::Generic("Invalid index scalar length".into()))?;
        let index = Scalar::from_repr(FieldBytes::clone_from_slice(&index_bytes))
            .into_option()
            .ok_or_else(|| SdkError::Generic("Invalid index scalar".into()))?;

        let share_bytes: [u8; 32] = self.secret_share.share.bytes[..]
            .try_into()
            .map_err(|_| SdkError::Generic("Invalid share scalar length".into()))?;
        let share = Scalar::from_repr(FieldBytes::clone_from_slice(&share_bytes))
            .into_option()
            .ok_or_else(|| SdkError::Generic("Invalid share scalar".into()))?;

        let proofs: Vec<k256PublicKey> = self
            .proofs
            .iter()
            .map(|bytes| {
                k256PublicKey::from_sec1_bytes(bytes)
                    .map_err(|e| SdkError::Generic(format!("Invalid proof public key: {e}")))
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(spark_wallet::VerifiableSecretShare {
            secret_share: spark_wallet::SecretShare {
                threshold: self.secret_share.threshold as usize,
                index,
                share,
            },
            proofs,
        })
    }
}

/// FFI-safe representation of `frost_secp256k1_tr::round2::SignatureShare`
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalFrostSignatureShare {
    /// Serialized signature share bytes (variable length, typically 32 bytes)
    pub bytes: Vec<u8>,
}

impl ExternalFrostSignatureShare {
    pub fn from_signature_share(
        share: &frost_secp256k1_tr::round2::SignatureShare,
    ) -> Result<Self, SdkError> {
        let bytes = share.serialize();
        Ok(Self { bytes })
    }

    pub fn to_signature_share(
        &self,
    ) -> Result<frost_secp256k1_tr::round2::SignatureShare, SdkError> {
        frost_secp256k1_tr::round2::SignatureShare::deserialize(&self.bytes)
            .map_err(|e| SdkError::Generic(format!("Failed to deserialize SignatureShare: {e}")))
    }
}

/// FFI-safe representation of `frost_secp256k1_tr::Signature`
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalFrostSignature {
    /// Serialized Frost signature bytes (64 bytes)
    pub bytes: Vec<u8>,
}

impl ExternalFrostSignature {
    pub fn from_frost_signature(sig: &frost_secp256k1_tr::Signature) -> Result<Self, SdkError> {
        let bytes = sig
            .serialize()
            .map_err(|e| SdkError::Generic(format!("Failed to serialize Frost signature: {e}")))?;
        let bytes = bytes.clone();
        Ok(Self { bytes })
    }

    pub fn to_frost_signature(&self) -> Result<frost_secp256k1_tr::Signature, SdkError> {
        frost_secp256k1_tr::Signature::deserialize(&self.bytes)
            .map_err(|e| SdkError::Generic(format!("Failed to deserialize Frost signature: {e}")))
    }
}

/// FFI-safe representation of `spark_wallet::FrostSigningCommitmentsWithNonces`
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalFrostCommitments {
    /// Serialized hiding nonce commitment (variable length, typically 33 bytes compressed point)
    pub hiding_commitment: Vec<u8>,
    /// Serialized binding nonce commitment (variable length, typically 33 bytes compressed point)
    pub binding_commitment: Vec<u8>,
    /// Encrypted nonces ciphertext
    pub nonces_ciphertext: Vec<u8>,
}

impl ExternalFrostCommitments {
    pub fn from_frost_commitments(
        commitments: &spark_wallet::FrostSigningCommitmentsWithNonces,
    ) -> Result<Self, SdkError> {
        let hiding_commitment = commitments.commitments.hiding().serialize().map_err(|e| {
            SdkError::Generic(format!("Failed to serialize hiding commitment: {e}"))
        })?;
        let binding_commitment = commitments.commitments.binding().serialize().map_err(|e| {
            SdkError::Generic(format!("Failed to serialize binding commitment: {e}"))
        })?;

        Ok(Self {
            hiding_commitment,
            binding_commitment,
            nonces_ciphertext: commitments.nonces_ciphertext.clone(),
        })
    }

    pub fn to_frost_commitments(
        &self,
    ) -> Result<spark_wallet::FrostSigningCommitmentsWithNonces, SdkError> {
        use frost_secp256k1_tr::round1::{NonceCommitment, SigningCommitments};

        let hiding = NonceCommitment::deserialize(&self.hiding_commitment).map_err(|e| {
            SdkError::Generic(format!("Failed to deserialize hiding commitment: {e}"))
        })?;
        let binding = NonceCommitment::deserialize(&self.binding_commitment).map_err(|e| {
            SdkError::Generic(format!("Failed to deserialize binding commitment: {e}"))
        })?;

        let commitments = SigningCommitments::new(hiding, binding);

        Ok(spark_wallet::FrostSigningCommitmentsWithNonces {
            commitments,
            nonces_ciphertext: self.nonces_ciphertext.clone(),
        })
    }
}

/// FFI-safe representation of `frost_secp256k1_tr::Identifier`
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalIdentifier {
    /// Serialized identifier bytes
    pub bytes: Vec<u8>,
}

impl ExternalIdentifier {
    pub fn from_identifier(id: &frost_secp256k1_tr::Identifier) -> Self {
        Self {
            bytes: id.serialize(),
        }
    }

    pub fn to_identifier(&self) -> Result<frost_secp256k1_tr::Identifier, SdkError> {
        frost_secp256k1_tr::Identifier::deserialize(&self.bytes)
            .map_err(|e| SdkError::Generic(format!("Invalid identifier: {e}")))
    }
}

/// FFI-safe representation of `frost_secp256k1_tr::round1::SigningCommitments`
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalSigningCommitments {
    /// Serialized hiding nonce commitment
    pub hiding: Vec<u8>,
    /// Serialized binding nonce commitment
    pub binding: Vec<u8>,
}

impl ExternalSigningCommitments {
    pub fn from_signing_commitments(
        commitments: &frost_secp256k1_tr::round1::SigningCommitments,
    ) -> Result<Self, SdkError> {
        let hiding = commitments.hiding().serialize().map_err(|e| {
            SdkError::Generic(format!("Failed to serialize hiding commitment: {e}"))
        })?;
        let binding = commitments.binding().serialize().map_err(|e| {
            SdkError::Generic(format!("Failed to serialize binding commitment: {e}"))
        })?;
        Ok(Self { hiding, binding })
    }

    pub fn to_signing_commitments(
        &self,
    ) -> Result<frost_secp256k1_tr::round1::SigningCommitments, SdkError> {
        use frost_secp256k1_tr::round1::NonceCommitment;

        let hiding = NonceCommitment::deserialize(&self.hiding)
            .map_err(|e| SdkError::Generic(format!("Failed to deserialize hiding: {e}")))?;
        let binding = NonceCommitment::deserialize(&self.binding)
            .map_err(|e| SdkError::Generic(format!("Failed to deserialize binding: {e}")))?;

        Ok(frost_secp256k1_tr::round1::SigningCommitments::new(
            hiding, binding,
        ))
    }
}

/// FFI-safe wrapper for (Identifier, `SigningCommitments`) pair
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct IdentifierCommitmentPair {
    pub identifier: ExternalIdentifier,
    pub commitment: ExternalSigningCommitments,
}

/// FFI-safe wrapper for (Identifier, `SignatureShare`) pair
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct IdentifierSignaturePair {
    pub identifier: ExternalIdentifier,
    pub signature: ExternalFrostSignatureShare,
}

/// FFI-safe wrapper for (Identifier, `PublicKey`) pair
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct IdentifierPublicKeyPair {
    pub identifier: ExternalIdentifier,
    pub public_key: Vec<u8>,
}

/// FFI-safe representation of `spark_wallet::SignFrostRequest`
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalSignFrostRequest {
    /// The message to sign
    pub message: Vec<u8>,
    /// The public key (33 bytes compressed)
    pub public_key: Vec<u8>,
    /// The private key source
    pub private_key: ExternalPrivateKeySource,
    /// The verifying key (33 bytes compressed)
    pub verifying_key: Vec<u8>,
    /// The self nonce commitment
    pub self_nonce_commitment: ExternalFrostCommitments,
    /// Statechain commitments as a list of identifier-commitment pairs
    pub statechain_commitments: Vec<IdentifierCommitmentPair>,
    /// Optional adaptor public key (33 bytes compressed)
    pub adaptor_public_key: Option<Vec<u8>>,
}

impl ExternalSignFrostRequest {
    pub fn from_sign_frost_request(
        request: &spark_wallet::SignFrostRequest,
    ) -> Result<Self, SdkError> {
        let statechain_commitments = request
            .statechain_commitments
            .iter()
            .map(|(id, comm)| {
                Ok(IdentifierCommitmentPair {
                    identifier: ExternalIdentifier::from_identifier(id),
                    commitment: ExternalSigningCommitments::from_signing_commitments(comm)?,
                })
            })
            .collect::<Result<Vec<_>, SdkError>>()?;

        Ok(Self {
            message: request.message.to_vec(),
            public_key: request.public_key.serialize().to_vec(),
            private_key: ExternalPrivateKeySource::from_private_key_source(request.private_key)?,
            verifying_key: request.verifying_key.serialize().to_vec(),
            self_nonce_commitment: ExternalFrostCommitments::from_frost_commitments(
                request.self_nonce_commitment,
            )?,
            statechain_commitments,
            adaptor_public_key: request.adaptor_public_key.map(|pk| pk.serialize().to_vec()),
        })
    }

    pub fn to_sign_frost_request(
        &self,
    ) -> Result<spark_wallet::SignFrostRequest<'static>, SdkError> {
        use std::collections::BTreeMap;

        let public_key = secp256k1::PublicKey::from_slice(&self.public_key)
            .map_err(|e| SdkError::Generic(format!("Invalid public key: {e}")))?;
        let verifying_key = secp256k1::PublicKey::from_slice(&self.verifying_key)
            .map_err(|e| SdkError::Generic(format!("Invalid verifying key: {e}")))?;

        let statechain_commitments: BTreeMap<_, _> = self
            .statechain_commitments
            .iter()
            .map(|pair| {
                Ok((
                    pair.identifier.to_identifier()?,
                    pair.commitment.to_signing_commitments()?,
                ))
            })
            .collect::<Result<_, SdkError>>()?;

        let adaptor_public_key = self
            .adaptor_public_key
            .as_ref()
            .map(|bytes| {
                secp256k1::PublicKey::from_slice(bytes)
                    .map_err(|e| SdkError::Generic(format!("Invalid adaptor public key: {e}")))
            })
            .transpose()?;

        // Note: This creates a static lifetime version with owned data
        // The actual usage will need to handle the conversion appropriately
        Ok(spark_wallet::SignFrostRequest {
            message: Box::leak(self.message.clone().into_boxed_slice()),
            public_key: Box::leak(Box::new(public_key)),
            private_key: Box::leak(Box::new(self.private_key.to_private_key_source()?)),
            verifying_key: Box::leak(Box::new(verifying_key)),
            self_nonce_commitment: Box::leak(Box::new(
                self.self_nonce_commitment.to_frost_commitments()?,
            )),
            statechain_commitments,
            adaptor_public_key: adaptor_public_key.map(|pk| Box::leak(Box::new(pk)) as &_),
        })
    }
}

/// FFI-safe representation of `spark_wallet::AggregateFrostRequest`
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalAggregateFrostRequest {
    /// The message that was signed
    pub message: Vec<u8>,
    /// Statechain signatures as a list of identifier-signature pairs
    pub statechain_signatures: Vec<IdentifierSignaturePair>,
    /// Statechain public keys as a list of identifier-publickey pairs
    pub statechain_public_keys: Vec<IdentifierPublicKeyPair>,
    /// The verifying key (33 bytes compressed)
    pub verifying_key: Vec<u8>,
    /// Statechain commitments as a list of identifier-commitment pairs
    pub statechain_commitments: Vec<IdentifierCommitmentPair>,
    /// The self commitment
    pub self_commitment: ExternalSigningCommitments,
    /// The public key (33 bytes compressed)
    pub public_key: Vec<u8>,
    /// The self signature share
    pub self_signature: ExternalFrostSignatureShare,
    /// Optional adaptor public key (33 bytes compressed)
    pub adaptor_public_key: Option<Vec<u8>>,
}

impl ExternalAggregateFrostRequest {
    pub fn from_aggregate_frost_request(
        request: &spark_wallet::AggregateFrostRequest,
    ) -> Result<Self, SdkError> {
        let statechain_signatures = request
            .statechain_signatures
            .iter()
            .map(|(id, share)| {
                Ok(IdentifierSignaturePair {
                    identifier: ExternalIdentifier::from_identifier(id),
                    signature: ExternalFrostSignatureShare::from_signature_share(share)?,
                })
            })
            .collect::<Result<Vec<_>, SdkError>>()?;

        let statechain_public_keys = request
            .statechain_public_keys
            .iter()
            .map(|(id, pk)| IdentifierPublicKeyPair {
                identifier: ExternalIdentifier::from_identifier(id),
                public_key: pk.serialize().to_vec(),
            })
            .collect();

        let statechain_commitments = request
            .statechain_commitments
            .iter()
            .map(|(id, comm)| {
                Ok(IdentifierCommitmentPair {
                    identifier: ExternalIdentifier::from_identifier(id),
                    commitment: ExternalSigningCommitments::from_signing_commitments(comm)?,
                })
            })
            .collect::<Result<Vec<_>, SdkError>>()?;

        Ok(Self {
            message: request.message.to_vec(),
            statechain_signatures,
            statechain_public_keys,
            verifying_key: request.verifying_key.serialize().to_vec(),
            statechain_commitments,
            self_commitment: ExternalSigningCommitments::from_signing_commitments(
                request.self_commitment,
            )?,
            public_key: request.public_key.serialize().to_vec(),
            self_signature: ExternalFrostSignatureShare::from_signature_share(
                request.self_signature,
            )?,
            adaptor_public_key: request.adaptor_public_key.map(|pk| pk.serialize().to_vec()),
        })
    }

    pub fn to_aggregate_frost_request(
        &self,
    ) -> Result<spark_wallet::AggregateFrostRequest<'static>, SdkError> {
        use std::collections::BTreeMap;

        let statechain_signatures: BTreeMap<_, _> = self
            .statechain_signatures
            .iter()
            .map(|pair| {
                Ok((
                    pair.identifier.to_identifier()?,
                    pair.signature.to_signature_share()?,
                ))
            })
            .collect::<Result<_, SdkError>>()?;

        let statechain_public_keys: BTreeMap<_, _> = self
            .statechain_public_keys
            .iter()
            .map(|pair| {
                Ok((
                    pair.identifier.to_identifier()?,
                    secp256k1::PublicKey::from_slice(&pair.public_key)
                        .map_err(|e| SdkError::Generic(format!("Invalid public key: {e}")))?,
                ))
            })
            .collect::<Result<_, SdkError>>()?;

        let verifying_key = secp256k1::PublicKey::from_slice(&self.verifying_key)
            .map_err(|e| SdkError::Generic(format!("Invalid verifying key: {e}")))?;

        let statechain_commitments: BTreeMap<_, _> = self
            .statechain_commitments
            .iter()
            .map(|pair| {
                Ok((
                    pair.identifier.to_identifier()?,
                    pair.commitment.to_signing_commitments()?,
                ))
            })
            .collect::<Result<_, SdkError>>()?;

        let public_key = secp256k1::PublicKey::from_slice(&self.public_key)
            .map_err(|e| SdkError::Generic(format!("Invalid public key: {e}")))?;

        let adaptor_public_key = self
            .adaptor_public_key
            .as_ref()
            .map(|bytes| {
                secp256k1::PublicKey::from_slice(bytes)
                    .map_err(|e| SdkError::Generic(format!("Invalid adaptor public key: {e}")))
            })
            .transpose()?;

        Ok(spark_wallet::AggregateFrostRequest {
            message: Box::leak(self.message.clone().into_boxed_slice()),
            statechain_signatures,
            statechain_public_keys,
            verifying_key: Box::leak(Box::new(verifying_key)),
            statechain_commitments,
            self_commitment: Box::leak(Box::new(self.self_commitment.to_signing_commitments()?)),
            public_key: Box::leak(Box::new(public_key)),
            self_signature: Box::leak(Box::new(self.self_signature.to_signature_share()?)),
            adaptor_public_key: adaptor_public_key.map(|pk| Box::leak(Box::new(pk)) as &_),
        })
    }
}
