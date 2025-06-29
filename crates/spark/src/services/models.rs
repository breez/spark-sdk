use std::collections::{BTreeMap, HashMap};

use bitcoin::{consensus::Encodable, secp256k1::PublicKey};

use frost_secp256k1_tr::{
    Identifier,
    round1::{NonceCommitment, SigningCommitments},
    round2::SignatureShare,
};
use spark_protos::common;

use crate::utils::refund::SignedTx;

use super::ServiceError;

impl From<crate::Network> for spark_protos::spark::Network {
    fn from(network: crate::Network) -> Self {
        match network {
            crate::Network::Mainnet => spark_protos::spark::Network::Mainnet,
            crate::Network::Regtest => spark_protos::spark::Network::Regtest,
            crate::Network::Testnet => spark_protos::spark::Network::Testnet,
            crate::Network::Signet => spark_protos::spark::Network::Signet,
        }
    }
}

pub(crate) fn to_proto_signing_commitments(
    signing_commitments: &BTreeMap<Identifier, SigningCommitments>,
) -> Result<HashMap<String, spark_protos::common::SigningCommitment>, ServiceError> {
    let mut proto_signing_commitments = HashMap::new();
    for (identifier, signing_commitment) in signing_commitments {
        proto_signing_commitments.insert(
            hex::encode(identifier.serialize()),
            spark_protos::common::SigningCommitment {
                hiding: signing_commitment.hiding().serialize()?,
                binding: signing_commitment.binding().serialize()?,
            },
        );
    }
    Ok(proto_signing_commitments)
}

// pub(crate) fn from_proto_signing_commitments(
//     proto_signing_commitments: HashMap<String, spark_protos::common::SigningCommitment>,
// ) -> Result<BTreeMap<Identifier, SigningCommitments>, ServiceError> {
//     let mut signing_commitments = BTreeMap::new();
//     for (identifier, signing_commitment) in proto_signing_commitments {
//         signing_commitments.insert(
//             Identifier::deserialize(&hex::decode(identifier).unwrap())?,
//             from_proto_signing_commitment(signing_commitment)?,
//         );
//     }
//     Ok(signing_commitments)
// }

pub(crate) fn to_proto_signing_commitment(
    signing_commitment: &SigningCommitments,
) -> Result<spark_protos::common::SigningCommitment, ServiceError> {
    Ok(spark_protos::common::SigningCommitment {
        hiding: signing_commitment.hiding().serialize()?,
        binding: signing_commitment.binding().serialize()?,
    })
}

pub(crate) fn from_proto_signing_commitment(
    proto_signing_commitments: spark_protos::common::SigningCommitment,
) -> Result<SigningCommitments, ServiceError> {
    Ok(SigningCommitments::new(
        NonceCommitment::deserialize(&proto_signing_commitments.hiding)?,
        NonceCommitment::deserialize(&proto_signing_commitments.binding)?,
    ))
}

pub(crate) fn to_proto_signed_tx(
    signed_tx: &SignedTx,
) -> Result<spark_protos::spark::UserSignedTxSigningJob, ServiceError> {
    let mut buf = Vec::new();
    signed_tx.tx.consensus_encode(&mut buf)?;

    Ok(spark_protos::spark::UserSignedTxSigningJob {
        leaf_id: signed_tx.node_id.clone(),
        signing_public_key: signed_tx.signing_public_key.serialize().to_vec(),
        raw_tx: buf,
        signing_nonce_commitment: Some(to_proto_signing_commitment(
            &signed_tx.user_signature_commitment,
        )?),
        signing_commitments: Some(spark_protos::spark::SigningCommitments {
            signing_commitments: to_proto_signing_commitments(&signed_tx.signing_commitments)?,
        }),
        user_signature: signed_tx.user_signature.serialize().to_vec(),
    })
}

pub(crate) fn marshal_frost_commitment(
    commitments: &SigningCommitments,
) -> Result<common::SigningCommitment, ServiceError> {
    let hiding = commitments.hiding().serialize().unwrap();
    let binding = commitments.binding().serialize().unwrap();

    Ok(common::SigningCommitment { hiding, binding })
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
    source: HashMap<String, common::SigningCommitment>,
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
