use std::collections::{BTreeMap, HashMap};
use std::fmt::Debug;
use std::str::FromStr;
use std::time::SystemTime;

use bitcoin::secp256k1::ecdsa::Signature;
use bitcoin::{Transaction, TxOut, Txid};
use bitcoin::{consensus::deserialize, secp256k1::PublicKey};
use frost_secp256k1_tr::{
    Identifier,
    round1::{NonceCommitment, SigningCommitments},
    round2::SignatureShare,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::address::SparkAddress;
use crate::core::Network;
use crate::operator::rpc as operator_rpc;
use crate::services::bech32m_encode_token_id;
use crate::signer::{FrostSigningCommitmentsWithNonces, PrivateKeySource};
use crate::tree::{SigningKeyshare, TreeNode, TreeNodeId};
use crate::{ssp::BitcoinNetwork, utils::refund::SignedTx};

use super::ServiceError;

pub use crate::ssp::LightningSendRequestStatus;

pub(crate) type ProofMap = HashMap<TreeNodeId, k256::PublicKey>;

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

impl From<BitcoinNetwork> for Network {
    fn from(value: BitcoinNetwork) -> Self {
        match value {
            BitcoinNetwork::Mainnet => Network::Mainnet,
            BitcoinNetwork::Testnet => Network::Testnet,
            BitcoinNetwork::Signet => Network::Signet,
            BitcoinNetwork::Regtest => Network::Regtest,
        }
    }
}

impl From<Network> for BitcoinNetwork {
    fn from(value: Network) -> Self {
        match value {
            Network::Mainnet => BitcoinNetwork::Mainnet,
            Network::Testnet => BitcoinNetwork::Testnet,
            Network::Signet => BitcoinNetwork::Signet,
            Network::Regtest => BitcoinNetwork::Regtest,
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
            hiding: signing_commitment.hiding().serialize()?,
            binding: signing_commitment.binding().serialize()?,
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
        Ok(operator_rpc::spark::UserSignedTxSigningJob {
            leaf_id: signed_tx.node_id.to_string(),
            signing_public_key: signed_tx.signing_public_key.serialize().to_vec(),
            raw_tx: bitcoin::consensus::serialize(&signed_tx.tx),
            signing_nonce_commitment: Some(signed_tx.user_signature_commitment.try_into()?),
            signing_commitments: Some(operator_rpc::spark::SigningCommitments {
                signing_commitments: to_proto_signing_commitments(&signed_tx.signing_commitments)?,
            }),
            user_signature: signed_tx.user_signature.serialize().to_vec(),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub(crate) enum SigningJobTxType {
    CpfpNode,
    DirectNode,
    CpfpRefund,
    DirectRefund,
    DirectFromCpfpRefund,
}

#[derive(Clone)]
pub(crate) struct SigningJob {
    pub tx_type: SigningJobTxType,
    pub tx: Transaction,
    pub parent_tx_out: TxOut,
    pub signing_public_key: PublicKey,
    pub signing_commitments: FrostSigningCommitmentsWithNonces,
}

impl AsRef<SigningJob> for SigningJob {
    fn as_ref(&self) -> &SigningJob {
        self
    }
}

impl TryFrom<&SigningJob> for operator_rpc::spark::SigningJob {
    type Error = ServiceError;

    fn try_from(signing_job: &SigningJob) -> Result<Self, Self::Error> {
        Ok(operator_rpc::spark::SigningJob {
            raw_tx: bitcoin::consensus::serialize(&signing_job.tx),
            signing_public_key: signing_job.signing_public_key.serialize().to_vec(),
            signing_nonce_commitment: Some(signing_job.signing_commitments.commitments.try_into()?),
        })
    }
}

pub(crate) struct SigningResult {
    pub signing_commitments: BTreeMap<Identifier, SigningCommitments>,
    pub signature_shares: BTreeMap<Identifier, SignatureShare>,
    pub public_keys: BTreeMap<Identifier, PublicKey>,
}

impl TryFrom<&operator_rpc::spark::SigningResult> for SigningResult {
    type Error = ServiceError;

    fn try_from(signing_result: &operator_rpc::spark::SigningResult) -> Result<Self, Self::Error> {
        Ok(SigningResult {
            signing_commitments: map_signing_nonce_commitments(
                &signing_result.signing_nonce_commitments,
            )?,
            signature_shares: map_signature_shares(&signing_result.signature_shares)?,
            public_keys: map_public_keys(&signing_result.public_keys)?,
        })
    }
}

pub(crate) struct ExtendLeafSigningResult {
    pub verifying_key: PublicKey,
    pub signing_result: Option<SigningResult>,
}

impl TryFrom<&operator_rpc::spark::ExtendLeafSigningResult> for ExtendLeafSigningResult {
    type Error = ServiceError;

    fn try_from(
        extend_leaf_signing_result: &operator_rpc::spark::ExtendLeafSigningResult,
    ) -> Result<Self, Self::Error> {
        Ok(ExtendLeafSigningResult {
            verifying_key: PublicKey::from_slice(&extend_leaf_signing_result.verifying_key)
                .map_err(|_| ServiceError::ValidationError("Invalid verifying key".to_string()))?,
            signing_result: extend_leaf_signing_result
                .signing_result
                .as_ref()
                .map(|sr| sr.try_into())
                .transpose()?,
        })
    }
}

pub(crate) fn map_public_keys(
    source: &HashMap<String, Vec<u8>>,
) -> Result<BTreeMap<Identifier, PublicKey>, ServiceError> {
    let mut public_keys = BTreeMap::new();
    for (identifier, public_key) in source {
        let identifier = Identifier::deserialize(
            &hex::decode(identifier).map_err(|_| ServiceError::InvalidIdentifier)?,
        )
        .map_err(|_| ServiceError::InvalidIdentifier)?;
        let public_key =
            PublicKey::from_slice(public_key).map_err(|_| ServiceError::InvalidPublicKey)?;
        public_keys.insert(identifier, public_key);
    }

    Ok(public_keys)
}

pub(crate) fn map_signature_shares(
    source: &HashMap<String, Vec<u8>>,
) -> Result<BTreeMap<Identifier, SignatureShare>, ServiceError> {
    let mut signature_shares = BTreeMap::new();
    for (identifier, signature_share) in source {
        let identifier = Identifier::deserialize(
            &hex::decode(identifier).map_err(|_| ServiceError::InvalidIdentifier)?,
        )
        .map_err(|_| ServiceError::InvalidIdentifier)?;
        let signature_share = SignatureShare::deserialize(signature_share)
            .map_err(|_| ServiceError::InvalidSignatureShare)?;
        signature_shares.insert(identifier, signature_share);
    }

    Ok(signature_shares)
}

pub(crate) fn map_signing_nonce_commitments(
    source: &HashMap<String, operator_rpc::common::SigningCommitment>,
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

#[derive(Debug)]
pub struct LeafKeyTweak {
    pub node: TreeNode,
    pub signing_key: PrivateKeySource,
    pub new_signing_key: PrivateKeySource,
}

// TODO: verify if the optional times should be optional
#[derive(Clone, Debug)]
pub struct Transfer {
    pub id: TransferId,
    pub sender_identity_public_key: PublicKey,
    pub receiver_identity_public_key: PublicKey,
    pub status: TransferStatus,
    pub total_value: u64,
    pub expiry_time: Option<u64>,
    pub leaves: Vec<TransferLeaf>,
    pub created_time: Option<u64>,
    pub updated_time: Option<u64>,
    pub transfer_type: TransferType,
}

impl TryFrom<operator_rpc::spark::Transfer> for Transfer {
    type Error = ServiceError;

    fn try_from(transfer: operator_rpc::spark::Transfer) -> Result<Self, Self::Error> {
        let id = TransferId::from_str(&transfer.id)
            .map_err(|_| ServiceError::Generic("Invalid transfer id".to_string()))?;

        let sender_identity_public_key =
            PublicKey::from_slice(&transfer.sender_identity_public_key).map_err(|_| {
                ServiceError::Generic("Invalid sender identity public key".to_string())
            })?;

        let receiver_identity_public_key =
            PublicKey::from_slice(&transfer.receiver_identity_public_key).map_err(|_| {
                ServiceError::Generic("Invalid receiver identity public key".to_string())
            })?;

        let status = transfer.status().into();

        let transfer_type = transfer.r#type().into();

        let leaves = transfer
            .leaves
            .into_iter()
            .map(|leaf| leaf.try_into())
            .collect::<Result<Vec<_>, _>>()?;

        let expiry_time = transfer.expiry_time.map(|ts| ts.seconds as u64);

        let created_time = transfer.created_time.map(|ts| ts.seconds as u64);

        let updated_time = transfer.updated_time.map(|ts| ts.seconds as u64);

        Ok(Transfer {
            id,
            sender_identity_public_key,
            receiver_identity_public_key,
            status,
            total_value: transfer.total_value,
            expiry_time,
            leaves,
            created_time,
            updated_time,
            transfer_type,
        })
    }
}

#[derive(Clone, Debug)]
pub struct TransferLeaf {
    pub leaf: TreeNode,
    pub secret_cipher: Vec<u8>,
    pub signature: Option<Signature>,
    pub intermediate_refund_tx: Transaction,
    pub intermediate_direct_refund_tx: Option<Transaction>,
    pub intermediate_direct_from_cpfp_refund_tx: Option<Transaction>,
}

impl TryFrom<operator_rpc::spark::TransferLeaf> for TransferLeaf {
    type Error = ServiceError;

    fn try_from(leaf: operator_rpc::spark::TransferLeaf) -> Result<Self, Self::Error> {
        let tree_node = leaf
            .leaf
            .ok_or_else(|| ServiceError::Generic("Missing leaf node".to_string()))?
            .try_into()?;

        let intermediate_refund_tx = deserialize(&leaf.intermediate_refund_tx).map_err(|_| {
            ServiceError::Generic("Invalid intermediate refund transaction".to_string())
        })?;
        let intermediate_direct_refund_tx = if leaf.intermediate_direct_refund_tx.is_empty() {
            None
        } else {
            Some(
                deserialize(&leaf.intermediate_direct_refund_tx).map_err(|_| {
                    ServiceError::Generic(
                        "Invalid intermediate direct refund transaction".to_string(),
                    )
                })?,
            )
        };
        let intermediate_direct_from_cpfp_refund_tx =
            if leaf.intermediate_direct_from_cpfp_refund_tx.is_empty() {
                None
            } else {
                Some(
                    deserialize(&leaf.intermediate_direct_from_cpfp_refund_tx).map_err(|_| {
                        ServiceError::Generic(
                            "Invalid intermediate direct from CPFP refund transaction".to_string(),
                        )
                    })?,
                )
            };

        let signature = match leaf.signature.len() {
            0 => None,
            64 => Some(
                bitcoin::secp256k1::ecdsa::Signature::from_compact(&leaf.signature)
                    .map_err(|_| ServiceError::Generic("Invalid signature format".to_string()))?,
            ),
            _ => Some(
                bitcoin::secp256k1::ecdsa::Signature::from_der(&leaf.signature)
                    .map_err(|_| ServiceError::Generic("Invalid signature format".to_string()))?,
            ),
        };

        Ok(TransferLeaf {
            leaf: tree_node,
            secret_cipher: leaf.secret_cipher,
            signature,
            intermediate_refund_tx,
            intermediate_direct_refund_tx,
            intermediate_direct_from_cpfp_refund_tx,
        })
    }
}

impl TryFrom<operator_rpc::spark::TreeNode> for TreeNode {
    type Error = ServiceError;

    fn try_from(node: operator_rpc::spark::TreeNode) -> Result<Self, Self::Error> {
        let id = node
            .id
            .parse()
            .map_err(|_| ServiceError::Generic(format!("Invalid node id: {}", node.id)))?;

        let parent_node_id = match node.parent_node_id {
            Some(parent_id) => Some(parent_id.parse().map_err(|_| {
                ServiceError::Generic(format!("Invalid parent node id: {parent_id}"))
            })?),
            None => None,
        };

        let node_tx = deserialize(&node.node_tx)
            .map_err(|_| ServiceError::Generic("Invalid node transaction".to_string()))?;

        let refund_tx = if node.refund_tx.is_empty() {
            None
        } else {
            Some(
                deserialize(&node.refund_tx)
                    .map_err(|_| ServiceError::Generic("Invalid refund transaction".to_string()))?,
            )
        };

        let direct_tx = if node.direct_tx.is_empty() {
            None
        } else {
            Some(
                deserialize(&node.direct_tx)
                    .map_err(|_| ServiceError::Generic("Invalid direct transaction".to_string()))?,
            )
        };

        let direct_refund_tx = if node.direct_refund_tx.is_empty() {
            None
        } else {
            Some(deserialize(&node.direct_refund_tx).map_err(|_| {
                ServiceError::Generic("Invalid direct refund transaction".to_string())
            })?)
        };

        let direct_from_cpfp_refund_tx = if node.direct_from_cpfp_refund_tx.is_empty() {
            None
        } else {
            Some(deserialize(&node.direct_from_cpfp_refund_tx).map_err(|_| {
                ServiceError::Generic("Invalid direct from CPFP refund transaction".to_string())
            })?)
        };

        let verifying_public_key = PublicKey::from_slice(&node.verifying_public_key)
            .map_err(|_| ServiceError::Generic("Invalid verifying public key".to_string()))?;

        let owner_identity_public_key = PublicKey::from_slice(&node.owner_identity_public_key)
            .map_err(|_| ServiceError::Generic("Invalid owner identity public key".to_string()))?;

        let signing_keyshare = node
            .signing_keyshare
            .ok_or_else(|| ServiceError::Generic("Missing signing keyshare".to_string()))?
            .try_into()?;

        let status = node
            .status
            .parse()
            .map_err(|_| ServiceError::Generic(format!("Unknown node status: {}", node.status)))?;

        Ok(TreeNode {
            id,
            tree_id: node.tree_id,
            value: node.value,
            parent_node_id,
            node_tx,
            refund_tx,
            direct_tx,
            direct_refund_tx,
            direct_from_cpfp_refund_tx,
            vout: node.vout,
            verifying_public_key,
            owner_identity_public_key,
            signing_keyshare,
            status,
        })
    }
}

impl TryFrom<operator_rpc::spark::SigningKeyshare> for SigningKeyshare {
    type Error = ServiceError;

    fn try_from(keyshare: operator_rpc::spark::SigningKeyshare) -> Result<Self, Self::Error> {
        use frost_secp256k1_tr::Identifier;

        let owner_identifiers = keyshare
            .owner_identifiers
            .into_iter()
            .map(|id_hex| {
                let id_bytes = hex::decode(&id_hex)
                    .map_err(|_| ServiceError::Generic("Invalid hex identifier".to_string()))?;
                Identifier::deserialize(&id_bytes)
                    .map_err(|_| ServiceError::Generic("Invalid identifier".to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let public_key = PublicKey::from_slice(&keyshare.public_key)
            .map_err(|_| ServiceError::Generic("Invalid public key".to_string()))?;

        Ok(SigningKeyshare {
            owner_identifiers,
            threshold: keyshare.threshold,
            public_key,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TransferId(Uuid);

impl TransferId {
    pub fn generate() -> Self {
        Self(Uuid::now_v7())
    }

    pub fn to_bytes(&self) -> [u8; 16] {
        self.0.to_bytes_le()
    }
}

impl std::fmt::Display for TransferId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for TransferId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err("TransferId cannot be empty".to_string());
        }

        // Validate the format of the transfer id
        let uuid = Uuid::from_str(s).map_err(|_| "Invalid TransferId format".to_string())?;
        Ok(TransferId(uuid))
    }
}

impl Serialize for TransferId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for TransferId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        TransferId::from_str(&s).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TransferStatus {
    SenderInitiated,
    SenderKeyTweakPending,
    SenderKeyTweaked,
    ReceiverKeyTweaked,
    ReceiverRefundSigned,
    Completed,
    Expired,
    Returned,
    SenderInitiatedCoordinator,
    ReceiverKeyTweakLocked,
    ReceiverKeyTweakApplied,
}

impl From<operator_rpc::spark::TransferStatus> for TransferStatus {
    fn from(status: operator_rpc::spark::TransferStatus) -> Self {
        match status {
            operator_rpc::spark::TransferStatus::SenderInitiated => TransferStatus::SenderInitiated,
            operator_rpc::spark::TransferStatus::SenderKeyTweakPending => {
                TransferStatus::SenderKeyTweakPending
            }
            operator_rpc::spark::TransferStatus::SenderKeyTweaked => {
                TransferStatus::SenderKeyTweaked
            }
            operator_rpc::spark::TransferStatus::ReceiverKeyTweaked => {
                TransferStatus::ReceiverKeyTweaked
            }
            operator_rpc::spark::TransferStatus::ReceiverRefundSigned => {
                TransferStatus::ReceiverRefundSigned
            }
            operator_rpc::spark::TransferStatus::Completed => TransferStatus::Completed,
            operator_rpc::spark::TransferStatus::Expired => TransferStatus::Expired,
            operator_rpc::spark::TransferStatus::Returned => TransferStatus::Returned,
            operator_rpc::spark::TransferStatus::SenderInitiatedCoordinator => {
                TransferStatus::SenderInitiatedCoordinator
            }
            operator_rpc::spark::TransferStatus::ReceiverKeyTweakLocked => {
                TransferStatus::ReceiverKeyTweakLocked
            }
            operator_rpc::spark::TransferStatus::ReceiverKeyTweakApplied => {
                TransferStatus::ReceiverKeyTweakApplied
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TransferType {
    PreimageSwap,
    CooperativeExit,
    Transfer,
    UtxoSwap,
    Swap,
    CounterSwap,
}

impl From<operator_rpc::spark::TransferType> for TransferType {
    fn from(transfer_type: operator_rpc::spark::TransferType) -> Self {
        match transfer_type {
            operator_rpc::spark::TransferType::PreimageSwap => TransferType::PreimageSwap,
            operator_rpc::spark::TransferType::CooperativeExit => TransferType::CooperativeExit,
            operator_rpc::spark::TransferType::Transfer => TransferType::Transfer,
            operator_rpc::spark::TransferType::UtxoSwap => TransferType::UtxoSwap,
            operator_rpc::spark::TransferType::Swap => TransferType::Swap,
            operator_rpc::spark::TransferType::CounterSwap => TransferType::CounterSwap,
        }
    }
}

#[derive(Debug, Clone)]
pub enum ExitSpeed {
    Fast,
    Medium,
    Slow,
}

impl From<ExitSpeed> for crate::ssp::ExitSpeed {
    fn from(speed: ExitSpeed) -> Self {
        match speed {
            ExitSpeed::Fast => crate::ssp::ExitSpeed::Fast,
            ExitSpeed::Medium => crate::ssp::ExitSpeed::Medium,
            ExitSpeed::Slow => crate::ssp::ExitSpeed::Slow,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TokenMetadata {
    pub identifier: String,
    pub issuer_public_key: PublicKey,
    pub name: String,
    pub ticker: String,
    pub decimals: u32,
    pub max_supply: u128,
    pub is_freezable: bool,
    pub creation_entity_public_key: Option<PublicKey>,
}

impl TryFrom<(operator_rpc::spark_token::TokenMetadata, Network)> for TokenMetadata {
    type Error = ServiceError;

    fn try_from(
        (metadata, network): (operator_rpc::spark_token::TokenMetadata, Network),
    ) -> Result<Self, Self::Error> {
        let identifier = bech32m_encode_token_id(&metadata.token_identifier, network)?;
        let issuer_public_key = PublicKey::from_slice(&metadata.issuer_public_key)
            .map_err(|_| ServiceError::Generic("Invalid issuer public key".to_string()))?;
        let name = metadata.token_name;
        let ticker = metadata.token_ticker;
        let decimals = metadata.decimals;
        let max_supply = u128::from_be_bytes(
            metadata
                .max_supply
                .try_into()
                .map_err(|_| ServiceError::Generic("Invalid max supply".to_string()))?,
        );
        let is_freezable = metadata.is_freezable;
        let creation_entity_public_key = metadata
            .creation_entity_public_key
            .map(|pk| {
                PublicKey::from_slice(&pk).map_err(|_| {
                    ServiceError::Generic("Invalid creation entity public key".to_string())
                })
            })
            .transpose()?;

        Ok(TokenMetadata {
            identifier,
            issuer_public_key,
            name,
            ticker,
            decimals,
            max_supply,
            is_freezable,
            creation_entity_public_key,
        })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct TokenOutput {
    pub id: String,
    pub owner_public_key: PublicKey,
    pub revocation_commitment: String,
    pub withdraw_bond_sats: u64,
    pub withdraw_relative_block_locktime: u64,
    pub token_public_key: Option<PublicKey>,
    pub token_identifier: String,
    pub token_amount: u128,
}

impl TryFrom<(operator_rpc::spark_token::TokenOutput, Network)> for TokenOutput {
    type Error = ServiceError;

    fn try_from(
        (output, network): (operator_rpc::spark_token::TokenOutput, Network),
    ) -> Result<Self, Self::Error> {
        let id = output
            .id
            .ok_or_else(|| ServiceError::Generic("Missing token output id".to_string()))?;
        let owner_public_key = PublicKey::from_slice(&output.owner_public_key)
            .map_err(|_| ServiceError::Generic("Invalid owner public key".to_string()))?;
        let revocation_commitment =
            hex::encode(output.revocation_commitment.ok_or_else(|| {
                ServiceError::Generic("Missing revocation commitment".to_string())
            })?);
        let withdraw_bond_sats = output
            .withdraw_bond_sats
            .ok_or_else(|| ServiceError::Generic("Missing withdraw bond sats".to_string()))?;
        let withdraw_relative_block_locktime =
            output.withdraw_relative_block_locktime.ok_or_else(|| {
                ServiceError::Generic("Missing withdraw relative block locktime".to_string())
            })?;
        let token_public_key = output
            .token_public_key
            .map(|pk| {
                PublicKey::from_slice(&pk)
                    .map_err(|_| ServiceError::Generic("Invalid token public key".to_string()))
            })
            .transpose()?;
        let token_identifier = bech32m_encode_token_id(
            &output
                .token_identifier
                .ok_or_else(|| ServiceError::Generic("Missing token identifier".to_string()))?,
            network,
        )?;
        let token_amount = u128::from_be_bytes(
            output
                .token_amount
                .try_into()
                .map_err(|_| ServiceError::Generic("Invalid token amount".to_string()))?,
        );
        Ok(TokenOutput {
            id,
            owner_public_key,
            revocation_commitment,
            withdraw_bond_sats,
            withdraw_relative_block_locktime,
            token_public_key,
            token_identifier,
            token_amount,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TokenOutputWithPrevOut {
    pub output: TokenOutput,
    pub prev_tx_hash: String,
    pub prev_tx_vout: u32,
}

impl
    TryFrom<(
        operator_rpc::spark_token::OutputWithPreviousTransactionData,
        Network,
    )> for TokenOutputWithPrevOut
{
    type Error = ServiceError;

    fn try_from(
        (output_with_prev_tx_data, network): (
            operator_rpc::spark_token::OutputWithPreviousTransactionData,
            Network,
        ),
    ) -> Result<Self, Self::Error> {
        let output = output_with_prev_tx_data
            .output
            .ok_or_else(|| ServiceError::Generic("Missing token output".to_string()))?;
        let output = TokenOutput::try_from((output, network))?;
        let prev_tx_hash = hex::encode(output_with_prev_tx_data.previous_transaction_hash);
        let prev_tx_vout = output_with_prev_tx_data.previous_transaction_vout;
        Ok(TokenOutputWithPrevOut {
            output,
            prev_tx_hash,
            prev_tx_vout,
        })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TokenTransaction {
    pub hash: String,
    pub inputs: TokenInputs,
    pub outputs: Vec<TokenOutput>,
    pub status: TokenTransactionStatus,
    pub created_timestamp: SystemTime,
}

impl
    TryFrom<(
        operator_rpc::spark_token::TokenTransactionWithStatus,
        Network,
    )> for TokenTransaction
{
    type Error = ServiceError;

    fn try_from(
        (transaction, network): (
            operator_rpc::spark_token::TokenTransactionWithStatus,
            Network,
        ),
    ) -> Result<Self, Self::Error> {
        let token_transaction = transaction.token_transaction.ok_or(ServiceError::Generic(
            "Missing token transaction".to_string(),
        ))?;

        let hash = hex::encode(transaction.token_transaction_hash);

        let inputs = token_transaction
            .token_inputs
            .ok_or(ServiceError::Generic("Missing token inputs".to_string()))?
            .try_into()?;

        let outputs = token_transaction
            .token_outputs
            .into_iter()
            .map(|output| (output, network).try_into())
            .collect::<Result<Vec<TokenOutput>, _>>()?;

        let status =
            operator_rpc::spark_token::TokenTransactionStatus::try_from(transaction.status)
                .map_err(|_| ServiceError::Generic("Invalid token transaction status".to_string()))?
                .into();

        // client_created_timestamp will always be filled for V2 transactions and V1 transactions will be discontinued soon
        let created_timestamp = token_transaction
            .client_created_timestamp
            .map(|ts| {
                std::time::UNIX_EPOCH
                    + std::time::Duration::from_secs(ts.seconds as u64)
                    + std::time::Duration::from_nanos(ts.nanos as u64)
            })
            .ok_or(ServiceError::Generic(
                "Missing client created timestamp. Could this be a V1 transaction?".to_string(),
            ))?;

        Ok(TokenTransaction {
            hash,
            inputs,
            outputs,
            status,
            created_timestamp,
        })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum TokenInputs {
    Mint(TokenMintInput),
    Transfer(TokenTransferInput),
    Create(TokenCreateInput),
}

impl TryFrom<operator_rpc::spark_token::token_transaction::TokenInputs> for TokenInputs {
    type Error = ServiceError;

    fn try_from(
        inputs: operator_rpc::spark_token::token_transaction::TokenInputs,
    ) -> Result<Self, Self::Error> {
        match inputs {
            operator_rpc::spark_token::token_transaction::TokenInputs::MintInput(input) => {
                Ok(TokenInputs::Mint(input.try_into()?))
            }
            operator_rpc::spark_token::token_transaction::TokenInputs::TransferInput(input) => {
                Ok(TokenInputs::Transfer(input.try_into()?))
            }
            operator_rpc::spark_token::token_transaction::TokenInputs::CreateInput(input) => {
                Ok(TokenInputs::Create(input.try_into()?))
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TokenMintInput {
    pub issuer_public_key: PublicKey,
    pub token_id: Option<Vec<u8>>,
}

impl TryFrom<operator_rpc::spark_token::TokenMintInput> for TokenMintInput {
    type Error = ServiceError;

    fn try_from(input: operator_rpc::spark_token::TokenMintInput) -> Result<Self, Self::Error> {
        let issuer_public_key = PublicKey::from_slice(&input.issuer_public_key)
            .map_err(|_| ServiceError::Generic("Invalid issuer public key".to_string()))?;
        let token_id = input.token_identifier;
        Ok(TokenMintInput {
            issuer_public_key,
            token_id,
        })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TokenTransferInput {
    pub outputs_to_spend: Vec<TokenOutputToSpend>,
}

impl TryFrom<operator_rpc::spark_token::TokenTransferInput> for TokenTransferInput {
    type Error = ServiceError;

    fn try_from(input: operator_rpc::spark_token::TokenTransferInput) -> Result<Self, Self::Error> {
        let outputs_to_spend = input
            .outputs_to_spend
            .into_iter()
            .map(|output| output.try_into())
            .collect::<Result<Vec<TokenOutputToSpend>, _>>()?;
        Ok(TokenTransferInput { outputs_to_spend })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TokenOutputToSpend {
    pub prev_token_tx_hash: String,
    pub prev_token_tx_vout: u32,
}

impl TryFrom<operator_rpc::spark_token::TokenOutputToSpend> for TokenOutputToSpend {
    type Error = ServiceError;

    fn try_from(
        output: operator_rpc::spark_token::TokenOutputToSpend,
    ) -> Result<Self, Self::Error> {
        let prev_token_tx_hash = hex::encode(output.prev_token_transaction_hash);
        let prev_token_tx_vout = output.prev_token_transaction_vout;

        Ok(TokenOutputToSpend {
            prev_token_tx_hash,
            prev_token_tx_vout,
        })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TokenCreateInput {
    issuer_public_key: PublicKey,
    name: String,
    ticker: String,
    decimals: u32,
    max_supply: u128,
    is_freezable: bool,
    creation_entity_public_key: Option<PublicKey>,
}

impl TryFrom<operator_rpc::spark_token::TokenCreateInput> for TokenCreateInput {
    type Error = ServiceError;

    fn try_from(input: operator_rpc::spark_token::TokenCreateInput) -> Result<Self, Self::Error> {
        let issuer_public_key = PublicKey::from_slice(&input.issuer_public_key)
            .map_err(|_| ServiceError::Generic("Invalid issuer public key".to_string()))?;
        let name = input.token_name;
        let ticker = input.token_ticker;
        let decimals = input.decimals;
        let max_supply = u128::from_be_bytes(
            input
                .max_supply
                .try_into()
                .map_err(|_| ServiceError::Generic("Invalid max supply".to_string()))?,
        );
        let is_freezable = input.is_freezable;
        let creation_entity_public_key = input
            .creation_entity_public_key
            .map(|pk| {
                PublicKey::from_slice(&pk).map_err(|_| {
                    ServiceError::Generic("Invalid creation entity public key".to_string())
                })
            })
            .transpose()?;

        Ok(TokenCreateInput {
            issuer_public_key,
            name,
            ticker,
            decimals,
            max_supply,
            is_freezable,
            creation_entity_public_key,
        })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum TokenTransactionStatus {
    /// Transaction was successfully constructed and validated by the Operator.
    Started,
    /// If not transfer transaction, transaction was accepted by the Operator and outputs are spendable.
    /// Else, transaction was accepted by the Operator, inputs are 'locked' until consensus or until transaction expiry (in the event that consensus is not reached)
    Signed,
    /// Operator has shared its revocation secret shares with other operators and is waiting for the system to collect enough shares to finalize the transaction.
    Revealed,
    /// Transaction has reached consensus across operators. Transaction is final.
    Finalized,
    /// Transaction was cancelled and cannot be recovered.
    StartedCancelled,
    /// Transaction was cancelled and cannot be recovered.
    SignedCancelled,
    Unknown,
}

impl From<operator_rpc::spark_token::TokenTransactionStatus> for TokenTransactionStatus {
    fn from(status: operator_rpc::spark_token::TokenTransactionStatus) -> Self {
        match status {
            operator_rpc::spark_token::TokenTransactionStatus::TokenTransactionStarted => {
                TokenTransactionStatus::Started
            }
            operator_rpc::spark_token::TokenTransactionStatus::TokenTransactionSigned => {
                TokenTransactionStatus::Signed
            }
            operator_rpc::spark_token::TokenTransactionStatus::TokenTransactionRevealed => {
                TokenTransactionStatus::Revealed
            }
            operator_rpc::spark_token::TokenTransactionStatus::TokenTransactionFinalized => {
                TokenTransactionStatus::Finalized
            }
            operator_rpc::spark_token::TokenTransactionStatus::TokenTransactionStartedCancelled => {
                TokenTransactionStatus::StartedCancelled
            }
            operator_rpc::spark_token::TokenTransactionStatus::TokenTransactionSignedCancelled => {
                TokenTransactionStatus::SignedCancelled
            }
            operator_rpc::spark_token::TokenTransactionStatus::TokenTransactionUnknown => {
                TokenTransactionStatus::Unknown
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct QueryTokenTransactionsFilter {
    /// If not provided, will use our own public key
    pub owner_public_keys: Option<Vec<PublicKey>>,
    pub issuer_public_keys: Vec<PublicKey>,
    pub token_transaction_hashes: Vec<String>,
    pub token_ids: Vec<String>,
    pub output_ids: Vec<String>,
}

pub struct Utxo {
    pub tx: Option<Transaction>,
    pub vout: u32,
    pub network: Network,
    pub txid: Txid,
}

impl TryFrom<operator_rpc::spark::Utxo> for Utxo {
    type Error = ServiceError;

    fn try_from(utxo: operator_rpc::spark::Utxo) -> Result<Self, Self::Error> {
        let network = Network::from_proto_network(utxo.network)
            .map_err(|_| ServiceError::InvalidNetwork(utxo.network))?;
        let mut tx: Option<Transaction> = None;
        if let Ok(t) = deserialize(&utxo.raw_tx) {
            tx = Some(t);
        }
        Ok(Utxo {
            tx,
            vout: utxo.vout,
            network,
            txid: Txid::from_str(&hex::encode(utxo.txid))
                .map_err(|_| ServiceError::InvalidTransaction)?,
        })
    }
}

#[derive(Clone, Debug)]
pub struct TransferTokenOutput {
    pub token_id: String,
    pub amount: u128,
    pub receiver_address: SparkAddress,
}

#[cfg(test)]
mod tests {
    use bitcoin::secp256k1::PublicKey;
    use macros::test_all;

    use crate::operator::rpc as operator_rpc;
    use crate::services::bech32m_decode_token_id;
    use crate::{Network, services::TokenOutputWithPrevOut};

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    #[test_all]
    fn test_token_output_with_prev_out_roundtrip() {
        let token_id = "123";
        let owner_public_key = PublicKey::from_slice(&[
            3, 141, 37, 201, 160, 148, 226, 93, 184, 201, 131, 47, 222, 91, 55, 171, 38, 95, 13,
            248, 175, 190, 44, 132, 189, 75, 131, 204, 215, 82, 93, 167, 177,
        ])
        .unwrap();
        let revocation_commitment = vec![1, 2, 3, 4, 5, 6];
        let withdraw_bond_sats = 5;
        let withdraw_relative_block_locktime = 10;
        let token_public_key = PublicKey::from_slice(&[
            2, 127, 55, 243, 159, 164, 203, 75, 127, 192, 114, 94, 161, 176, 56, 167, 40, 38, 14,
            107, 203, 243, 227, 234, 184, 42, 180, 200, 218, 192, 76, 120, 108,
        ])
        .unwrap();
        let token_identifier = "btknrt1sn0g08xew2y6fzcvlca3kpdmy5ftkd7skg2m8dxh5kmwfnm7lpaq487jlc";
        let token_amount = 100000000u128;
        let previous_transaction_hash = vec![1, 2, 3];
        let previous_transaction_vout = 5;

        let output_with_previous_transaction_data =
            operator_rpc::spark_token::OutputWithPreviousTransactionData {
                output: Some(operator_rpc::spark_token::TokenOutput {
                    id: Some(token_id.to_string()),
                    owner_public_key: owner_public_key.serialize().to_vec(),
                    revocation_commitment: Some(revocation_commitment.clone()),
                    withdraw_bond_sats: Some(withdraw_bond_sats),
                    withdraw_relative_block_locktime: Some(withdraw_relative_block_locktime),
                    token_public_key: Some(token_public_key.serialize().to_vec()),
                    token_identifier: Some(
                        bech32m_decode_token_id(token_identifier, Some(Network::Regtest)).unwrap(),
                    ),
                    token_amount: token_amount.to_be_bytes().to_vec(),
                }),
                previous_transaction_hash: previous_transaction_hash.clone(),
                previous_transaction_vout,
            };

        let output_with_prev_out = TokenOutputWithPrevOut::try_from((
            output_with_previous_transaction_data.clone(),
            Network::Regtest,
        ))
        .unwrap();

        assert_eq!(output_with_prev_out.output.id, token_id);
        assert_eq!(
            output_with_prev_out.output.owner_public_key,
            owner_public_key
        );
        assert_eq!(
            output_with_prev_out.output.revocation_commitment,
            hex::encode(revocation_commitment)
        );
        assert_eq!(
            output_with_prev_out.output.withdraw_bond_sats,
            withdraw_bond_sats
        );
        assert_eq!(
            output_with_prev_out.output.withdraw_relative_block_locktime,
            withdraw_relative_block_locktime
        );
        assert_eq!(
            output_with_prev_out.output.token_public_key,
            Some(token_public_key)
        );
        assert_eq!(
            output_with_prev_out.output.token_identifier,
            token_identifier
        );
        assert_eq!(output_with_prev_out.output.token_amount, token_amount);
        assert_eq!(
            output_with_prev_out.prev_tx_hash,
            hex::encode(previous_transaction_hash)
        );
        assert_eq!(output_with_prev_out.prev_tx_vout, previous_transaction_vout);
    }
}
