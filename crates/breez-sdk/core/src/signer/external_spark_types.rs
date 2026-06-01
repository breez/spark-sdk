//! FFI-compatible types for the high-level [`ExternalSparkSigner`] trait.
//!
//! These mirror the flow-level `spark_wallet::SparkSigner` request/response
//! types using FFI-safe representations (bytes, strings, vectors of pairs),
//! reusing the primitive conversions from [`super::external_types`].

use bitcoin::secp256k1;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::SdkError;

use super::external_types::{
    ExternalFrostCommitments, ExternalFrostSignatureShare, ExternalIdentifier,
    ExternalSigningCommitments, ExternalTreeNodeId, IdentifierCommitmentPair,
};

/// FFI-safe representation of `spark_wallet::FrostDerivation`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ExternalFrostDerivation {
    /// The signing key for a tree leaf.
    SigningLeaf { leaf_id: ExternalTreeNodeId },
    /// The static-deposit key at `index`.
    StaticDeposit { index: u32 },
    /// The HTLC-preimage key.
    HtlcPreimage,
    /// The wallet identity key.
    Identity,
}

impl ExternalFrostDerivation {
    pub fn from_derivation(d: &spark_wallet::FrostDerivation) -> Result<Self, SdkError> {
        Ok(match d {
            spark_wallet::FrostDerivation::SigningLeaf { leaf_id } => Self::SigningLeaf {
                leaf_id: ExternalTreeNodeId::from_tree_node_id(leaf_id)?,
            },
            spark_wallet::FrostDerivation::StaticDeposit { index } => {
                Self::StaticDeposit { index: *index }
            }
            spark_wallet::FrostDerivation::HtlcPreimage => Self::HtlcPreimage,
            spark_wallet::FrostDerivation::Identity => Self::Identity,
        })
    }

    pub fn to_derivation(&self) -> Result<spark_wallet::FrostDerivation, SdkError> {
        Ok(match self {
            Self::SigningLeaf { leaf_id } => spark_wallet::FrostDerivation::SigningLeaf {
                leaf_id: leaf_id.to_tree_node_id()?,
            },
            Self::StaticDeposit { index } => {
                spark_wallet::FrostDerivation::StaticDeposit { index: *index }
            }
            Self::HtlcPreimage => spark_wallet::FrostDerivation::HtlcPreimage,
            Self::Identity => spark_wallet::FrostDerivation::Identity,
        })
    }
}

/// FFI-safe representation of `spark_wallet::FrostJob`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalFrostJob {
    /// Which key to sign with.
    pub derivation: ExternalFrostDerivation,
    /// 32-byte BIP-341 sighash to sign.
    pub sighash: Vec<u8>,
    /// FROST group verifying key (33 bytes compressed).
    pub verifying_key: Vec<u8>,
    /// Per-operator round-1 commitments.
    pub operator_commitments: Vec<IdentifierCommitmentPair>,
    /// Optional adaptor public key (33 bytes compressed).
    pub adaptor_public_key: Option<Vec<u8>>,
}

impl ExternalFrostJob {
    pub fn from_frost_job(job: &spark_wallet::FrostJob) -> Result<Self, SdkError> {
        let operator_commitments = job
            .operator_commitments
            .iter()
            .map(|(id, comm)| {
                Ok(IdentifierCommitmentPair {
                    identifier: ExternalIdentifier::from_identifier(id),
                    commitment: ExternalSigningCommitments::from_signing_commitments(comm)?,
                })
            })
            .collect::<Result<Vec<_>, SdkError>>()?;
        Ok(Self {
            derivation: ExternalFrostDerivation::from_derivation(&job.derivation)?,
            sighash: job.sighash.to_vec(),
            verifying_key: job.verifying_key.serialize().to_vec(),
            operator_commitments,
            adaptor_public_key: job.adaptor_public_key.map(|pk| pk.serialize().to_vec()),
        })
    }

