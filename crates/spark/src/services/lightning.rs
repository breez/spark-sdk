use crate::address::SparkAddress;
use crate::core::Network;
use crate::operator::OperatorPool;
use crate::operator::rpc::spark::{
    InitiatePreimageSwapResponse, StartTransferRequest, StorePreimageShareV2Request,
};
use crate::services::{
    LeafKeyTweak, ServiceError, Transfer, TransferId, TransferObserver, TransferService,
};
use crate::signer::{
    OperatorRecipient, PrepareLightningReceiveRequest, PrepareTransferRequest, PreparedTransfer,
};
use crate::ssp::{
    LightningReceiveRequestStatus, RequestLightningReceiveInput, RequestLightningSendInput,
    ServiceProvider,
};
use crate::utils::leaf_key_tweak::prepare_leaf_key_tweaks_to_send;
use crate::utils::preimage_swap::{SwapNodesForPreimageRequest, swap_nodes_for_preimage};
use crate::{signer::SparkSigner, tree::TreeNode};
use bitcoin::hashes::{Hash, sha256};
use bitcoin::secp256k1::PublicKey;
use hex::ToHex;
use lightning_invoice::{Bolt11Invoice, Bolt11InvoiceDescriptionRef};
use platform_utils::time::SystemTime;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tracing::warn;

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

/// Verify an SSP-returned receive invoice commits to the parameters we requested.
///
/// A swapped payment hash is settled by whoever holds its preimage (the SSP),
/// leaving the receiver uncredited. The operators reject a hash that disagrees
/// with the invoice in `store_preimage_share_v2`, but HODL invoices store no
/// share and so reach no operator: there this is the only hash check. Amount,
/// network and description hash are validated nowhere else.
///
/// Memo and expiry only warn: neither is critical, so a future SSP bug degrades
/// them rather than blocking the receive outright.
fn validate_received_invoice(
    decoded_invoice: &Bolt11Invoice,
    payment_hash: sha256::Hash,
    network: bitcoin::Network,
    amount_sats: u64,
    description_hash: Option<[u8; 32]>,
    memo: Option<&str>,
    expiry_secs: u32,
) -> Result<(), ServiceError> {
    if *decoded_invoice.payment_hash() != payment_hash {
        return Err(ServiceError::ValidationError(
            "SSP invoice payment hash does not match the requested hash".to_string(),
        ));
    }
    if decoded_invoice.network() != network {
        return Err(ServiceError::ValidationError(
            "SSP invoice network does not match the requested network".to_string(),
        ));
    }
    // A zero request asks for an amountless invoice, so a fixed amount is as much
    // of a mismatch as the wrong one: it pins the payer to an amount we never asked for.
    let expected_amount_msats = (amount_sats > 0).then(|| amount_sats.saturating_mul(1000));
    if decoded_invoice.amount_milli_satoshis() != expected_amount_msats {
        return Err(ServiceError::ValidationError(
            "SSP invoice amount does not match the requested amount".to_string(),
        ));
    }
    if let Some(expected) = description_hash
        && !matches!(
            decoded_invoice.description(),
            Bolt11InvoiceDescriptionRef::Hash(h) if h.0.to_byte_array() == expected
        )
    {
        return Err(ServiceError::ValidationError(
            "SSP invoice description hash does not match the requested hash".to_string(),
        ));
    }
    // Compare the raw string: `UntrustedString`'s `Display` sanitizes control
    // characters, so it would report a mismatch for an identical memo.
    if let Some(memo) = memo
        && !matches!(
            decoded_invoice.description(),
            Bolt11InvoiceDescriptionRef::Direct(d) if d.as_inner().0 == memo
        )
    {
        warn!("SSP invoice description does not match the requested memo");
    }
    let expiry = decoded_invoice.expiry_time();
    if expiry != Duration::from_secs(u64::from(expiry_secs)) {
        warn!("SSP invoice expiry {expiry:?} does not match the requested {expiry_secs}s");
    }
    Ok(())
}

