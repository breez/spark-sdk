use crate::address::SparkAddress;
use crate::core::Network;
use crate::operator::OperatorPool;
use crate::operator::rpc::spark::initiate_preimage_swap_request::Reason;
use crate::operator::rpc::spark::{
    GetSigningCommitmentsRequest, InitiatePreimageSwapRequest, InitiatePreimageSwapResponse,
    InvoiceAmount, InvoiceAmountProof, SecretShare, StartUserSignedTransferRequest,
    StorePreimageShareRequest,
};
use crate::services::{ServiceError, Transfer, TransferId};
use crate::signer::SecretToSplit;
use crate::ssp::{
    LightningReceiveRequestStatus, RequestLightningReceiveInput, RequestLightningSendInput,
    ServiceProvider,
};
use crate::utils::{leaf_key_tweak::prepare_leaf_key_tweaks_to_send, refund::sign_refunds};
use crate::{signer::Signer, tree::TreeNode};
use bitcoin::hashes::{Hash, sha256};
use bitcoin::secp256k1::PublicKey;
use frost_secp256k1_tr::Identifier;
use hex::ToHex;
use lightning_invoice::Bolt11Invoice;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::Arc;

use super::LeafKeyTweak;
use super::models::{LightningSendRequestStatus, map_signing_nonce_commitments};

const DEFAULT_EXPIRY_SECS: u32 = 60 * 60 * 24 * 30;
const RECEIVER_IDENTITY_PUBLIC_KEY_SHORT_CHANNEL_ID: u64 = 17592187092992000001;

pub struct LightningSwap {
    pub transfer: Transfer,
    pub leaves: Vec<LeafKeyTweak>,
    pub receiver_identity_public_key: PublicKey,
    pub bolt11_invoice: String,
    pub user_amount_sat: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LightningSendPayment {
    pub id: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub network: Network,
    pub encoded_invoice: String,
    pub fee_msat: u64,
    pub idempotency_key: String,
    pub status: LightningSendRequestStatus,
    pub transfer_id: Option<TransferId>,
    pub payment_preimage: Option<String>,
}

impl TryFrom<crate::ssp::LightningSendRequest> for LightningSendPayment {
    type Error = ServiceError;

    fn try_from(value: crate::ssp::LightningSendRequest) -> Result<Self, Self::Error> {
        let transfer_id = match &value.transfer {
            Some(transfer) => match &transfer.spark_id {
                Some(id) => Some(TransferId::from_str(id).map_err(|_| {
                    ServiceError::SSPswapError("Invalid transfer id format".to_string())
                })?),
                None => None,
            },
            None => None,
        };
        Ok(Self {
            id: value.id,
            created_at: value.created_at.timestamp(),
            updated_at: value.updated_at.timestamp(),
            network: value.network.into(),
            encoded_invoice: value.encoded_invoice,
            fee_msat: value
                .fee
                .as_sats()
                .map_err(|_| ServiceError::Generic("Failed to parse fee".to_string()))?,
            idempotency_key: value.idempotency_key,
            status: value.status,
            transfer_id,
            payment_preimage: value.lightning_send_payment_preimage,
        })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LightningReceivePayment {
    pub id: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub network: Network,
    pub status: LightningReceiveRequestStatus,
    pub invoice: String,
    pub transfer_id: Option<TransferId>,
    pub transfer_amount_sat: Option<u64>,
    pub payment_preimage: Option<String>,
}

impl TryFrom<crate::ssp::LightningReceiveRequest> for LightningReceivePayment {
    type Error = ServiceError;

    fn try_from(value: crate::ssp::LightningReceiveRequest) -> Result<Self, Self::Error> {
        let transfer_id = match &value.transfer {
            Some(transfer) => match &transfer.spark_id {
                Some(id) => Some(TransferId::from_str(id).map_err(|_| {
                    ServiceError::SSPswapError("Invalid transfer id format".to_string())
                })?),
                None => None,
            },
            None => None,
        };
        Ok(Self {
            id: value.id,
            created_at: value.created_at.timestamp(),
            updated_at: value.updated_at.timestamp(),
            network: value.network.into(),
            status: value.lightning_request_status,
            invoice: value.invoice.encoded_invoice,
            transfer_id,
            transfer_amount_sat: match value.transfer {
                Some(t) => Some(
                    t.total_amount
                        .as_sats()
                        .map_err(|_| ServiceError::Generic("Failed to parse fee".to_string()))?,
                ),
                None => None,
            },
            payment_preimage: value.lightning_receive_payment_preimage,
        })
    }
}

struct SwapNodesForPreimageRequest<'a> {
    leaves: &'a [LeafKeyTweak],
    receiver_pubkey: &'a PublicKey,
    payment_hash: &'a sha256::Hash,
    invoice: &'a str,
    invoice_amount_sats: u64,
    fee_sats: u64,
    is_inbound_payment: bool,
}

pub struct LightningService<S> {
    operator_pool: Arc<OperatorPool<S>>,
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
        operator_pool: Arc<OperatorPool<S>>,
        ssp_client: Arc<ServiceProvider<S>>,
        network: Network,
        signer: S,
        split_secret_threshold: u32,
    ) -> Self {
        LightningService {
            operator_pool,
            ssp_client,
            network,
            signer,
            split_secret_threshold,
        }
    }