    pub fn to_frost_job(&self) -> Result<spark_wallet::FrostJob, SdkError> {
        let sighash: [u8; 32] = self.sighash[..]
            .try_into()
            .map_err(|_| SdkError::Generic("FROST sighash must be 32 bytes".to_string()))?;
        let verifying_key = secp256k1::PublicKey::from_slice(&self.verifying_key)
            .map_err(|e| SdkError::Generic(format!("Invalid verifying key: {e}")))?;
        let operator_commitments: BTreeMap<_, _> = self
            .operator_commitments
            .iter()
            .map(|p| {
                Ok((
                    p.identifier.to_identifier()?,
                    p.commitment.to_signing_commitments()?,
                ))
            })
            .collect::<Result<_, SdkError>>()?;
        let adaptor_public_key = self
            .adaptor_public_key
            .as_ref()
            .map(|b| {
                secp256k1::PublicKey::from_slice(b)
                    .map_err(|e| SdkError::Generic(format!("Invalid adaptor public key: {e}")))
            })
            .transpose()?;
        Ok(spark_wallet::FrostJob {
            derivation: self.derivation.to_derivation()?,
            sighash,
            verifying_key,
            operator_commitments,
            adaptor_public_key,
        })
    }
}

/// FFI-safe representation of `spark_wallet::FrostShareResult`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalFrostShareResult {
    /// The user's nonce commitment (round-1 output).
    pub commitment: ExternalFrostCommitments,
    /// The user's signature share (round-2 output).
    pub signature_share: ExternalFrostSignatureShare,
}

impl ExternalFrostShareResult {
    pub fn from_frost_share_result(
        result: &spark_wallet::FrostShareResult,
    ) -> Result<Self, SdkError> {
        Ok(Self {
            commitment: ExternalFrostCommitments::from_frost_commitments(&result.commitment)?,
            signature_share: ExternalFrostSignatureShare::from_signature_share(
                &result.signature_share,
            )?,
        })
    }

    pub fn to_frost_share_result(&self) -> Result<spark_wallet::FrostShareResult, SdkError> {
        Ok(spark_wallet::FrostShareResult {
            commitment: self.commitment.to_frost_commitments()?,
            signature_share: self.signature_share.to_signature_share()?,
        })
    }
}

/// FFI-safe representation of `spark_wallet::OperatorRecipient`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalOperatorRecipient {
    /// Numeric operator id (determines the Feldman share index).
    pub id: u64,
    /// FROST identifier.
    pub identifier: ExternalIdentifier,
    /// The operator's ECIES / identity public key (33 bytes compressed).
    pub public_key: Vec<u8>,
}

impl ExternalOperatorRecipient {
    pub fn from_operator_recipient(
        recipient: &spark_wallet::OperatorRecipient,
    ) -> Result<Self, SdkError> {
        Ok(Self {
            id: recipient.id as u64,
            identifier: ExternalIdentifier::from_identifier(&recipient.identifier),
            public_key: recipient.public_key.serialize().to_vec(),
        })
    }

    pub fn to_operator_recipient(&self) -> Result<spark_wallet::OperatorRecipient, SdkError> {
        Ok(spark_wallet::OperatorRecipient {
            id: self.id as usize,
            identifier: self.identifier.to_identifier()?,
            public_key: secp256k1::PublicKey::from_slice(&self.public_key)
                .map_err(|e| SdkError::Generic(format!("Invalid operator public key: {e}")))?,
        })
    }
}

/// FFI-safe representation of `spark_wallet::OperatorPackage`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ExternalOperatorPackage {
    /// The operator this package is encrypted for.
    pub operator_identifier: ExternalIdentifier,
    /// The ECIES-encrypted package bytes.
    pub encrypted_package: Vec<u8>,
}

impl ExternalOperatorPackage {
    pub fn from_operator_package(pkg: &spark_wallet::OperatorPackage) -> Result<Self, SdkError> {
        Ok(Self {
            operator_identifier: ExternalIdentifier::from_identifier(&pkg.operator_identifier),
            encrypted_package: pkg.encrypted_package.clone(),
        })
    }

    pub fn to_operator_package(&self) -> Result<spark_wallet::OperatorPackage, SdkError> {
        Ok(spark_wallet::OperatorPackage {
            operator_identifier: self.operator_identifier.to_identifier()?,
            encrypted_package: self.encrypted_package.clone(),
        })
    }
}