/// Verify the Spark address embedded in an SSP-returned receive invoice is ours,
/// and is there only if we asked for one.
///
/// A payer that finds an address here transfers to it directly and settles nothing
/// over Lightning, so an injected address is paid instead of us with no payment
/// hash, preimage or operator involved to catch it. The address rides in a route
/// hint, so an invoice correct in every other respect can still carry one.
fn validate_received_spark_address(
    spark_address: Option<&SparkAddress>,
    include_spark_address: bool,
    identity_pubkey: PublicKey,
) -> Result<(), ServiceError> {
    match (spark_address, include_spark_address) {
        (Some(_), false) => Err(ServiceError::ValidationError(
            "SSP invoice carries a Spark address that was not requested".to_string(),
        )),
        (Some(address), true) if address.identity_public_key != identity_pubkey => {
            Err(ServiceError::ValidationError(
                "SSP invoice Spark address does not match our identity".to_string(),
            ))
        }
        (None, true) => Err(ServiceError::ValidationError(
            "SSP invoice is missing the requested Spark address".to_string(),
        )),
        _ => Ok(()),
    }
}

pub struct LightningService {
    operator_pool: Arc<OperatorPool>,
    ssp_client: Arc<ServiceProvider>,
    network: Network,
    spark_signer: Arc<dyn SparkSigner>,
    transfer_service: Arc<TransferService>,
    split_secret_threshold: u32,
    transfer_observer: Option<Arc<dyn TransferObserver>>,
}

impl LightningService {
    pub fn new(
        operator_pool: Arc<OperatorPool>,
        ssp_client: Arc<ServiceProvider>,
        network: Network,
        spark_signer: Arc<dyn SparkSigner>,
        transfer_service: Arc<TransferService>,
        split_secret_threshold: u32,
        transfer_observer: Option<Arc<dyn TransferObserver>>,
    ) -> Self {
        LightningService {
            operator_pool,
            ssp_client,
            network,
            spark_signer,
            transfer_service,
            split_secret_threshold,
            transfer_observer,
        }
    }

    /// Builds the operator-recipient list for share-encryption from the pool.
    fn operator_recipients(&self) -> Vec<OperatorRecipient> {
        self.operator_pool
            .get_all_operators()
            .map(|op| OperatorRecipient {
                id: op.id,
                identifier: op.identifier,
                public_key: op.identity_public_key,
            })
            .collect()
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
            None => self.spark_signer.get_identity_public_key().await?,
        };

        let is_hodl = external_payment_hash.is_some();
        if !is_hodl && preimage.is_some() {
            return Err(ServiceError::InvalidInput(
                "external preimage is not supported; the signer generates it in-enclave"
                    .to_string(),
            ));
        }

