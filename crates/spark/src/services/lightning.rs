use crate::address::SparkAddress;
use crate::core::Network;
use crate::operator::OperatorPool;
use crate::operator::rpc::spark::{SecretShare, StorePreimageShareRequest};
use crate::services::{ServiceError, Transfer, TransferId, TransferObserver, TransferService};
use crate::signer::SecretToSplit;
use crate::ssp::{
    LightningReceiveRequestStatus, RequestLightningReceiveInput, RequestLightningSendInput,
    ServiceProvider,
};
use crate::utils::leaf_key_tweak::prepare_leaf_key_tweaks_to_send;
use crate::utils::preimage_swap::{SwapNodesForPreimageRequest, swap_nodes_for_preimage};
use crate::{signer::Signer, tree::TreeNode};
use bitcoin::hashes::{Hash, sha256};
use bitcoin::secp256k1::PublicKey;
use hex::ToHex;
use lightning_invoice::Bolt11Invoice;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use web_time::SystemTime;

use super::models::LightningSendRequestStatus;

const DEFAULT_RECEIVE_EXPIRY_SECS: u32 = 60 * 60 * 24 * 30; // 30 days
const DEFAULT_SEND_EXPIRY_SECS: u64 = 60 * 60 * 24 * 16; // 16 days
const RECEIVER_IDENTITY_PUBLIC_KEY_SHORT_CHANNEL_ID: u64 = 17592187092992000001;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum InvoiceDescription {
    Memo(String),
    DescriptionHash([u8; 32]),
}

