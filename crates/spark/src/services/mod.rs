mod deposit;
mod error;
mod lightning;
mod transfer;

use std::collections::{BTreeMap, HashMap};

use bitcoin::consensus::Encodable;
pub use deposit::*;
pub use error::*;
pub use lightning::{LightningSendPayment, LightningService};
pub use transfer::*;

use frost_secp256k1_tr::{
    Identifier,
    round1::{NonceCommitment, SigningCommitments},
};

use crate::utils::refund::SignedTx;

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

pub(crate) fn from_proto_signing_commitments(
    proto_signing_commitments: HashMap<String, spark_protos::common::SigningCommitment>,
) -> Result<BTreeMap<Identifier, SigningCommitments>, ServiceError> {
    let mut signing_commitments = BTreeMap::new();
    for (identifier, signing_commitment) in proto_signing_commitments {
        signing_commitments.insert(
            Identifier::deserialize(&hex::decode(identifier).unwrap())?,
            from_proto_signing_commitment(signing_commitment)?,
        );
    }
    Ok(signing_commitments)
}

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