    pub async fn create_lightning_invoice(
        &self,
        amount_sats: u64,
        memo: Option<String>,
        preimage: Option<Vec<u8>>,
        expiry_secs: Option<u32>,
    ) -> Result<LightningReceivePayment, ServiceError> {
        let preimage = preimage.unwrap_or_else(|| {
            bitcoin::secp256k1::SecretKey::new(&mut OsRng)
                .secret_bytes()
                .to_vec()
        });
        let expiry = expiry_secs.unwrap_or(DEFAULT_EXPIRY_SECS);
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
                payment_hash: payment_hash.encode_hex(),
                description_hash: None,
                expiry_secs: Some(expiry.into()),
                memo,
                include_spark_address: true,
            })
            .await?;
        let decoded_invoice = Bolt11Invoice::from_str(&invoice.invoice.encoded_invoice)
            .map_err(|err| ServiceError::InvoiceDecodingError(err.to_string()))?;
        let spark_address = self.extract_spark_address(&decoded_invoice);
        let Some(spark_address) = spark_address else {
            return Err(ServiceError::SSPswapError(
                "Invalid invoice. Spark address not found".to_string(),
            ));
        };

        let identity_pubkey = self.signer.get_identity_public_key()?;
        if spark_address.identity_public_key != identity_pubkey {
            return Err(ServiceError::SSPswapError(
                "Invalid invoice. Spark address mismatch".to_string(),
            ));
        }
        let shares = self.signer.split_secret_with_proofs(
            &SecretToSplit::Preimage(preimage),
            self.split_secret_threshold,
            self.operator_pool.len(),
        )?;

        let requests =
            self.operator_pool
                .get_all_operators()
                .zip(shares)
                .map(|(operator, share)| {
                    operator
                        .client
                        .store_preimage_share(StorePreimageShareRequest {
                            payment_hash: payment_hash.to_byte_array().to_vec(),
                            preimage_share: Some(SecretShare {
                                secret_share: share.secret_share.share.to_bytes().to_vec(),
                                proofs: share
                                    .proofs
                                    .iter()
                                    .map(|p| p.to_sec1_bytes().to_vec())
                                    .collect(),
                            }),
                            threshold: share.secret_share.threshold as u32,
                            invoice_string: invoice.clone().invoice.encoded_invoice,
                            user_identity_public_key: identity_pubkey.serialize().to_vec(),
                        })
                });

        futures::future::try_join_all(requests)
            .await
            .map_err(|e| ServiceError::PreimageShareStoreFailed(e.to_string()))?;