impl InvoiceDescription {
    fn into_memo_and_description_hash(self) -> (Option<String>, Option<[u8; 32]>) {
        match self {
            InvoiceDescription::Memo(memo) => (Some(memo), None),
            InvoiceDescription::DescriptionHash(hash) => (None, Some(hash)),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LightningSendPayment {
    pub id: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub network: Network,
    pub encoded_invoice: String,
    pub fee_sat: u64,
    pub idempotency_key: String,
    pub status: LightningSendStatus,
    pub transfer_id: Option<TransferId>,
    pub payment_preimage: Option<String>,
}

#[derive(Debug, Copy, Clone, Deserialize, Serialize)]
pub enum LightningSendStatus {
    Created,
    UserTransferValidationFailed,
    LightningPaymentInitiated,
    LightningPaymentFailed,
    LightningPaymentSucceeded,
    PreimageProvided,
    PreimageProvidingFailed,
    TransferCompleted,
    TransferFailed,
    PendingUserSwapReturn,
    UserSwapReturned,
    UserSwapReturnFailed,
    RequestValidated,
    Unknown,
}

impl From<LightningSendRequestStatus> for LightningSendStatus {
    fn from(value: LightningSendRequestStatus) -> Self {
        match value {
            LightningSendRequestStatus::Created => LightningSendStatus::Created,
            LightningSendRequestStatus::RequestValidated => LightningSendStatus::RequestValidated,
            LightningSendRequestStatus::LightningPaymentInitiated => {
                LightningSendStatus::LightningPaymentInitiated
            }
            LightningSendRequestStatus::UserTransferValidationFailed => {
                LightningSendStatus::UserTransferValidationFailed
            }
            LightningSendRequestStatus::LightningPaymentFailed => {
                LightningSendStatus::LightningPaymentFailed
            }
            LightningSendRequestStatus::LightningPaymentSucceeded => {
                LightningSendStatus::LightningPaymentSucceeded
            }
            LightningSendRequestStatus::PreimageProvided => LightningSendStatus::PreimageProvided,
            LightningSendRequestStatus::PreimageProvidingFailed => {
                LightningSendStatus::PreimageProvidingFailed
            }
            LightningSendRequestStatus::TransferCompleted => LightningSendStatus::TransferCompleted,
            LightningSendRequestStatus::TransferFailed => LightningSendStatus::TransferFailed,
            LightningSendRequestStatus::PendingUserSwapReturn => {
                LightningSendStatus::PendingUserSwapReturn
            }
            LightningSendRequestStatus::UserSwapReturned => LightningSendStatus::UserSwapReturned,
            LightningSendRequestStatus::UserSwapReturnFailed => {
                LightningSendStatus::UserSwapReturnFailed
            }
            LightningSendRequestStatus::Unknown => LightningSendStatus::Unknown,
        }
    }
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
            fee_sat: value
                .fee
                .as_sats()
                .map_err(|_| ServiceError::Generic("Failed to parse fee".to_string()))?,
            idempotency_key: value.idempotency_key,
            status: value.status.into(),
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

pub struct PayLightningResult {
    pub transfer: Transfer,
    pub lightning_send_payment: Option<LightningSendPayment>,
    pub payment_hash: sha256::Hash,
}

pub struct LightningService {
    operator_pool: Arc<OperatorPool>,
    ssp_client: Arc<ServiceProvider>,
    network: Network,
    signer: Arc<dyn Signer>,
    transfer_service: Arc<TransferService>,
    split_secret_threshold: u32,
    transfer_observer: Option<Arc<dyn TransferObserver>>,
}

impl LightningService {
    pub fn new(
        operator_pool: Arc<OperatorPool>,
        ssp_client: Arc<ServiceProvider>,
        network: Network,
        signer: Arc<dyn Signer>,
        transfer_service: Arc<TransferService>,
        split_secret_threshold: u32,
        transfer_observer: Option<Arc<dyn TransferObserver>>,
    ) -> Self {
        LightningService {
            operator_pool,
            ssp_client,
            network,
            signer,
            transfer_service,
            split_secret_threshold,
            transfer_observer,
        }
    }

    pub async fn create_lightning_invoice(
        &self,
        amount_sats: u64,
        description: Option<InvoiceDescription>,
        preimage: Option<Vec<u8>>,
        expiry_secs: Option<u32>,
        include_spark_address: bool,
        identity_pubkey: Option<PublicKey>,
    ) -> Result<LightningReceivePayment, ServiceError> {
        self.create_lightning_invoice_inner(
            amount_sats,
            description,
            preimage,
            None,
            expiry_secs,
            include_spark_address,
            identity_pubkey,
        )
        .await
    }

    /// Creates a HODL lightning invoice using an externally-provided payment hash.
    /// No preimage shares are stored with operators, so the SSP will hold the HTLC
    /// until `provide_preimage` is called with the matching preimage, or the HTLC expires.
    ///
    /// Spark addresses are never included in HODL invoices because a direct Spark
    /// transfer would bypass the Lightning HTLC hold mechanism entirely.
    pub async fn create_hodl_lightning_invoice(
        &self,
        amount_sats: u64,
        description: Option<InvoiceDescription>,
        payment_hash: sha256::Hash,
        expiry_secs: Option<u32>,
        identity_pubkey: Option<PublicKey>,
    ) -> Result<LightningReceivePayment, ServiceError> {
        self.create_lightning_invoice_inner(
            amount_sats,
            description,
            None,
            Some(payment_hash),
            expiry_secs,
            false,
            identity_pubkey,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn create_lightning_invoice_inner(
        &self,
        amount_sats: u64,
        description: Option<InvoiceDescription>,
        preimage: Option<Vec<u8>>,
        external_payment_hash: Option<sha256::Hash>,
        expiry_secs: Option<u32>,
        include_spark_address: bool,
        identity_pubkey: Option<PublicKey>,
    ) -> Result<LightningReceivePayment, ServiceError> {
        // Validate expiry_secs does not exceed i32::MAX (server limitation)
        if let Some(expiry) = expiry_secs
            && expiry > i32::MAX as u32
        {
            return Err(ServiceError::ValidationError(format!(
                "expiry_secs {} exceeds maximum allowed value of {}",
                expiry,
                i32::MAX
            )));
        }

        let identity_pubkey = match identity_pubkey {
            Some(pk) => pk,
            None => self.signer.get_identity_public_key().await?,
        };

        let is_hodl = external_payment_hash.is_some();

        let (preimage, payment_hash) = if let Some(hash) = external_payment_hash {
            (None, hash)
        } else {
            let preimage = preimage.unwrap_or_else(|| {
                bitcoin::secp256k1::SecretKey::new(&mut OsRng)
                    .secret_bytes()
                    .to_vec()
            });
            let hash = sha256::Hash::hash(&preimage);
            (Some(preimage), hash)
        };

        let expiry = expiry_secs.unwrap_or(DEFAULT_RECEIVE_EXPIRY_SECS);

        let (memo, description_hash) = match description {
            Some(desc) => desc.into_memo_and_description_hash(),
            None => (None, None),
        };

        let invoice = self
            .ssp_client
            .request_lightning_receive(RequestLightningReceiveInput {
                receiver_identity_pubkey: Some(identity_pubkey.serialize().to_vec().encode_hex()),
                amount_sats,
                network: self.network.into(),
                payment_hash: payment_hash.encode_hex(),
                description_hash: description_hash.map(|h| h.encode_hex()),
                expiry_secs: Some(expiry.into()),
                memo,
                include_spark_address,
                spark_invoice: None,
            })
            .await?;
        let decoded_invoice = Bolt11Invoice::from_str(&invoice.invoice.encoded_invoice)
            .map_err(|err| ServiceError::InvoiceDecodingError(err.to_string()))?;

        // check if the spark address in the invoice matches the identity pubkey only
        if include_spark_address {
            let spark_address = self.extract_spark_address(&decoded_invoice);
            let Some(spark_address) = spark_address else {
                return Err(ServiceError::SSPswapError(
                    "Invalid invoice. Spark address not found".to_string(),
                ));
            };
            if spark_address.identity_public_key != identity_pubkey {
                return Err(ServiceError::SSPswapError(
                    "Invalid invoice. Spark address mismatch".to_string(),
                ));
            }
        }

        if !is_hodl {
            let preimage = preimage.ok_or_else(|| {
                ServiceError::Generic("preimage must be set for non-HODL invoices".to_string())
            })?;
            let shares = self
                .signer
                .split_secret_with_proofs(
                    &SecretToSplit::Preimage(preimage),
                    self.split_secret_threshold,
                    self.operator_pool.len(),
                )
                .await?;

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
                                invoice_string: invoice.invoice.encoded_invoice.clone(),
                                user_identity_public_key: identity_pubkey.serialize().to_vec(),
                            })
                    });

            futures::future::try_join_all(requests)
                .await
                .map_err(|e| ServiceError::PreimageShareStoreFailed(e.to_string()))?;
        }

        invoice.try_into()
    }

    pub async fn pay_lightning_invoice(
        &self,
        invoice: &str,
        amount_to_send: Option<u64>,
        leaves: &[TreeNode],
        transfer_id: Option<TransferId>,
    ) -> Result<PayLightningResult, ServiceError> {
        let ssp_identity_public_key = self.ssp_client.identity_public_key();
        let expiry_time = SystemTime::now() + Duration::from_secs(DEFAULT_SEND_EXPIRY_SECS);
        let unwrapped_transfer_id = match &transfer_id {
            Some(transfer_id) => transfer_id.clone(),
            None => TransferId::generate(),
        };

        // Decode invoice and validate amount
        let decoded_invoice = Bolt11Invoice::from_str(invoice)
            .map_err(|err| ServiceError::InvoiceDecodingError(err.to_string()))?;
        let amount_sats = get_invoice_amount_sats(&decoded_invoice, amount_to_send)?;
        let payment_hash = decoded_invoice.payment_hash();

        if let Some(transfer_observer) = &self.transfer_observer {
            transfer_observer
                .before_send_lightning_payment(&unwrapped_transfer_id, invoice, amount_sats)
                .await?;
        }

        // Prepare leaf tweaks
        let leaf_tweaks =
            prepare_leaf_key_tweaks_to_send(&self.signer, leaves.to_vec(), None).await?;

        let prepared_transfer_request = self
            .transfer_service
            .prepare_transfer_request(
                &unwrapped_transfer_id,
                &leaf_tweaks,
                &ssp_identity_public_key,
                Default::default(),
                Some(payment_hash),
                Some(expiry_time),
                None, // No adaptor public key for lightning transfers
            )
            .await?;

        let initiate_preimage_swap_res = swap_nodes_for_preimage(
            &self.operator_pool,
            &self.signer,
            self.network,
            SwapNodesForPreimageRequest {
                transfer_id: &unwrapped_transfer_id,
                leaves: &leaf_tweaks,
                receiver_pubkey: &ssp_identity_public_key,
                payment_hash,
                invoice_str: Some(invoice),
                amount_sats,
                fee_sats: 0, // TODO: this must use the estimated fee.
                is_inbound_payment: false,
                transfer_request: Some(prepared_transfer_request.transfer_request),
                expiry_time: &expiry_time,
            },
        )
        .await;
        let transfer: Transfer = match (&transfer_id, initiate_preimage_swap_res) {
            (_, Ok(initiate_preimage_swap)) => initiate_preimage_swap
                .transfer
                .ok_or(ServiceError::SSPswapError(
                    "Swap response did not contain a transfer".to_string(),
                ))?
                .try_into()?,
            (Some(transfer_id), Err(e)) => {
                let transfer = self
                    .transfer_service
                    .recover_transfer_on_rpc_connection_error(transfer_id, e)
                    .await?;
                return Ok(PayLightningResult {
                    transfer,
                    lightning_send_payment: None,
                    payment_hash: *payment_hash,
                });
            }
            (_, Err(e)) => return Err(e),
        };

        let mut lightning_send_payment: LightningSendPayment = self
            .ssp_client
            .request_lightning_send(RequestLightningSendInput {
                encoded_invoice: invoice.to_string(),
                idempotency_key: None,
                amount_sats: amount_to_send,
                user_outbound_transfer_external_id: Some(unwrapped_transfer_id.to_string()),
            })
            .await?
            .try_into()?;
        // If ssp doesn't return a transfer id, we use the transfer id from the initiate preimage swap
        if lightning_send_payment.transfer_id.is_none() {
            lightning_send_payment.transfer_id = Some(transfer.id.clone());
        }

        Ok(PayLightningResult {
            lightning_send_payment: Some(lightning_send_payment),
            transfer,
            payment_hash: *payment_hash,
        })
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
        if prefer_spark && let Some(receiver_address) = self.extract_spark_address(&decoded_invoice)
        {
            return Ok((to_pay_sat, Some(receiver_address)));
        }

        let fee_estimate = self
            .ssp_client
            .get_lightning_send_fee_estimate(invoice, Some(to_pay_sat))
            .await?;

        let fee_sat = fee_estimate
            .as_sats()
            .map_err(|_| ServiceError::Generic("Failed to parse fee".to_string()))?;
        if let Some(max_fee_sat) = max_fee_sat
            && fee_sat > max_fee_sat
        {
            return Err(ServiceError::ValidationError(format!(
                "Fee exceeds maximum allowed fee {fee_sat} > {max_fee_sat}",
            )));
        }

        Ok((fee_sat + to_pay_sat, None))
    }

    pub fn extract_spark_address_from_invoice(
        &self,
        invoice: &str,
    ) -> Result<Option<SparkAddress>, ServiceError> {
        let decoded_invoice = Bolt11Invoice::from_str(invoice)
            .map_err(|err| ServiceError::InvoiceDecodingError(err.to_string()))?;
        Ok(self.extract_spark_address(&decoded_invoice))
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

    Ok(to_pay_sat)
}
