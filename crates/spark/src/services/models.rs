use std::collections::{BTreeMap, HashMap};

use bitcoin::{consensus::Encodable, secp256k1::PublicKey};

use frost_secp256k1_tr::{
    Identifier,
    round1::{NonceCommitment, SigningCommitments},
    round2::SignatureShare,
};

use crate::utils::refund::SignedTx;

use super::ServiceError;
use crate::operator::rpc as operator_rpc;

impl From<crate::Network> for operator_rpc::spark::Network {
    fn from(network: crate::Network) -> Self {
        match network {
            crate::Network::Mainnet => operator_rpc::spark::Network::Mainnet,
            crate::Network::Regtest => operator_rpc::spark::Network::Regtest,
            crate::Network::Testnet => operator_rpc::spark::Network::Testnet,
            crate::Network::Signet => operator_rpc::spark::Network::Signet,
        }
    }
}

pub(crate) fn to_proto_signing_commitments(
    signing_commitments: &BTreeMap<Identifier, SigningCommitments>,
) -> Result<HashMap<String, operator_rpc::common::SigningCommitment>, ServiceError> {
    let mut proto_signing_commitments = HashMap::new();
    for (identifier, signing_commitment) in signing_commitments {
        proto_signing_commitments.insert(
            hex::encode(identifier.serialize()),
            operator_rpc::common::SigningCommitment {
                hiding: signing_commitment.hiding().serialize()?,
                binding: signing_commitment.binding().serialize()?,
            },
        );
    }
    Ok(proto_signing_commitments)
}

impl TryFrom<SigningCommitments> for operator_rpc::common::SigningCommitment {
    type Error = ServiceError;

    fn try_from(signing_commitment: SigningCommitments) -> Result<Self, Self::Error> {
        Ok(operator_rpc::common::SigningCommitment {
            hiding: signing_commitment.hiding().serialize().unwrap(),
            binding: signing_commitment.binding().serialize().unwrap(),
        })
    }
}

impl TryFrom<operator_rpc::common::SigningCommitment> for SigningCommitments {
    type Error = ServiceError;

    fn try_from(
        proto_signing_commitments: operator_rpc::common::SigningCommitment,
    ) -> Result<Self, Self::Error> {
        Ok(SigningCommitments::new(
            NonceCommitment::deserialize(&proto_signing_commitments.hiding)?,
            NonceCommitment::deserialize(&proto_signing_commitments.binding)?,
        ))
    }
}

impl TryFrom<SignedTx> for operator_rpc::spark::UserSignedTxSigningJob {
    type Error = ServiceError;

    fn try_from(signed_tx: SignedTx) -> Result<Self, Self::Error> {
        let mut buf = Vec::new();
        signed_tx.tx.consensus_encode(&mut buf)?;

        Ok(operator_rpc::spark::UserSignedTxSigningJob {
            leaf_id: signed_tx.node_id.clone(),
            signing_public_key: signed_tx.signing_public_key.serialize().to_vec(),
            raw_tx: buf,
            signing_nonce_commitment: Some(signed_tx.user_signature_commitment.try_into()?),
            signing_commitments: Some(operator_rpc::spark::SigningCommitments {
                signing_commitments: to_proto_signing_commitments(&signed_tx.signing_commitments)?,
            }),
            user_signature: signed_tx.user_signature.serialize().to_vec(),
        })
    }
}

pub(crate) fn map_public_keys(
    source: HashMap<String, Vec<u8>>,
) -> Result<BTreeMap<Identifier, PublicKey>, ServiceError> {
    let mut public_keys = BTreeMap::new();
    for (identifier, public_key) in source {
        let identifier = Identifier::deserialize(
            &hex::decode(identifier).map_err(|_| ServiceError::InvalidIdentifier)?,
        )
        .map_err(|_| ServiceError::InvalidIdentifier)?;
        let public_key =
            PublicKey::from_slice(&public_key).map_err(|_| ServiceError::InvalidPublicKey)?;
        public_keys.insert(identifier, public_key);
    }

    Ok(public_keys)
}

pub(crate) fn map_signature_shares(
    source: HashMap<String, Vec<u8>>,
) -> Result<BTreeMap<Identifier, SignatureShare>, ServiceError> {
    let mut signature_shares = BTreeMap::new();
    for (identifier, signature_share) in source {
        let identifier = Identifier::deserialize(
            &hex::decode(identifier).map_err(|_| ServiceError::InvalidIdentifier)?,
        )
        .map_err(|_| ServiceError::InvalidIdentifier)?;
        let signature_share = SignatureShare::deserialize(&signature_share)
            .map_err(|_| ServiceError::InvalidSignatureShare)?;
        signature_shares.insert(identifier, signature_share);
    }

    Ok(signature_shares)
}

pub(crate) fn map_signing_nonce_commitments(
    source: HashMap<String, operator_rpc::common::SigningCommitment>,
) -> Result<BTreeMap<Identifier, SigningCommitments>, ServiceError> {
    let mut nonce_commitments = BTreeMap::new();
    for (identifier, commitment) in source {
        let identifier = Identifier::deserialize(
            &hex::decode(identifier).map_err(|_| ServiceError::InvalidIdentifier)?,
        )
        .map_err(|_| ServiceError::InvalidIdentifier)?;
        let commitments = SigningCommitments::new(
            NonceCommitment::deserialize(&commitment.hiding)
                .map_err(|_| ServiceError::InvalidSignatureShare)?,
            NonceCommitment::deserialize(&commitment.binding)
                .map_err(|_| ServiceError::InvalidSignatureShare)?,
        );
        nonce_commitments.insert(identifier, commitments);
    }

    Ok(nonce_commitments)
}
