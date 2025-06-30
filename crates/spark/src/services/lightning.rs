use crate::core::Network;
use crate::operator::OperatorPool;
use crate::operator::rpc::SparkRpcClient;
use crate::operator::rpc::spark::initiate_preimage_swap_request::Reason;
use crate::operator::rpc::spark::{
    GetSigningCommitmentsRequest, InitiatePreimageSwapRequest, InitiatePreimageSwapResponse,
    InvoiceAmount, InvoiceAmountProof, SecretShare, StartUserSignedTransferRequest,
    StorePreimageShareRequest,
};
use crate::services::ServiceError;
use crate::ssp::{RequestLightningReceiveInput, RequestLightningSendInput, ServiceProvider};
use crate::utils::refund as refund_utils;
use crate::{signer::Signer, tree::TreeNode};
use bitcoin::hashes::{Hash, sha256};
use bitcoin::secp256k1::PublicKey;
use frost_secp256k1_tr::Identifier;
use hex::ToHex;
use lightning_invoice::Bolt11Invoice;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::Arc;
use uuid::Uuid;

use super::LeafKeyTweak;
use super::models::{RequestStatus, map_signing_nonce_commitments};

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

pub struct LightningReceiveRequest {
    pub id: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub network: Network,
    pub status: RequestStatus,
    pub invoice: String,
    pub transfer_id: Option<String>,
    pub transfer_amount_sat: Option<u64>,
    pub payment_preimage: Option<String>,
}

impl TryFrom<crate::ssp::LightningReceiveRequest> for LightningReceiveRequest {
    type Error = ServiceError;

    fn try_from(value: crate::ssp::LightningReceiveRequest) -> Result<Self, Self::Error> {
        Ok(Self {
            id: value.id,
            created_at: value.created_at.timestamp(),
            updated_at: value.updated_at.timestamp(),
            network: value.network.into(),
            status: value.status.into(),
            invoice: value.invoice.encoded_invoice,
            transfer_id: value.transfer.as_ref().and_then(|t| t.spark_id.clone()),
            transfer_amount_sat: match value.transfer {
                Some(t) => Some(
                    t.total_amount
                        .original_value
                        .parse()
                        .map_err(|_| ServiceError::Generic("Failed to parse fee".to_string()))?,
                ),
                None => None,
            },
            payment_preimage: value.payment_preimage,
        })
    }
}

pub struct LightningService<S>
where
    S: Signer,
{
    coordinator_spark_client: Arc<SparkRpcClient<S>>,
    operators_spark_clients: Vec<Arc<SparkRpcClient<S>>>,
    ssp_client: Arc<ServiceProvider<S>>,
    network: Network,
    signer: S,
    split_secret_threshold: u32,
}

impl<S> LightningService<S>
where
    S: Signer,
{
    pub fn new(
        coordinator_spark_client: Arc<SparkRpcClient<S>>,
        operators_spark_clients: Vec<Arc<SparkRpcClient<S>>>,
        ssp_client: Arc<ServiceProvider<S>>,
        network: Network,
        signer: S,
        split_secret_threshold: u32,
    ) -> Self {
        LightningService {
            coordinator_spark_client,
            operators_spark_clients,
            ssp_client,
            network,
            signer,
            split_secret_threshold,
        }
    }

    pub async fn create_lightning_invoice_with_preimage(
        &self,
        amount_sats: i64,
        memo: Option<String>,
        preimage: Vec<u8>,
        expiry_secs: Option<i32>,
    ) -> Result<LightningReceiveRequest, ServiceError> {
        let payment_hash = sha256::Hash::hash(&preimage);
        let invoice = self
            .ssp_client
            .request_lightning_receive(RequestLightningReceiveInput {
                receiver_identity_pubkey: Some(
                    self.signer
                        .get_identity_public_key()?
                        .serialize()
                        .to_vec()
                        .encode_hex(),
                ),
                amount_sats,
                network: self.network.into(),
                payment_hash: Some(payment_hash.encode_hex()),
                description_hash: None,
                expiry_secs: expiry_secs,
                memo: memo,
                include_spark_address: Some(false),
            })
            .await?;

        let shares = self
            .signer
            .split_secret_with_proofs(
                preimage,
                self.split_secret_threshold,
                self.operators_spark_clients.len() as u32,
            )
            .await?;

        let identity_pubkey = self.signer.get_identity_public_key()?;
        let requests = self
            .operators_spark_clients
            .iter()
            .zip(shares)
            .map(|(operator, share)| {
                operator.store_preimage_share(StorePreimageShareRequest {
                    payment_hash: payment_hash.to_byte_array().to_vec(),
                    preimage_share: Some(SecretShare {
                        secret_share: share.secret_share.share.to_bytes().to_vec(),
                        proofs: share.proofs,
                    }),
                    threshold: share.secret_share.threshold as u32,
                    invoice_string: invoice.clone().invoice.encoded_invoice,
                    user_identity_public_key: identity_pubkey.serialize().to_vec(),
                })
            });

        futures::future::try_join_all(requests)
            .await
            .map_err(|_| ServiceError::PerimageShareStoreFailed)?;

        invoice.try_into()
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
            .coordinator_spark_client
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
            .coordinator_spark_client
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
