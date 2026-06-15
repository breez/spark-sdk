//! FFI-compatible types for the `ExternalBreezSigner` trait
//!
//! These types are designed to be simpler and FFI-safe, using basic types like
//! Vec<u8> and String instead of complex Rust types.
use bitcoin::bip32::DerivationPath;
use bitcoin::hashes::{Hash, Hmac, sha256};
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct HashedMessageBytes {
    pub bytes: Vec<u8>,
}

impl HashedMessageBytes {
    pub fn from_hmac(hmac: &Hmac<sha256::Hash>) -> Self {
        Self {
            bytes: hmac.to_byte_array().to_vec(),
        }
    }

    pub fn to_hmac(&self) -> Result<Hmac<sha256::Hash>, SdkError> {
        Hmac::<sha256::Hash>::from_slice(&self.bytes)
            .map_err(|e| SdkError::Generic(format!("Invalid HMAC bytes: {e}")))
    }
}

/// FFI-safe representation of a 32-byte message digest for ECDSA signing
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MessageBytes {
    pub bytes: Vec<u8>,
}

impl MessageBytes {
    /// Create `MessageBytes` from a 32-byte digest
    pub fn new(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    /// Convert to 32-byte array for `secp256k1::Message`
    pub fn to_digest(&self) -> Result<[u8; 32], SdkError> {
        self.bytes
            .clone()
            .try_into()
            .map_err(|_| SdkError::Generic("Message digest must be 32 bytes".to_string()))
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
#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SecretBytes {
    pub bytes: Vec<u8>,
}

/// Redacted `Debug`: never print the raw key, so it can't leak into logs even
/// when wrapped in a `Debug`-deriving container. Mirrors `secp256k1::SecretKey`.
impl std::fmt::Debug for SecretBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("SecretBytes").field(&"<redacted>").finish()
    }
}

impl SecretBytes {
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