        // For non-HODL invoices the signer generates the preimage in-enclave
        // (it never leaves), returning its hash plus the per-operator encrypted
        // preimage shares to store with the coordinator.
        let (payment_hash, prepared_receive) = match external_payment_hash {
            Some(hash) => (hash, None),
            None => {
                let prepared = self
                    .spark_signer
                    .prepare_lightning_receive(PrepareLightningReceiveRequest {
                        operator_recipients: self.operator_recipients(),
                        threshold: self.split_secret_threshold,
                    })
                    .await?;
                (
                    sha256::Hash::from_byte_array(prepared.payment_hash),
                    Some(prepared),
                )
            }
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
                memo: memo.clone(),
                include_spark_address,
                spark_invoice: None,
            })
            .await?;
        let decoded_invoice = Bolt11Invoice::from_str(&invoice.invoice.encoded_invoice)
            .map_err(|err| ServiceError::InvoiceDecodingError(err.to_string()))?;

        // Must precede the preimage share store and the return: a mismatch has to
        // abort the receive before the invoice can reach a payer.
        validate_received_invoice(
            &decoded_invoice,
            payment_hash,
            self.network.into(),
            amount_sats,
            description_hash,
            memo.as_deref(),
            expiry,
        )?;
        validate_received_spark_address(
            self.extract_spark_address(&decoded_invoice).as_ref(),
            include_spark_address,
            identity_pubkey,
        )?;

        if let Some(prepared) = prepared_receive {
            // The signer already Feldman-split the preimage and ECIES-encrypted
            // a share per operator; just forward them to the coordinator.
            let encrypted_preimage_shares: std::collections::HashMap<String, Vec<u8>> = prepared
                .operator_preimage_packages
                .into_iter()
                .map(|p| {
                    (
                        hex::encode(p.operator_identifier.serialize()),
                        p.encrypted_package,
                    )
                })
                .collect();

            self.operator_pool
                .get_coordinator()
                .client
                .store_preimage_share_v2(StorePreimageShareV2Request {
                    payment_hash: payment_hash.to_byte_array().to_vec(),
                    encrypted_preimage_shares,
                    threshold: self.split_secret_threshold,
                    invoice_string: invoice.invoice.encoded_invoice.clone(),
                    user_identity_public_key: identity_pubkey.serialize().to_vec(),
                })
                .await
                .map_err(|e: crate::operator::rpc::OperatorRpcError| {
                    ServiceError::PreimageShareStoreFailed(e.to_string())
                })?;
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
        let recover_on_error = transfer_id.is_some();
        let unwrapped_transfer_id = transfer_id.unwrap_or_else(TransferId::generate);
        self.send_lightning_inner(
            &unwrapped_transfer_id,
            leaves,
            invoice,
            amount_to_send,
            None,
            recover_on_error,
        )
        .await
    }

    async fn send_lightning_inner(
        &self,
        transfer_id: &TransferId,
        leaves: &[TreeNode],
        invoice: &str,
        amount_to_send: Option<u64>,
        prepared: Option<PreparedTransfer>,
        recover_on_error: bool,
    ) -> Result<PayLightningResult, ServiceError> {
        let ssp_identity_public_key = self.ssp_client.identity_public_key();
        let expiry_time = SystemTime::now() + Duration::from_secs(DEFAULT_SEND_EXPIRY_SECS);
        let decoded_invoice = Bolt11Invoice::from_str(invoice)
            .map_err(|err| ServiceError::InvoiceDecodingError(err.to_string()))?;
        let amount_sats = get_invoice_amount_sats(&decoded_invoice, amount_to_send)?;
        let payment_hash = *decoded_invoice.payment_hash();

        self.notify_before_send_lightning(transfer_id, invoice, amount_sats)
            .await?;

        let leaf_key_tweaks = prepare_leaf_key_tweaks_to_send(leaves.to_vec());

        let prepared_transfer_request = match prepared {
            Some(prepared) => {
                self.transfer_service
                    .assemble_transfer_request_with_prepared(
                        transfer_id,
                        &leaf_key_tweaks,
                        &ssp_identity_public_key,
                        Some(&payment_hash),
                        Some(expiry_time),
                        None,
                        prepared,
                    )
                    .await?
            }
            None => {
                self.transfer_service
                    .prepare_transfer_request(
                        transfer_id,
                        &leaf_key_tweaks,
                        &ssp_identity_public_key,
                        Some(&payment_hash),
                        Some(expiry_time),
                        None,
                    )
                    .await?
            }
        };

        let initiate_preimage_swap_res = self
            .initiate_lightning_preimage_swap(
                transfer_id,
                &leaf_key_tweaks,
                &payment_hash,
                invoice,
                amount_sats,
                &expiry_time,
                prepared_transfer_request.transfer_request,
            )
            .await;

        let transfer: Transfer = match initiate_preimage_swap_res {
            Ok(initiate_preimage_swap) => transfer_from_preimage_swap(initiate_preimage_swap)?,
            Err(e) if recover_on_error => {
                return self
                    .recovered_lightning_result(transfer_id, e, payment_hash)
                    .await;
            }
            Err(e) => return Err(e),
        };

        self.request_lightning_send_result(
            transfer,
            transfer_id,
            invoice,
            amount_to_send,
            payment_hash,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn initiate_lightning_preimage_swap(
        &self,
        transfer_id: &TransferId,
        leaf_tweaks: &[LeafKeyTweak],
        payment_hash: &sha256::Hash,
        invoice: &str,
        amount_sats: u64,
        expiry_time: &SystemTime,
        transfer_request: StartTransferRequest,
    ) -> Result<InitiatePreimageSwapResponse, ServiceError> {
        let receiver_pubkey = self.ssp_client.identity_public_key();
        swap_nodes_for_preimage(
            &self.operator_pool,
            &self.spark_signer,
            self.network,
            SwapNodesForPreimageRequest {
                transfer_id,
                leaves: leaf_tweaks,
                receiver_pubkey: &receiver_pubkey,
                payment_hash,
                invoice_str: Some(invoice),
                amount_sats,
                fee_sats: 0,
                is_inbound_payment: false,
                transfer_request: Some(transfer_request),
                expiry_time,
            },
        )
        .await
    }

    async fn recovered_lightning_result(
        &self,
        transfer_id: &TransferId,
        error: ServiceError,
        payment_hash: sha256::Hash,
    ) -> Result<PayLightningResult, ServiceError> {
        let transfer = self
            .transfer_service
            .recover_transfer_on_rpc_connection_error(transfer_id, error)
            .await?;
        Ok(PayLightningResult {
            transfer,
            lightning_send_payment: None,
            payment_hash,
        })
    }

    async fn request_lightning_send_result(
        &self,
        transfer: Transfer,
        transfer_id: &TransferId,
        invoice: &str,
        amount_to_send: Option<u64>,
        payment_hash: sha256::Hash,
    ) -> Result<PayLightningResult, ServiceError> {
        let mut lightning_send_payment: LightningSendPayment = self
            .ssp_client
            .request_lightning_send(RequestLightningSendInput {
                encoded_invoice: invoice.to_string(),
                idempotency_key: None,
                amount_sats: amount_to_send,
                user_outbound_transfer_external_id: Some(transfer_id.to_string()),
            })
            .await?
            .try_into()?;
        if lightning_send_payment.transfer_id.is_none() {
            lightning_send_payment.transfer_id = Some(transfer.id.clone());
        }

        Ok(PayLightningResult {
            lightning_send_payment: Some(lightning_send_payment),
            transfer,
            payment_hash,
        })
    }

    pub fn prepare_lightning_send(
        &self,
        leaves: &[TreeNode],
        transfer_id: Option<TransferId>,
    ) -> PrepareTransferRequest {
        let ssp_identity_public_key = self.ssp_client.identity_public_key();
        let transfer_id = transfer_id.unwrap_or_else(TransferId::generate);
        self.transfer_service.build_transfer_approval_request(
            &transfer_id,
            leaves,
            &ssp_identity_public_key,
        )
    }

    async fn notify_before_send_lightning(
        &self,
        transfer_id: &TransferId,
        invoice: &str,
        amount_sats: u64,
    ) -> Result<(), ServiceError> {
        if let Some(transfer_observer) = &self.transfer_observer {
            transfer_observer
                .before_send_lightning_payment(transfer_id, invoice, amount_sats)
                .await?;
        }
        Ok(())
    }

    pub async fn submit_lightning_send(
        &self,
        transfer_id: TransferId,
        leaves: &[TreeNode],
        invoice: &str,
        amount_to_send: Option<u64>,
        approved_transfer: PreparedTransfer,
    ) -> Result<PayLightningResult, ServiceError> {
        self.send_lightning_inner(
            &transfer_id,
            leaves,
            invoice,
            amount_to_send,
            Some(approved_transfer),
            true,
        )
        .await
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

fn transfer_from_preimage_swap(
    response: InitiatePreimageSwapResponse,
) -> Result<Transfer, ServiceError> {
    response
        .transfer
        .ok_or(ServiceError::SSPswapError(
            "Swap response did not contain a transfer".to_string(),
        ))?
        .try_into()
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

#[cfg(test)]
mod validate_received_invoice_tests {
    use super::{
        ServiceError, SparkAddress, validate_received_invoice, validate_received_spark_address,
    };
    use bitcoin::hashes::{Hash, sha256};
    use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
    use lightning_invoice::{Bolt11Invoice, Currency, InvoiceBuilder, PaymentSecret};

    const EXPIRY_SECS: u32 = 3600;

    fn pubkey(byte: u8) -> PublicKey {
        let secp = Secp256k1::new();
        SecretKey::from_slice(&[byte; 32])
            .unwrap()
            .public_key(&secp)
    }

    fn hash(byte: u8) -> sha256::Hash {
        sha256::Hash::from_byte_array([byte; 32])
    }

    fn invoice(
        currency: Currency,
        payment_hash: sha256::Hash,
        amount_msat: Option<u64>,
        desc_hash: Option<sha256::Hash>,
    ) -> Bolt11Invoice {
        let secp = Secp256k1::new();
        let key = SecretKey::from_slice(&[0x11; 32]).unwrap();
        let sign = |m: &_| secp.sign_ecdsa_recoverable(m, &key);
        // `InvoiceBuilder` encodes description kind and amount in its type, so no
        // shared base builder is possible and each arm chains in full. Only one arm
        // runs, so `sign` moving into each is fine.
        match (desc_hash, amount_msat) {
            (Some(dh), Some(amt)) => InvoiceBuilder::new(currency)
                .description_hash(dh)
                .payment_hash(payment_hash)
                .payment_secret(PaymentSecret([0x22; 32]))
                .duration_since_epoch(std::time::Duration::from_secs(1_700_000_000))
                .min_final_cltv_expiry_delta(144)
                .expiry_time(std::time::Duration::from_secs(EXPIRY_SECS as u64))
                .amount_milli_satoshis(amt)
                .build_signed(sign)
                .unwrap(),
            (Some(dh), None) => InvoiceBuilder::new(currency)
                .description_hash(dh)
                .payment_hash(payment_hash)
                .payment_secret(PaymentSecret([0x22; 32]))
                .duration_since_epoch(std::time::Duration::from_secs(1_700_000_000))
                .min_final_cltv_expiry_delta(144)
                .expiry_time(std::time::Duration::from_secs(EXPIRY_SECS as u64))
                .build_signed(sign)
                .unwrap(),
            (None, Some(amt)) => InvoiceBuilder::new(currency)
                .description("test".to_string())
                .payment_hash(payment_hash)
                .payment_secret(PaymentSecret([0x22; 32]))
                .duration_since_epoch(std::time::Duration::from_secs(1_700_000_000))
                .min_final_cltv_expiry_delta(144)
                .expiry_time(std::time::Duration::from_secs(EXPIRY_SECS as u64))
                .amount_milli_satoshis(amt)
                .build_signed(sign)
                .unwrap(),
            (None, None) => InvoiceBuilder::new(currency)
                .description("test".to_string())
                .payment_hash(payment_hash)
                .payment_secret(PaymentSecret([0x22; 32]))
                .duration_since_epoch(std::time::Duration::from_secs(1_700_000_000))
                .min_final_cltv_expiry_delta(144)
                .expiry_time(std::time::Duration::from_secs(EXPIRY_SECS as u64))
                .build_signed(sign)
                .unwrap(),
        }
    }

    /// Assert the validation failed, and specifically on the guard identified by
    /// `needle`. Every check returns `ValidationError`, so asserting the message
    /// keyword proves the intended guard fired rather than an unrelated one.
    fn assert_rejected(result: Result<(), ServiceError>, needle: &str) {
        match result {
            Err(ServiceError::ValidationError(msg)) => assert!(
                msg.contains(needle),
                "expected a `{needle}` validation error, got: {msg}"
            ),
            other => panic!("expected a `{needle}` ValidationError, got: {other:?}"),
        }
    }

    #[test]
    fn accepts_matching_invoice() {
        let inv = invoice(Currency::Regtest, hash(1), Some(50_000), None);
        assert!(
            validate_received_invoice(
                &inv,
                hash(1),
                bitcoin::Network::Regtest,
                50,
                None,
                None,
                EXPIRY_SECS
            )
            .is_ok()
        );
    }

    #[test]
    fn rejects_payment_hash_mismatch() {
        let inv = invoice(Currency::Regtest, hash(1), Some(50_000), None);
        assert_rejected(
            validate_received_invoice(
                &inv,
                hash(2),
                bitcoin::Network::Regtest,
                50,
                None,
                None,
                EXPIRY_SECS,
            ),
            "payment hash",
        );
    }

    #[test]
    fn rejects_network_mismatch() {
        let inv = invoice(Currency::Regtest, hash(1), Some(50_000), None);
        assert_rejected(
            validate_received_invoice(
                &inv,
                hash(1),
                bitcoin::Network::Bitcoin,
                50,
                None,
                None,
                EXPIRY_SECS,
            ),
            "network",
        );
    }

    #[test]
    fn rejects_amount_mismatch() {
        let inv = invoice(Currency::Regtest, hash(1), Some(50_000), None);
        assert_rejected(
            validate_received_invoice(
                &inv,
                hash(1),
                bitcoin::Network::Regtest,
                60,
                None,
                None,
                EXPIRY_SECS,
            ),
            "amount",
        );
    }

    #[test]
    fn accepts_amountless_invoice_when_amountless_requested() {
        let inv = invoice(Currency::Regtest, hash(1), None, None);
        assert!(
            validate_received_invoice(
                &inv,
                hash(1),
                bitcoin::Network::Regtest,
                0,
                None,
                None,
                EXPIRY_SECS
            )
            .is_ok()
        );
    }

    #[test]
    fn accepts_matching_description_hash() {
        let dh = hash(9);
        let inv = invoice(Currency::Regtest, hash(1), Some(50_000), Some(dh));
        assert!(
            validate_received_invoice(
                &inv,
                hash(1),
                bitcoin::Network::Regtest,
                50,
                Some(dh.to_byte_array()),
                None,
                EXPIRY_SECS,
            )
            .is_ok()
        );
    }

    #[test]
    fn rejects_description_hash_mismatch() {
        let inv = invoice(Currency::Regtest, hash(1), Some(50_000), Some(hash(9)));
        assert_rejected(
            validate_received_invoice(
                &inv,
                hash(1),
                bitcoin::Network::Regtest,
                50,
                Some(hash(8).to_byte_array()),
                None,
                EXPIRY_SECS,
            ),
            "description hash",
        );
    }

    #[test]
    fn rejects_direct_description_when_hash_requested() {
        // The invoice carries a plain-text description, but we requested a
        // description hash: the wrong description *type* must be rejected too.
        let inv = invoice(Currency::Regtest, hash(1), Some(50_000), None);
        assert_rejected(
            validate_received_invoice(
                &inv,
                hash(1),
                bitcoin::Network::Regtest,
                50,
                Some(hash(9).to_byte_array()),
                None,
                EXPIRY_SECS,
            ),
            "description hash",
        );
    }

    #[test]
    fn rejects_fixed_amount_when_amountless_requested() {
        let inv = invoice(Currency::Regtest, hash(1), Some(50_000), None);
        assert_rejected(
            validate_received_invoice(
                &inv,
                hash(1),
                bitcoin::Network::Regtest,
                0,
                None,
                None,
                EXPIRY_SECS,
            ),
            "amount",
        );
    }

    /// Memo and expiry are advisory, so a mismatch must still return `Ok`.
    #[test]
    fn memo_and_expiry_mismatch_are_accepted() {
        // `invoice` builds a "test" description and an `EXPIRY_SECS` expiry.
        let inv = invoice(Currency::Regtest, hash(1), Some(50_000), None);
        assert!(
            validate_received_invoice(
                &inv,
                hash(1),
                bitcoin::Network::Regtest,
                50,
                None,
                Some("not the memo we asked for"),
                EXPIRY_SECS + 1,
            )
            .is_ok()
        );
    }

    fn spark_address(byte: u8) -> SparkAddress {
        SparkAddress::new(pubkey(byte), crate::core::Network::Regtest, None)
    }

    #[test]
    fn accepts_requested_spark_address_matching_identity() {
        assert!(
            validate_received_spark_address(Some(&spark_address(0x11)), true, pubkey(0x11)).is_ok()
        );
    }

    #[test]
    fn rejects_requested_spark_address_for_another_identity() {
        assert_rejected(
            validate_received_spark_address(Some(&spark_address(0x22)), true, pubkey(0x11)),
            "does not match our identity",
        );
    }

    #[test]
    fn rejects_missing_spark_address_when_requested() {
        assert_rejected(
            validate_received_spark_address(None, true, pubkey(0x11)),
            "missing the requested Spark address",
        );
    }

    /// A payer transfers straight to an address found in the invoice, so one we
    /// never asked for is a redirect of the funds even when it names our identity.
    #[test]
    fn rejects_unrequested_spark_address() {
        assert_rejected(
            validate_received_spark_address(Some(&spark_address(0x22)), false, pubkey(0x11)),
            "not requested",
        );
        assert_rejected(
            validate_received_spark_address(Some(&spark_address(0x11)), false, pubkey(0x11)),
            "not requested",
        );
    }

    #[test]
    fn accepts_absent_spark_address_when_not_requested() {
        assert!(validate_received_spark_address(None, false, pubkey(0x11)).is_ok());
    }
}