        invoice.try_into()
    }

    pub async fn start_lightning_swap(
        &self,
        invoice: &str,
        amount_to_send: Option<u64>,
        leaves: &[TreeNode],
    ) -> Result<LightningSwap, ServiceError> {
        let decoded_invoice = Bolt11Invoice::from_str(invoice)
            .map_err(|err| ServiceError::InvoiceDecodingError(err.to_string()))?;
        let amount_sats: u64 = get_invoice_amount_sats(&decoded_invoice, amount_to_send)?;
        let payment_hash = decoded_invoice.payment_hash();

        // prepare leaf tweaks
        let leaf_tweaks = prepare_leaf_key_tweaks_to_send(&self.signer, leaves.to_vec())?;

        let swap_response = self
            .swap_nodes_for_preimage(SwapNodesForPreimageRequest {
                leaves: &leaf_tweaks,
                receiver_pubkey: &self.ssp_client.identity_public_key(),
                payment_hash,
                invoice,
                invoice_amount_sats: amount_sats,
                fee_sats: 0, // TODO: this must use the estimated fee.
                is_inbound_payment: false,
            })
            .await?;

        let transfer = swap_response.transfer.ok_or(ServiceError::SSPswapError(
            "Swap response did not contain a transfer".to_string(),
        ))?;

        Ok(LightningSwap {
            transfer: transfer.try_into()?,
            leaves: leaf_tweaks,
            receiver_identity_public_key: self.ssp_client.identity_public_key(),
            bolt11_invoice: invoice.to_string(),
            user_amount_sat: amount_to_send,
        })
    }

    pub async fn finalize_lightning_swap(
        &self,
        swap: &LightningSwap,
    ) -> Result<LightningSendPayment, ServiceError> {
        let decoded_invoice = Bolt11Invoice::from_str(&swap.bolt11_invoice)
            .map_err(|err| ServiceError::InvoiceDecodingError(err.to_string()))?;
        let res = self
            .ssp_client
            .request_lightning_send(RequestLightningSendInput {
                encoded_invoice: swap.bolt11_invoice.to_string(),
                idempotency_key: decoded_invoice.payment_hash().encode_hex(),
                amount_sats: swap.user_amount_sat,
            })
            .await?;

        res.try_into()
    }

    pub async fn validate_payment(
        &self,
        invoice: &str,
        max_fee_sat: Option<u64>,
        amount_to_send: Option<u64>,
        prefer_spark: bool,
    ) -> Result<(u64, Option<SparkAddress>), ServiceError> {
        let decoded_invoice = Bolt11Invoice::from_str(invoice)
            .map_err(|err| ServiceError::InvoiceDecodingError(err.to_string()))?;

        if decoded_invoice.network() != self.network.into() {
            return Err(ServiceError::ValidationError(
                "Invoice network does not match".to_string(),
            ));
        }

        // get the invoice amount in sats, then validate the amount
        let to_pay_sat = get_invoice_amount_sats(&decoded_invoice, amount_to_send)?;
        if prefer_spark {
            if let Some(receiver_address) = self.extract_spark_address(&decoded_invoice) {
                return Ok((to_pay_sat, Some(receiver_address)));
            }
        }

        let fee_estimate = self
            .ssp_client
            .get_lightning_send_fee_estimate(invoice, Some(to_pay_sat))
            .await?;

        let fee_sat = fee_estimate
            .as_sats()
            .map_err(|_| ServiceError::Generic("Failed to parse fee".to_string()))?;
        if let Some(max_fee_sat) = max_fee_sat {
            if fee_sat > max_fee_sat {
                return Err(ServiceError::ValidationError(
                    "Fee exceeds maximum allowed fee".to_string(),
                ));
            }
        }

        Ok((fee_sat + to_pay_sat, None))
    }

    fn extract_spark_address(&self, decoded_invoice: &Bolt11Invoice) -> Option<SparkAddress> {
        for route_hint in decoded_invoice.route_hints() {
            for node in route_hint.0 {
                if node.short_channel_id == RECEIVER_IDENTITY_PUBLIC_KEY_SHORT_CHANNEL_ID {
                    return Some(SparkAddress::new(node.src_node_id, self.network, None));
                }
            }
        }
        None
    }

    pub async fn fetch_lightning_send_fee_estimate(
        &self,
        invoice: &str,
        amount_to_send: Option<u64>,
    ) -> Result<u64, ServiceError> {
        let decoded_invoice = Bolt11Invoice::from_str(invoice)
            .map_err(|err| ServiceError::InvoiceDecodingError(err.to_string()))?;
        let amount_sat = get_invoice_amount_sats(&decoded_invoice, amount_to_send)?;
        self.ssp_client
            .get_lightning_send_fee_estimate(invoice, Some(amount_sat))
            .await?
            .as_sats()
            .map_err(|_| ServiceError::Generic("Failed to parse fee".to_string()))
    }

    pub async fn get_lightning_send_payment(
        &self,
        id: &str,
    ) -> Result<Option<LightningSendPayment>, ServiceError> {
        let res = self.ssp_client.get_lightning_send_request(id).await?;

        match res {
            Some(request) => Ok(Some(request.try_into()?)),
            None => Ok(None),
        }
    }

    pub async fn get_lightning_receive_payment(
        &self,
        id: &str,
    ) -> Result<Option<LightningReceivePayment>, ServiceError> {
        let res = self.ssp_client.get_lightning_receive_request(id).await?;

        match res {
            Some(request) => Ok(Some(request.try_into()?)),
            None => Ok(None),
        }
    }

    async fn swap_nodes_for_preimage(
        &self,
        req: SwapNodesForPreimageRequest<'_>,
    ) -> Result<InitiatePreimageSwapResponse, ServiceError> {
        // get signing commitments
        let node_ids: Vec<String> = req
            .leaves
            .iter()
            .map(|l| l.node.id.clone().to_string())
            .collect();
        let spark_commitments = self
            .operator_pool
            .get_coordinator()
            .client
            .get_signing_commitments(GetSigningCommitmentsRequest { node_ids })
            .await?;

        // get user signed refunds
        let signing_commitments: Vec<
            BTreeMap<Identifier, frost_secp256k1_tr::round1::SigningCommitments>,
        > = spark_commitments
            .signing_commitments
            .iter()
            .map(|sc| map_signing_nonce_commitments(&sc.signing_nonce_commitments))
            .collect::<Result<Vec<_>, _>>()?;

        let user_signed_refunds = sign_refunds(
            &self.signer,
            req.leaves,
            signing_commitments,
            req.receiver_pubkey,
            self.network,
        )
        .await?;

        let transfer_id = TransferId::generate();
        let reason = if req.is_inbound_payment {
            Reason::Receive
        } else {
            Reason::Send
        };

        let request_data = InitiatePreimageSwapRequest {
            payment_hash: req.payment_hash.to_byte_array().to_vec(),
            reason: reason as i32,
            invoice_amount: Some(InvoiceAmount {
                invoice_amount_proof: Some(InvoiceAmountProof {
                    bolt11_invoice: req.invoice.to_string(),
                }),
                value_sats: req.invoice_amount_sats,
            }),
            transfer: Some(StartUserSignedTransferRequest {
                transfer_id: transfer_id.to_string(),
                owner_identity_public_key: self
                    .signer
                    .get_identity_public_key()?
                    .serialize()
                    .to_vec(),
                receiver_identity_public_key: req.receiver_pubkey.serialize().to_vec(),
                expiry_time: Default::default(),
                leaves_to_send: user_signed_refunds
                    .into_iter()
                    .map(|l| l.try_into())
                    .collect::<Result<Vec<_>, _>>()?,
            }),
            receiver_identity_public_key: req.receiver_pubkey.serialize().to_vec(),
            fee_sats: req.fee_sats,
        };

        let response = self
            .operator_pool
            .get_coordinator()
            .client
            .initiate_preimage_swap(request_data)
            .await?;
        Ok(response)
    }
}

fn get_invoice_amount_sats(
    invoice: &Bolt11Invoice,
    amount_to_send: Option<u64>,
) -> Result<u64, ServiceError> {
    let invoice_amount_sats = invoice
        .amount_milli_satoshis()
        .unwrap_or_default()
        .div_ceil(1000);
    let to_pay_sat = amount_to_send.unwrap_or(invoice_amount_sats);
    if to_pay_sat == 0 {
        return Err(ServiceError::ValidationError(
            "Amount must be provided for 0 amount invoice".to_string(),
        ));
    }
    if to_pay_sat < invoice_amount_sats {
        return Err(ServiceError::ValidationError(
            "Amount must not be less than the invoice amount".to_string(),
        ));
    }

    // it seems that spark currently don't support over payment
    if invoice_amount_sats > 0 && to_pay_sat > invoice_amount_sats {
        return Err(ServiceError::ValidationError(
            "Overpayments are not allowed".to_string(),
        ));
    }
    Ok(to_pay_sat)
}
