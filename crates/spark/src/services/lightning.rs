use crate::core::Network;
use crate::operator::rpc::SparkRpcClient;
use crate::operator::rpc::spark::initiate_preimage_swap_request::Reason;
use crate::operator::rpc::spark::{
    GetSigningCommitmentsRequest, InitiatePreimageSwapRequest, InitiatePreimageSwapResponse,
    InvoiceAmount, InvoiceAmountProof, StartUserSignedTransferRequest,
};
use crate::services::ServiceError;
use crate::ssp::{BitcoinNetwork, RequestLightningSendInput, ServiceProvider};
use crate::utils::refund as refund_utils;
use crate::{signer::Signer, tree::TreeNode};
use bip32::secp256k1::elliptic_curve::generic_array::iter;
use bitcoin::hashes::{Hash, sha256};
use bitcoin::secp256k1::PublicKey;
use frost_secp256k1_tr::Identifier;
use hex::ToHex;
use lightning_invoice::Bolt11Invoice;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::Arc;
use uuid::Uuid;

use super::LeafKeyTweak;
use super::models::map_signing_nonce_commitments;

pub struct LightningSwap {
    pub transfer_id: Uuid,
    pub leaves: Vec<LeafKeyTweak>,
    pub receiver_identity_public_key: PublicKey,
    pub bolt11_invoice: Bolt11Invoice,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LightningSendPayment {
    pub id: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub network: Network,
    pub encoded_invoice: String,
    pub fee_msat: u64,
    pub idempotency_key: String,
    pub status: RequestStatus,
    pub transfer_id: Option<String>,
    pub payment_preimage: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RequestStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

impl From<crate::ssp::RequestStatus> for RequestStatus {
    fn from(value: crate::ssp::RequestStatus) -> Self {
        match value {
            crate::ssp::RequestStatus::Pending => RequestStatus::Pending,
            crate::ssp::RequestStatus::InProgress => RequestStatus::InProgress,
            crate::ssp::RequestStatus::Completed => RequestStatus::Completed,
            crate::ssp::RequestStatus::Failed => RequestStatus::Failed,
        }
    }
}

impl TryFrom<crate::ssp::LightningSendRequest> for LightningSendPayment {
    type Error = ServiceError;

    fn try_from(value: crate::ssp::LightningSendRequest) -> Result<Self, Self::Error> {
        Ok(Self {
            id: value.id,
            created_at: value.created_at.timestamp(),
            updated_at: value.updated_at.timestamp(),
            network: value.network.into(),
            encoded_invoice: value.encoded_invoice,
            fee_msat: value
                .fee
                .original_value
                .parse()
                .map_err(|_| ServiceError::Generic("Failed to parse fee".to_string()))?,
            idempotency_key: value.idempotency_key,
            status: value.status.into(),
            transfer_id: value.transfer.and_then(|t| t.spark_id),
            payment_preimage: value.payment_preimage,
        })
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

pub struct LightningService<S>
where
    S: Signer,
{
    spark_client: Arc<SparkRpcClient<S>>,
    ssp_client: Arc<ServiceProvider<S>>,
    network: Network,
    signer: S,
}

impl<S> LightningService<S>
where
    S: Signer,
{
    pub fn new(
        spark_client: Arc<SparkRpcClient<S>>,
        ssp_client: Arc<ServiceProvider<S>>,
        network: Network,
        signer: S,
    ) -> Self {
        LightningService {
            spark_client,
            ssp_client,
            network,
            signer,
        }
    }

    pub async fn start_lightning_swap(
        &self,
        invoice: &str,
        leaves: &Vec<TreeNode>,
    ) -> Result<LightningSwap, ServiceError> {
        let invoice = self.validate_payment(invoice)?;
        let amount_sats = get_invoice_amount_sats(&invoice)?;
        let payment_hash = invoice.payment_hash();

        let leaves_amount: u64 = leaves.iter().map(|l| l.value).sum();
        if leaves_amount != amount_sats {
            return Err(ServiceError::ValidationError(
                "Amount must match the invoice amount".to_string(),
            ));
        }

        // prepare leaf tweaks
        let mut leaf_tweaks = Vec::with_capacity(leaves.len());
        for tree_node in leaves {
            let signing_public_key = self.signer.get_public_key_for_node(&tree_node.id)?;
            let new_signing_public_key = self.signer.generate_random_public_key()?;
            // derive the signing key
            let leaf_tweak = LeafKeyTweak {
                node: tree_node.clone(),
                signing_public_key,
                new_signing_public_key,
            };
            leaf_tweaks.push(leaf_tweak);
        }

        let swap_response = self
            .swap_nodes_for_preimage(
                &leaf_tweaks,
                &self.ssp_client.identity_public_key(),
                payment_hash,
                &invoice,
                amount_sats,
                0, // TODO: this must use the estimated fee.
                false,
            )
            .await?;

        let transfer = swap_response.transfer.ok_or(ServiceError::SSPswapError(
            "Swap response did not contain a transfer".to_string(),
        ))?;

        Ok(LightningSwap {
            transfer_id: Uuid::from_str(&transfer.id).map_err(|_| {
                ServiceError::SSPswapError(
                    "Swap response did not contain a valid transfer id".to_string(),
                )
            })?,
            leaves: leaf_tweaks,
            receiver_identity_public_key: self.ssp_client.identity_public_key(),
            bolt11_invoice: invoice.clone(),
        })
    }

    pub async fn finalize_lightning_swap(
        &self,
        swap: &LightningSwap,
    ) -> Result<LightningSendPayment, ServiceError> {
        let res = self
            .ssp_client
            .request_lightning_send(RequestLightningSendInput {
                encoded_invoice: swap.bolt11_invoice.to_string(),
                idempotency_key: Some(swap.bolt11_invoice.payment_hash().encode_hex()),
            })
            .await?;

        res.try_into()
    }

    pub fn validate_payment(&self, invoice: &str) -> Result<Bolt11Invoice, ServiceError> {
        let invoice = Bolt11Invoice::from_str(invoice)
            .map_err(|err| ServiceError::InvoiceDecodingError(err.to_string()))?;

        // get the invoice amount in sats, then validate the amount
        let amount_sats = get_invoice_amount_sats(&invoice)?;
        if amount_sats == 0 {
            return Err(ServiceError::ValidationError(
                "Amount must be greater than 0".to_string(),
            ));
        }

        Ok(invoice)
    }

    async fn swap_nodes_for_preimage(
        &self,
        leaves: &Vec<LeafKeyTweak>,
        receiver_pubkey: &PublicKey,
        payment_hash: &sha256::Hash,
        invoice: &Bolt11Invoice,
        invoice_amount_sats: u64,
        fee_sats: u64,
        is_inbound_payment: bool,
    ) -> Result<InitiatePreimageSwapResponse, ServiceError> {
        // get signing commitments
        let node_ids: Vec<String> = leaves
            .iter()
            .map(|l| l.node.id.clone().to_string())
            .collect();
        let spark_commitments = self
            .spark_client
            .get_signing_commitments(GetSigningCommitmentsRequest { node_ids })
            .await?;

        // get user signed refunds
        let signing_commitments: Vec<
            BTreeMap<Identifier, frost_secp256k1_tr::round1::SigningCommitments>,
        > = spark_commitments
            .signing_commitments
            .iter()
            .map(|sc| map_signing_nonce_commitments(sc.signing_nonce_commitments.clone()))
            .collect::<Result<Vec<_>, _>>()?;

        let user_signed_refunds = refund_utils::sign_refunds(
            &self.signer,
            leaves,
            signing_commitments,
            receiver_pubkey,
            self.network,
        )
        .await?;

        let transfer_id = Uuid::now_v7().to_string();
        let reason = if is_inbound_payment {
            Reason::Receive
        } else {
            Reason::Send
        };

        let request_data = InitiatePreimageSwapRequest {
            payment_hash: payment_hash.to_byte_array().to_vec(),
            reason: reason as i32,
            invoice_amount: Some(InvoiceAmount {
                invoice_amount_proof: Some(InvoiceAmountProof {
                    bolt11_invoice: invoice.to_string(),
                }),
                value_sats: invoice_amount_sats,
            }),
            transfer: Some(StartUserSignedTransferRequest {
                transfer_id: transfer_id.clone(),
                owner_identity_public_key: self
                    .signer
                    .get_identity_public_key()?
                    .serialize()
                    .to_vec(),
                receiver_identity_public_key: receiver_pubkey.serialize().to_vec(),
                expiry_time: Default::default(),
                leaves_to_send: user_signed_refunds
                    .into_iter()
                    .map(|l| l.try_into())
                    .collect::<Result<Vec<_>, _>>()?,
            }),
            receiver_identity_public_key: receiver_pubkey.serialize().to_vec(),
            fee_sats,
        };

        let response = self
            .spark_client
            .initiate_preimage_swap(request_data)
            .await?;

        Ok(response)
    }
}

fn get_invoice_amount_sats(invoice: &Bolt11Invoice) -> Result<u64, ServiceError> {
    let invoice_amount_msats = invoice
        .amount_milli_satoshis()
        .ok_or(ServiceError::InvoiceDecodingError(invoice.to_string()))?;

    Ok(invoice_amount_msats.div_ceil(1000))
}
