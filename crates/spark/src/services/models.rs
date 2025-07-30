use std::collections::{BTreeMap, HashMap};
use std::fmt::Debug;
use std::str::FromStr;

use bitcoin::Transaction;
use bitcoin::secp256k1::ecdsa::Signature;
use bitcoin::{consensus::deserialize, secp256k1::PublicKey};
use frost_secp256k1_tr::{
    Identifier,
    round1::{NonceCommitment, SigningCommitments},
    round2::SignatureShare,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::core::Network;
use crate::operator::rpc as operator_rpc;
use crate::signer::PrivateKeySource;
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
    identifier: Vec<u8>,
    issuer_public_key: PublicKey,
    name: String,
    ticker: String,
    decimals: u32,
    max_supply: u128,
    is_freezable: bool,
    creation_entity_public_key: Option<PublicKey>,
}

impl TryFrom<operator_rpc::spark_token::TokenMetadata> for TokenMetadata {
    type Error = ServiceError;

    fn try_from(metadata: operator_rpc::spark_token::TokenMetadata) -> Result<Self, Self::Error> {
        let identifier = metadata.token_identifier;
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TokenOutput {
    pub id: String,
    pub owner_public_key: PublicKey,
    pub revocation_commitment: Vec<u8>,
    pub withdraw_bond_sats: u64,
    pub withdraw_relative_block_locktime: u64,
    pub token_public_key: PublicKey,
    pub token_identifier: Vec<u8>,
    pub token_amount: u128,
}

impl TryFrom<operator_rpc::spark_token::TokenOutput> for TokenOutput {
    type Error = ServiceError;

    fn try_from(output: operator_rpc::spark_token::TokenOutput) -> Result<Self, Self::Error> {
        let id = output
            .id
            .ok_or_else(|| ServiceError::Generic("Missing token output id".to_string()))?;
        let owner_public_key = PublicKey::from_slice(&output.owner_public_key)
            .map_err(|_| ServiceError::Generic("Invalid owner public key".to_string()))?;
        let revocation_commitment = output
            .revocation_commitment
            .ok_or_else(|| ServiceError::Generic("Missing revocation commitment".to_string()))?;
        let withdraw_bond_sats = output
            .withdraw_bond_sats
            .ok_or_else(|| ServiceError::Generic("Missing withdraw bond sats".to_string()))?;
        let withdraw_relative_block_locktime =
            output.withdraw_relative_block_locktime.ok_or_else(|| {
                ServiceError::Generic("Missing withdraw relative block locktime".to_string())
            })?;
        let token_public_key = PublicKey::from_slice(
            &output
                .token_public_key
                .ok_or_else(|| ServiceError::Generic("Missing token public key".to_string()))?,
        )
        .map_err(|_| ServiceError::Generic("Invalid token public key".to_string()))?;
        let token_identifier = output
            .token_identifier
            .ok_or_else(|| ServiceError::Generic("Missing token identifier".to_string()))?;
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TokenTransaction {
    pub inputs: Vec<TokenInput>,
    pub outputs: Vec<TokenOutput>,
    pub status: TokenTransactionStatus,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum TokenInput {
    Mint(TokenMintInput),
    Transfer(TokenTransferInput),
    Create(TokenCreateInput),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TokenMintInput {
    pub issuer_public_key: PublicKey,
    pub token_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TokenTransferInput {
    pub outputs_to_spend: Vec<TokenOutputToSpend>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TokenOutputToSpend {
    prev_token_tx_hash: String,
    prev_token_tx_vout: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TokenCreateInput {
    issuer_public_key: PublicKey,
    token_name: String,
    token_ticker: String,
    token_decimals: u8,
    token_max_supply: u128,
    is_freezable: bool,
    creation_entity_public_key: Option<PublicKey>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum TokenTransactionStatus {
    Started,
    Signed,
    Revealed,
    Finalized,
    StartedCancelled,
    SignedCancelled,
    Unknown,
    Unrecognized,
}
