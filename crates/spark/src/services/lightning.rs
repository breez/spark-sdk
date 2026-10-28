use crate::address::SparkAddress;
use crate::core::Network;
use crate::operator::OperatorPool;
use crate::operator::rpc::spark::initiate_preimage_swap_request::Reason as PreimageSwapReason;
use crate::operator::rpc::spark::{
    InitiatePreimageSwapResponse, InitiatePreimageSwapV4Request, InvoiceAmount, InvoiceAmountProof,
    SecretShare, StartTransferRequest, StorePreimageShareV2Request, TransferManifest,
};
use crate::quote::{
    ManifestDenomination, QuoteError, ReceiveAmountBasis, ReceiveQuoteFee, SendManifestBucket,
    manifest_denomination, manifest_gross_sats, manifest_send_buckets, receive_attestation_digest,
    receive_quote_fee, send_issuance_digest, serialized_transfer_manifest_hash,
    validate_quoted_receive_manifest, validate_quoted_send_manifest, verify_identity_signature,
};
use crate::services::{
    LeafKeyTweak, Preimage, ServiceError, Transfer, TransferId, TransferObserver, TransferService,
    TransferStatus, TransferType,
};
use crate::signer::{
    OperatorRecipient, PrepareLightningReceiveRequest, PrepareTransferRequest, PreparedTransfer,
    QuoteSignatureKind, SecretToSplit, SignQuoteRequest, Signer, SparkSigner,
};
use crate::ssp::{
    CommittedReceiveQuote, CommittedSendQuote, IssuedQuote, LightningReceiveRequestStatus,
    RequestLightningReceiveInput, RequestLightningSendInput, ServiceProvider,
    SparkFeeAttributionStatus,
};
use crate::utils::preimage_swap::{SwapNodesForPreimageRequest, swap_nodes_for_preimage};
use bitcoin::hashes::{Hash, sha256};
use bitcoin::secp256k1::PublicKey;
use hex::ToHex;
use lightning_invoice::{Bolt11Invoice, Bolt11InvoiceDescriptionRef};
use platform_utils::time::{SystemTime, UNIX_EPOCH};
use prost::Message as _;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, warn};

use super::models::LightningSendRequestStatus;

const DEFAULT_RECEIVE_EXPIRY_SECS: u32 = 60 * 60 * 24 * 30; // 30 days
const DEFAULT_SEND_EXPIRY_SECS: u64 = 60 * 60 * 24 * 16; // 16 days
const RECEIVER_IDENTITY_PUBLIC_KEY_SHORT_CHANNEL_ID: u64 = 17592187092992000001;

/// Splits `preimage` into verifiable secret shares, encrypts each for its operator,
/// and stores them at the coordinator under its payment hash. Storing the shares is
/// what makes an invoice a normal (non-HODL) invoice: with the shares present, the
/// operators reconstruct the preimage during the receiver-side `Reason::Receive`
/// preimage swap and return it atomically with the leaf transfer (a HODL invoice
/// has no shares, so the operators front the leaves and hold for the preimage).
///
/// `invoice_string` is the bolt11 the operators validate the swap amount against;
/// `identity_pubkey` is the share owner (the receiver), which the operators check
/// against the swap's receiver.
pub async fn store_preimage_shares(
    operator_pool: &OperatorPool,
    signer: &Arc<dyn Signer>,
    split_secret_threshold: u32,
    preimage: &Preimage,
    invoice_string: String,
    identity_pubkey: PublicKey,
) -> Result<(), ServiceError> {
    let shares = signer
        .split_secret_with_proofs(
            &SecretToSplit::Preimage(preimage.to_vec()),
            split_secret_threshold,
            operator_pool.len(),
        )
        .await?;

    // Build the encrypted preimage shares map for the V2 endpoint: one ECIES blob
    // per operator, keyed by that operator's identifier.
    let mut encrypted_shares: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for (operator, share) in operator_pool.get_all_operators().zip(shares) {
        let secret_share_proto = SecretShare {
            secret_share: share.secret_share.share.to_bytes().to_vec(),
            proofs: share
                .proofs
                .iter()
                .map(|p| p.to_sec1_bytes().to_vec())
                .collect(),
        };
        let proto_bytes = secret_share_proto.encode_to_vec();
        let public_key_bytes = operator.identity_public_key.serialize_uncompressed();
        let encrypted = ::utils::ecies::encrypt(&public_key_bytes, &proto_bytes)
            .map_err(|e| ServiceError::Generic(format!("ECIES encryption failed: {e}")))?;
        let operator_identifier = hex::encode(operator.identifier.serialize());
        encrypted_shares.insert(operator_identifier, encrypted);
    }

    operator_pool
        .get_coordinator()
        .client
        .store_preimage_share_v2(StorePreimageShareV2Request {
            payment_hash: preimage.compute_hash().to_byte_array().to_vec(),
            encrypted_preimage_shares: encrypted_shares.into_iter().collect(),
            threshold: split_secret_threshold,
            invoice_string,
            user_identity_public_key: identity_pubkey.serialize().to_vec(),
        })
        .await
        .map_err(|e: crate::operator::rpc::OperatorRpcError| {
            ServiceError::PreimageShareStoreFailed(e.to_string())
        })?;
    Ok(())
}

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
    /// The markup of the fee quote the invoice was issued under. Unset on an
    /// unquoted invoice.
    #[serde(default)]
    pub quote_fee: Option<ReceiveQuoteFee>,
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
            quote_fee: None,
        })
    }
}

/// A receive quote that passed validation, to be attested once the invoice's
/// payment hash exists.
struct AcceptedReceiveQuote {
    issued_quote: IssuedQuote,
    /// What the invoice must be issued for: the quote's gross, or zero on an
    /// amountless one.
    invoiced_sats: u64,
    fee: ReceiveQuoteFee,
}

/// What a send quote prices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendQuoteAmount {
    /// The invoice's own amount, the fees on top.
    Invoice,
    /// What an amountless invoice is paid, the fees on top.
    Payment(u64),
    /// What the wallet spends in all. An amountless invoice is paid what is
    /// left after the fees. An invoice with its own amount is paid that, and
    /// the fees take the rest.
    Total(u64),
}

/// A send quote checked against the invoice it pays, ready to fund.
#[derive(Debug, Clone)]
pub struct QuotedLightningSend {
    pub issued_quote: IssuedQuote,
    /// The transfer the quote is single-use for: the send is keyed on it.
    pub transfer_id: TransferId,
    pub payment_hash: sha256::Hash,
    /// The payment the quote prices, which its signature binds.
    pub payment_msats: u64,
    /// `payment_msats` in sats, rounded up.
    pub amount_sats: u64,
    /// The amount of an amountless invoice, which the SSP is told again when
    /// the send is committed. `None` for an invoice that carries its own.
    pub amount_to_send: Option<u64>,
    /// What the wallet parts with: the invoice amount plus every fee, partner
    /// markup included.
    pub total_sats: u64,
    /// What to fund for each receiver, in manifest order.
    pub buckets: Vec<SendManifestBucket>,
    /// When the quote stops being committable, in seconds since the epoch.
    pub expires_at: u64,
    manifest: TransferManifest,
}

impl QuotedLightningSend {
    /// The digest this wallet signs as the quote's sole sender.
    pub fn manifest_hash(&self) -> Result<[u8; 32], QuoteError> {
        serialized_transfer_manifest_hash(&self.issued_quote.serialized_manifest)
    }
}

pub struct PayLightningResult {
    pub transfer: Transfer,
    pub lightning_send_payment: LightningSendPayment,
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

    /// Creates a Lightning invoice paying `identity_pubkey` (this wallet when
    /// unset).
    ///
    /// With `quote_basis` set, the invoice is first priced by an SSP fee quote
    /// whose `amount_sats` is read on that basis, and this wallet attests to the
    /// quote. Fee attribution is best-effort: if the quote cannot be had or
    /// does not check out, the invoice is created unquoted.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_lightning_invoice(
        &self,
        amount_sats: u64,
        description: Option<InvoiceDescription>,
        preimage: Option<Vec<u8>>,
        expiry_secs: Option<u32>,
        include_spark_address: bool,
        identity_pubkey: Option<PublicKey>,
        quote_basis: Option<ReceiveAmountBasis>,
    ) -> Result<LightningReceivePayment, ServiceError> {
        self.create_lightning_invoice_inner(
            amount_sats,
            description,
            preimage,
            None,
            expiry_secs,
            include_spark_address,
            identity_pubkey,
            quote_basis,
        )
        .await
    }

    /// Creates a HODL lightning invoice using an externally-provided payment hash.
    /// No preimage shares are stored with operators, so the SSP will hold the HTLC
    /// until `provide_preimage` is called with the matching preimage, or the HTLC expires.
    ///
    /// Spark addresses are never included in HODL invoices because a direct Spark
    /// transfer would bypass the Lightning HTLC hold mechanism entirely.
    ///
    /// The invoice is always this wallet's: the wallet that attests a quoted
    /// receive is the one that releases its preimage, so it must hold it.
    ///
    /// With `quote_basis` set, the invoice is priced by a fee quote attested over
    /// `payment_hash`, as [`create_lightning_invoice`](Self::create_lightning_invoice) does.
    pub async fn create_hodl_lightning_invoice(
        &self,
        amount_sats: u64,
        description: Option<InvoiceDescription>,
        payment_hash: sha256::Hash,
        expiry_secs: Option<u32>,
        quote_basis: Option<ReceiveAmountBasis>,
    ) -> Result<LightningReceivePayment, ServiceError> {
        self.create_lightning_invoice_inner(
            amount_sats,
            description,
            None,
            Some(payment_hash),
            expiry_secs,
            false,
            None,
            quote_basis,
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
        quote_basis: Option<ReceiveAmountBasis>,
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

        let own_identity_pubkey = self.spark_signer.get_identity_public_key().await?;
        let identity_pubkey = identity_pubkey.unwrap_or(own_identity_pubkey);

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

        let input = RequestLightningReceiveInput {
            receiver_identity_pubkey: Some(identity_pubkey.serialize().to_vec().encode_hex()),
            amount_sats,
            network: self.network.into(),
            payment_hash: payment_hash.encode_hex(),
            description_hash: description_hash.map(|h| h.encode_hex()),
            expiry_secs: Some(expiry.into()),
            memo: memo.clone(),
            include_spark_address,
            spark_invoice: None,
        };

        // A Spark fallback address would let the payer settle without the fee,
        // so a fee-bearing quote is never combined with one.
        let quote = match quote_basis {
            Some(basis) if !include_spark_address => {
                self.accepted_receive_quote(
                    amount_sats,
                    basis,
                    identity_pubkey,
                    own_identity_pubkey,
                )
                .await
            }
            _ => None,
        };
        let (invoice, quote) = match quote {
            Some(quote) => match self
                .request_quoted_lightning_receive(input.clone(), &quote, payment_hash)
                .await
            {
                Ok(invoice) => (invoice, Some(quote)),
                Err(e) => {
                    warn!("Receiving without a fee quote, as committing it failed: {e}");
                    (
                        self.ssp_client.request_lightning_receive(input).await?,
                        None,
                    )
                }
            },
            None => (
                self.ssp_client.request_lightning_receive(input).await?,
                None,
            ),
        };
        let invoiced_sats = quote
            .as_ref()
            .map_or(amount_sats, |quote| quote.invoiced_sats);
        let decoded_invoice = Bolt11Invoice::from_str(&invoice.invoice.encoded_invoice)
            .map_err(|err| ServiceError::InvoiceDecodingError(err.to_string()))?;

        // Must precede the preimage share store and the return: a mismatch has to
        // abort the receive before the invoice can reach a payer.
        validate_received_invoice(
            &decoded_invoice,
            payment_hash,
            self.network.into(),
            invoiced_sats,
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
                    // The attestor of a quoted receive owns its preimage shares,
                    // also when it attests for another payee.
                    user_identity_public_key: if quote.is_some() {
                        own_identity_pubkey
                    } else {
                        identity_pubkey
                    }
                    .serialize()
                    .to_vec(),
                })
                .await
                .map_err(|e: crate::operator::rpc::OperatorRpcError| {
                    ServiceError::PreimageShareStoreFailed(e.to_string())
                })?;
        }

        let mut payment: LightningReceivePayment = invoice.try_into()?;
        payment.quote_fee = quote.map(|quote| quote.fee);
        Ok(payment)
    }

    /// A receive quote for `amount_sats` read on `basis`, checked before this
    /// wallet attests to it. `None`, logged, when there is none to be had or
    /// it does not describe the receive asked for: attribution must never
    /// block a receive.
    async fn accepted_receive_quote(
        &self,
        amount_sats: u64,
        basis: ReceiveAmountBasis,
        payee: PublicKey,
        attestor: PublicKey,
    ) -> Option<AcceptedReceiveQuote> {
        match self
            .try_receive_quote(amount_sats, basis, payee, attestor)
            .await
        {
            Ok(quote) => Some(quote),
            Err(e) => {
                warn!("Receiving without a fee quote: {e}");
                None
            }
        }
    }

    async fn try_receive_quote(
        &self,
        amount_sats: u64,
        basis: ReceiveAmountBasis,
        payee: PublicKey,
        attestor: PublicKey,
    ) -> Result<AcceptedReceiveQuote, ServiceError> {
        let quote = self
            .ssp_client
            .lightning_receive_quote(
                self.network.into(),
                amount_sats,
                basis,
                (payee != attestor).then_some(payee),
            )
            .await?;
        if quote.attribution_status != SparkFeeAttributionStatus::Attributed {
            debug!(
                "Receive quote carries no partner fee: {:?}",
                quote.attribution_status
            );
        }
        let manifest = TransferManifest::decode(quote.issued_quote.serialized_manifest.as_slice())
            .map_err(|e| QuoteError::invalid(format!("undecodable manifest: {e}")))?;
        self.check_quote_terms(&manifest)?;
        validate_quoted_receive_manifest(&manifest, amount_sats, basis, &payee, &attestor)?;
        let invoiced_sats = match manifest_denomination(&manifest) {
            ManifestDenomination::Sats => manifest_gross_sats(&manifest)?,
            ManifestDenomination::Bps => 0,
        };
        Ok(AcceptedReceiveQuote {
            issued_quote: quote.issued_quote,
            invoiced_sats,
            fee: receive_quote_fee(&manifest, &payee)?,
        })
    }

    /// Requests the invoice under `quote`, attesting to it for `payment_hash`.
    async fn request_quoted_lightning_receive(
        &self,
        mut input: RequestLightningReceiveInput,
        quote: &AcceptedReceiveQuote,
        payment_hash: sha256::Hash,
    ) -> Result<crate::ssp::LightningReceiveRequest, ServiceError> {
        // The SSP refuses a request naming anything but the manifest's gross.
        input.amount_sats = quote.invoiced_sats;
        let digest = receive_attestation_digest(
            &quote.issued_quote.serialized_manifest,
            &payment_hash.to_byte_array(),
        )?;
        let signed = self
            .spark_signer
            .sign_quote(SignQuoteRequest {
                kind: QuoteSignatureKind::ReceiveAttestation,
                digest,
            })
            .await?;
        Ok(self
            .ssp_client
            .request_quoted_lightning_receive(
                input,
                &CommittedReceiveQuote {
                    issued_quote: quote.issued_quote.clone(),
                    attestor_signature: signed.signature.serialize_der().to_vec(),
                },
            )
            .await?)
    }

    /// Refuses a quote for another network, or one already expired.
    fn check_quote_terms(&self, manifest: &TransferManifest) -> Result<(), QuoteError> {
        if manifest.network != i32::from(self.network.to_proto_network()) {
            return Err(QuoteError::invalid(format!(
                "quote is for network {}, not {}",
                manifest.network, self.network
            )));
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if quote_expiry(manifest)? <= now {
            return Err(QuoteError::invalid("quote has expired"));
        }
        Ok(())
    }

    pub async fn pay_lightning_invoice(
        &self,
        invoice: &str,
        amount_to_send: Option<u64>,
        leaves: &[LeafKeyTweak],
        transfer_id: Option<TransferId>,
    ) -> Result<PayLightningResult, ServiceError> {
        let unwrapped_transfer_id = transfer_id.unwrap_or_else(TransferId::generate);
        self.send_lightning_inner(
            &unwrapped_transfer_id,
            leaves,
            invoice,
            amount_to_send,
            None,
        )
        .await
    }

    async fn send_lightning_inner(
        &self,
        transfer_id: &TransferId,
        leaves: &[LeafKeyTweak],
        invoice: &str,
        amount_to_send: Option<u64>,
        prepared: Option<PreparedTransfer>,
    ) -> Result<PayLightningResult, ServiceError> {
        let ssp_identity_public_key = self.ssp_client.identity_public_key();
        let expiry_time = SystemTime::now() + Duration::from_secs(DEFAULT_SEND_EXPIRY_SECS);
        let decoded_invoice = Bolt11Invoice::from_str(invoice)
            .map_err(|err| ServiceError::InvoiceDecodingError(err.to_string()))?;
        let amount_sats = get_invoice_amount_sats(&decoded_invoice, amount_to_send)?;
        let payment_hash = *decoded_invoice.payment_hash();

        self.notify_before_send_lightning(transfer_id, invoice, amount_sats)
            .await?;

        let prepared_transfer_request = match prepared {
            Some(prepared) => {
                self.transfer_service
                    .assemble_transfer_request_with_prepared(
                        transfer_id,
                        leaves,
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
                        leaves,
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
                &payment_hash,
                invoice,
                amount_sats,
                prepared_transfer_request.transfer_request,
            )
            .await;

        let transfer: Transfer = match initiate_preimage_swap_res {
            Ok(initiate_preimage_swap) => transfer_from_preimage_swap(initiate_preimage_swap)?,
            // The preimage swap can commit server-side and still lose its response. Bind
            // the recovered transfer rather than returning it: only request_lightning_send
            // finishes the send, and nothing else advances a WaitingForPreimage transfer
            // before it expires. Recover even for an id generated here: the query also
            // matches our own sender key, so it resolves only to this call's transfer,
            // and it is the last chance to finish the send. The operator commit is the
            // first effectful step, and the invoice it pays exists nowhere server-side.
            Err(e) => {
                self.transfer_service
                    .recover_committed_transfer(transfer_id, e)
                    .await?
            }
        };

        self.request_lightning_send_result(
            transfer,
            transfer_id,
            invoice,
            amount_to_send,
            payment_hash,
            None,
        )
        .await
    }

    /// Asks the SSP to pay a send whose preimage swap is already committed with
    /// the operators.
    ///
    /// Returns `None` when there is nothing left to pay: the transfer is gone,
    /// it settled or was released on its own, or it never committed leaves under
    /// a preimage condition. The type check is what keeps a Spark-routed send,
    /// which pays the invoice without ever involving the SSP, from being paid a
    /// second time over Lightning.
    ///
    /// A quoted send resumes with the quote it committed its leaves under: the
    /// SSP must pay it against the same split.
    pub async fn resume_lightning_send(
        &self,
        transfer_id: &TransferId,
        invoice: &str,
        amount_to_send: Option<u64>,
        quote: Option<&IssuedQuote>,
    ) -> Result<Option<PayLightningResult>, ServiceError> {
        let Some(transfer) = self.transfer_service.query_transfer(transfer_id).await? else {
            return Ok(None);
        };
        if !is_resumable_send(transfer.transfer_type, transfer.status) {
            debug!(
                "Not resuming transfer {transfer_id}: type {:?}, status {}",
                transfer.transfer_type, transfer.status
            );
            return Ok(None);
        }

        let decoded_invoice = Bolt11Invoice::from_str(invoice)
            .map_err(|err| ServiceError::InvoiceDecodingError(err.to_string()))?;
        let payment_hash = *decoded_invoice.payment_hash();

        let quote = match quote {
            Some(issued_quote) => Some(CommittedSendQuote {
                total_sats: manifest_gross_sats(&decode_manifest(issued_quote)?)?,
                issued_quote: issued_quote.clone(),
            }),
            None => None,
        };
        self.request_lightning_send_result(
            transfer,
            transfer_id,
            invoice,
            amount_to_send,
            payment_hash,
            quote,
        )
        .await
        .map(Some)
    }

    /// Asks the SSP for a fee quote on paying `invoice` for `amount`, committing
    /// `transfer_id` when set, and checks it.
    pub async fn fetch_lightning_send_quote(
        &self,
        invoice: &str,
        amount: SendQuoteAmount,
        transfer_id: Option<&TransferId>,
    ) -> Result<QuotedLightningSend, ServiceError> {
        let (amount_sats, basis) = match amount {
            SendQuoteAmount::Invoice => (None, ReceiveAmountBasis::Net),
            SendQuoteAmount::Payment(sats) => (Some(sats), ReceiveAmountBasis::Net),
            SendQuoteAmount::Total(sats) => (Some(sats), ReceiveAmountBasis::Gross),
        };
        let invoice_has_amount = Bolt11Invoice::from_str(invoice)
            .map_err(|err| ServiceError::InvoiceDecodingError(err.to_string()))?
            .amount_milli_satoshis()
            .is_some_and(|msat| msat > 0);
        // The SSP refuses a payment amount beside the invoice's own.
        let amount_sats =
            amount_sats.filter(|_| !invoice_has_amount || basis == ReceiveAmountBasis::Gross);
        let quote = self
            .ssp_client
            .lightning_send_quote(
                invoice,
                amount_sats,
                basis,
                transfer_id.map(ToString::to_string),
            )
            .await?;
        if quote.attribution_status != SparkFeeAttributionStatus::Attributed {
            debug!(
                "Send quote carries no partner fee: {:?}",
                quote.attribution_status
            );
        }
        let payment_msats = quote
            .payment_msats
            .ok_or_else(|| QuoteError::invalid("the send quote names no payment"))?;
        let quoted = self
            .verify_lightning_send_quote(invoice, &quote.issued_quote, payment_msats, amount)
            .await?;
        if transfer_id.is_some_and(|id| *id != quoted.transfer_id) {
            return Err(QuoteError::invalid("the quote commits another transfer").into());
        }
        Ok(quoted)
    }

    /// Checks a send quote before any leaf is funded against it: the SSP signed
    /// it for this invoice and `payment_msats`, which is the payment `amount`
    /// asks for, it is for this network and unexpired, and this wallet is its
    /// sole sender. Re-run on a quote carried over from elsewhere, since only
    /// the SSP signature vouches for it.
    pub async fn verify_lightning_send_quote(
        &self,
        invoice: &str,
        issued_quote: &IssuedQuote,
        payment_msats: u64,
        amount: SendQuoteAmount,
    ) -> Result<QuotedLightningSend, ServiceError> {
        let decoded_invoice = Bolt11Invoice::from_str(invoice)
            .map_err(|err| ServiceError::InvoiceDecodingError(err.to_string()))?;
        if decoded_invoice.network() != self.network.into() {
            return Err(ServiceError::ValidationError(
                "Invoice network does not match".to_string(),
            ));
        }
        let manifest = decode_manifest(issued_quote)?;
        let total_sats = manifest_gross_sats(&manifest)?;
        let invoice_msats = decoded_invoice
            .amount_milli_satoshis()
            .filter(|msat| *msat > 0);
        let expected_msats = match (invoice_msats, amount) {
            (Some(msats), SendQuoteAmount::Invoice) => Some(msats),
            (Some(msats), SendQuoteAmount::Payment(sats)) if sats == msats.div_ceil(1000) => {
                Some(msats)
            }
            (None, SendQuoteAmount::Payment(sats)) if sats > 0 => Some(sats.saturating_mul(1000)),
            // An invoice with its own amount is paid exactly that: what the
            // total leaves beyond it and the fees goes to the fees.
            (_, SendQuoteAmount::Total(sats)) => {
                if total_sats != sats {
                    return Err(QuoteError::invalid(format!(
                        "the quote spends {total_sats} sats, not the {sats} asked for"
                    ))
                    .into());
                }
                invoice_msats
            }
            _ => {
                return Err(
                    QuoteError::invalid("the quoted amount does not fit the invoice").into(),
                );
            }
        };
        if expected_msats.is_some_and(|msats| msats != payment_msats) || payment_msats == 0 {
            return Err(QuoteError::invalid(format!(
                "the quote prices a payment of {payment_msats} msat"
            ))
            .into());
        }
        // An amountless invoice is paid the amount the commit names, in sats.
        let amount_to_send = match invoice_msats {
            Some(_) => None,
            None if payment_msats.is_multiple_of(1000) => Some(payment_msats / 1000),
            None => {
                return Err(
                    QuoteError::invalid("an amountless invoice is paid in whole satoshis").into(),
                );
            }
        };
        let amount_sats = payment_msats.div_ceil(1000);
        let payment_hash = *decoded_invoice.payment_hash();

        verify_identity_signature(
            &send_issuance_digest(
                &issued_quote.serialized_manifest,
                &payment_hash.to_byte_array(),
                payment_msats,
            )?,
            &issued_quote.issuer_signature,
            &self.ssp_client.identity_public_key(),
        )?;
        self.check_quote_terms(&manifest)?;
        let sender = self.spark_signer.get_identity_public_key().await?;
        validate_quoted_send_manifest(&manifest, amount_sats, &sender)?;

        Ok(QuotedLightningSend {
            issued_quote: issued_quote.clone(),
            transfer_id: TransferId::from_str(&manifest.transfer_id)
                .map_err(|e| QuoteError::invalid(format!("quote transfer id: {e}")))?,
            payment_hash,
            payment_msats,
            amount_sats,
            amount_to_send,
            total_sats,
            buckets: manifest_send_buckets(&manifest)?,
            expires_at: quote_expiry(&manifest)?,
            manifest,
        })
    }

    /// Pays `invoice` under `quote`: each of `leaves` goes to the receiver it
    /// is paired with, which must fund every bucket of the quote exactly. This
    /// wallet signs the manifest as its sole sender, and the operators enforce
    /// the split.
    pub async fn pay_quoted_lightning_invoice(
        &self,
        invoice: &str,
        quote: &QuotedLightningSend,
        leaves: &[(LeafKeyTweak, PublicKey)],
    ) -> Result<PayLightningResult, ServiceError> {
        let request = self
            .transfer_service
            .build_split_prepare_transfer_request(&quote.transfer_id, leaves);
        let prepared = self.spark_signer.prepare_transfer(request).await?;
        let digest = quote.manifest_hash()?;
        let manifest_hash_signature = self
            .spark_signer
            .sign_quote(SignQuoteRequest {
                kind: QuoteSignatureKind::TransferManifest,
                digest,
            })
            .await?
            .signature;
        self.submit_quoted_lightning_send(
            invoice,
            quote,
            leaves,
            prepared,
            &manifest_hash_signature,
        )
        .await
    }

    /// The signer request for a quoted send's key tweaks, each leaf going to
    /// the receiver it is paired with, for a signer that approves it apart.
    pub fn prepare_quoted_lightning_send(
        &self,
        quote: &QuotedLightningSend,
        leaves: &[(LeafKeyTweak, PublicKey)],
    ) -> PrepareTransferRequest {
        self.transfer_service
            .build_split_prepare_transfer_request(&quote.transfer_id, leaves)
    }

    /// [`pay_quoted_lightning_invoice`](Self::pay_quoted_lightning_invoice)
    /// with the key tweaks and manifest signature produced elsewhere, by a
    /// signer that approved them.
    pub async fn submit_quoted_lightning_send(
        &self,
        invoice: &str,
        quote: &QuotedLightningSend,
        leaves: &[(LeafKeyTweak, PublicKey)],
        prepared: PreparedTransfer,
        manifest_hash_signature: &bitcoin::secp256k1::ecdsa::Signature,
    ) -> Result<PayLightningResult, ServiceError> {
        // Selecting and swapping leaves takes time, so the quote may have
        // lapsed since it was checked.
        self.check_quote_terms(&quote.manifest)?;
        check_buckets_funded(&quote.buckets, leaves)?;
        self.notify_before_send_lightning(&quote.transfer_id, invoice, quote.amount_sats)
            .await?;

        let expiry_time = SystemTime::now() + Duration::from_secs(DEFAULT_SEND_EXPIRY_SECS);
        let transfer_request = self
            .transfer_service
            .assemble_split_lightning_transfer(
                &quote.transfer_id,
                leaves,
                &quote.payment_hash,
                expiry_time,
                quote.manifest.clone(),
                manifest_hash_signature,
                prepared,
            )
            .await?
            .transfer_request;

        let swap = self
            .operator_pool
            .get_coordinator()
            .client
            .initiate_preimage_swap_v4(
                InitiatePreimageSwapV4Request {
                    payment_hash: quote.payment_hash.to_byte_array().to_vec(),
                    invoice_amount: Some(InvoiceAmount {
                        invoice_amount_proof: Some(InvoiceAmountProof {
                            bolt11_invoice: invoice.to_string(),
                        }),
                        value_sats: quote.amount_sats,
                    }),
                    reason: PreimageSwapReason::Send as i32,
                    // A send's terms are bound by the SSP's own signature over
                    // the payment hash, so it attests with an empty signature.
                    attestor_identity_public_key: self
                        .ssp_client
                        .identity_public_key()
                        .serialize()
                        .to_vec(),
                    attestor_signature: Vec::new(),
                    transfer_v3_request: Some(transfer_request),
                },
                Some(quote.transfer_id.to_string()),
            )
            .await;
        let transfer = match swap {
            Ok(response) => transfer_from_preimage_swap(response)?,
            // As on an unquoted send: the swap can commit and still lose its
            // response, and only the SSP request below finishes the send.
            Err(e) => {
                self.transfer_service
                    .recover_committed_transfer(&quote.transfer_id, e.into())
                    .await?
            }
        };

        self.request_lightning_send_result(
            transfer,
            &quote.transfer_id,
            invoice,
            quote.amount_to_send,
            quote.payment_hash,
            Some(CommittedSendQuote {
                issued_quote: quote.issued_quote.clone(),
                total_sats: quote.total_sats,
            }),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn initiate_lightning_preimage_swap(
        &self,
        payment_hash: &sha256::Hash,
        invoice: &str,
        amount_sats: u64,
        transfer_request: StartTransferRequest,
    ) -> Result<InitiatePreimageSwapResponse, ServiceError> {
        let receiver_pubkey = self.ssp_client.identity_public_key();
        swap_nodes_for_preimage(
            &self.operator_pool,
            SwapNodesForPreimageRequest {
                receiver_pubkey: &receiver_pubkey,
                payment_hash,
                invoice_str: Some(invoice),
                amount_sats,
                fee_sats: 0,
                is_inbound_payment: false,
                transfer_request,
            },
        )
        .await
    }

    async fn request_lightning_send_result(
        &self,
        transfer: Transfer,
        transfer_id: &TransferId,
        invoice: &str,
        amount_to_send: Option<u64>,
        payment_hash: sha256::Hash,
        quote: Option<CommittedSendQuote>,
    ) -> Result<PayLightningResult, ServiceError> {
        let input = RequestLightningSendInput {
            encoded_invoice: invoice.to_string(),
            // The transfer id keys the request, so a repeat, whether the
            // client's own retry or a resume that cannot tell a lost response
            // from a lost request, resolves to the request the SSP already has.
            // Setting idempotency_key as well is refused outright: "Only
            // idempotency_key or user_outbound_transfer_external_id is needed".
            idempotency_key: None,
            amount_sats: amount_to_send,
            user_outbound_transfer_external_id: Some(transfer_id.to_string()),
        };
        let request = match &quote {
            Some(quote) => {
                self.ssp_client
                    .request_quoted_lightning_send(input, quote)
                    .await?
            }
            None => self.ssp_client.request_lightning_send(input).await?,
        };
        let mut lightning_send_payment: LightningSendPayment = request.try_into()?;
        if lightning_send_payment.transfer_id.is_none() {
            lightning_send_payment.transfer_id = Some(transfer.id.clone());
        }

        Ok(PayLightningResult {
            lightning_send_payment,
            transfer,
            payment_hash,
        })
    }

    pub fn prepare_lightning_send(
        &self,
        leaves: &[LeafKeyTweak],
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
        leaves: &[LeafKeyTweak],
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

/// Whether a send still has leaves committed under a preimage condition that
/// only the SSP can settle.
///
/// The type check carries the weight: a Spark-routed send pays the invoice
/// without the SSP ever being involved, so asking the SSP to pay it would pay
/// the same invoice twice.
fn is_resumable_send(transfer_type: TransferType, status: TransferStatus) -> bool {
    transfer_type == TransferType::PreimageSwap
        && !matches!(
            status,
            TransferStatus::Completed | TransferStatus::Expired | TransferStatus::Returned
        )
}

fn decode_manifest(issued_quote: &IssuedQuote) -> Result<TransferManifest, QuoteError> {
    TransferManifest::decode(issued_quote.serialized_manifest.as_slice())
        .map_err(|e| QuoteError::invalid(format!("undecodable manifest: {e}")))
}

fn quote_expiry(manifest: &TransferManifest) -> Result<u64, QuoteError> {
    manifest
        .quote_expiry_time
        .as_ref()
        .and_then(|t| u64::try_from(t.seconds).ok())
        .ok_or_else(|| QuoteError::invalid("quote has no expiry"))
}

/// Refuses leaves that do not fund each bucket exactly: the operators enforce
/// exact cover per receiver, so a mismatch would strand the leaves at prepare.
fn check_buckets_funded(
    buckets: &[SendManifestBucket],
    leaves: &[(LeafKeyTweak, PublicKey)],
) -> Result<(), QuoteError> {
    let mut funded: Vec<(PublicKey, u64)> = Vec::new();
    for (leaf, receiver) in leaves {
        match funded.iter_mut().find(|(key, _)| key == receiver) {
            Some((_, sats)) => *sats = sats.saturating_add(leaf.node.value),
            None => funded.push((*receiver, leaf.node.value)),
        }
    }
    let exact = funded.len() == buckets.len()
        && buckets.iter().all(|bucket| {
            funded.iter().any(|(key, sats)| {
                *key == bucket.receiver_identity_public_key && *sats == bucket.amount_sats
            })
        });
    if exact {
        Ok(())
    } else {
        Err(QuoteError::invalid(
            "the leaves do not fund each of the quote's receivers exactly",
        ))
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

#[cfg(test)]
mod get_invoice_amount_sats_tests {
    use super::{ServiceError, get_invoice_amount_sats};
    use bitcoin::hashes::{Hash, sha256};
    use bitcoin::secp256k1::{Secp256k1, SecretKey};
    use lightning_invoice::{Bolt11Invoice, Currency, InvoiceBuilder, PaymentSecret};

    fn invoice(amount_msat: Option<u64>) -> Bolt11Invoice {
        let secp = Secp256k1::new();
        let key = SecretKey::from_slice(&[0x11; 32]).unwrap();
        let sign = |m: &_| secp.sign_ecdsa_recoverable(m, &key);
        let builder = InvoiceBuilder::new(Currency::Bitcoin)
            .description("test".to_string())
            .payment_hash(sha256::Hash::from_byte_array([0x33; 32]))
            .payment_secret(PaymentSecret([0x22; 32]))
            .duration_since_epoch(std::time::Duration::from_secs(1_700_000_000))
            .min_final_cltv_expiry_delta(144);
        match amount_msat {
            Some(amt) => builder
                .amount_milli_satoshis(amt)
                .build_signed(sign)
                .unwrap(),
            None => builder.build_signed(sign).unwrap(),
        }
    }

    #[test]
    fn rounds_a_sub_sat_remainder_up() {
        assert_eq!(
            get_invoice_amount_sats(&invoice(Some(1_500)), None).unwrap(),
            2
        );
        assert_eq!(get_invoice_amount_sats(&invoice(Some(1)), None).unwrap(), 1);
        assert_eq!(
            get_invoice_amount_sats(&invoice(Some(2_000)), None).unwrap(),
            2
        );
    }

    #[test]
    fn accepts_the_rounded_up_invoice_amount() {
        assert_eq!(
            get_invoice_amount_sats(&invoice(Some(1_500)), Some(2)).unwrap(),
            2
        );
    }

    #[test]
    fn rejects_an_amount_below_the_rounded_up_invoice_amount() {
        assert!(matches!(
            get_invoice_amount_sats(&invoice(Some(1_500)), Some(1)),
            Err(ServiceError::ValidationError(_))
        ));
    }

    #[test]
    fn rejects_an_amountless_invoice_without_an_amount() {
        assert!(matches!(
            get_invoice_amount_sats(&invoice(None), None),
            Err(ServiceError::ValidationError(_))
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVERY_STATUS: [TransferStatus; 12] = [
        TransferStatus::SenderInitiated,
        TransferStatus::SenderKeyTweakPending,
        TransferStatus::SenderKeyTweaked,
        TransferStatus::ReceiverKeyTweaked,
        TransferStatus::ReceiverRefundSigned,
        TransferStatus::Completed,
        TransferStatus::Expired,
        TransferStatus::Returned,
        TransferStatus::SenderInitiatedCoordinator,
        TransferStatus::ReceiverKeyTweakLocked,
        TransferStatus::ReceiverKeyTweakApplied,
        TransferStatus::ApplyingSenderKeyTweak,
    ];

    const EVERY_TYPE: [TransferType; 8] = [
        TransferType::PreimageSwap,
        TransferType::CooperativeExit,
        TransferType::Transfer,
        TransferType::UtxoSwap,
        TransferType::Swap,
        TransferType::CounterSwap,
        TransferType::PrimarySwapV3,
        TransferType::CounterSwapV3,
    ];

    /// The money-critical case: a Spark-routed send pays the invoice itself, so
    /// resuming one would pay the same invoice a second time over Lightning.
    #[test]
    fn only_preimage_swaps_resume() {
        for transfer_type in EVERY_TYPE {
            for status in EVERY_STATUS {
                if transfer_type != TransferType::PreimageSwap {
                    assert!(
                        !is_resumable_send(transfer_type, status),
                        "{transfer_type:?} at {status} must never be resumed"
                    );
                }
            }
        }
    }

    /// A settled or released transfer has no leaves left under the preimage
    /// condition, so asking the SSP to pay again would open a request against
    /// leaves that are gone.
    #[test]
    fn terminal_preimage_swaps_do_not_resume() {
        for status in [
            TransferStatus::Completed,
            TransferStatus::Expired,
            TransferStatus::Returned,
        ] {
            assert!(!is_resumable_send(TransferType::PreimageSwap, status));
        }
    }

    #[test]
    fn in_flight_preimage_swaps_resume() {
        for status in EVERY_STATUS {
            if matches!(
                status,
                TransferStatus::Completed | TransferStatus::Expired | TransferStatus::Returned
            ) {
                continue;
            }
            assert!(
                is_resumable_send(TransferType::PreimageSwap, status),
                "{status} still holds committed leaves and must resume"
            );
        }
    }
}

#[cfg(test)]
mod quoted_payment_tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use bitcoin::secp256k1::{Secp256k1, SecretKey};
    use lightning_invoice::{Currency, InvoiceBuilder, PaymentSecret};
    use macros::async_test_all;
    use platform_utils::{HttpClient, HttpError, HttpResponse};
    use serde_json::{Value, json};

    use super::*;
    use crate::operator::rpc::spark::manifest_amount::Amount;
    use crate::operator::rpc::spark::{
        FeeComponent, FeeRole, FeeSource, ManifestAmount, ManifestEdge, Network as ProtoNetwork,
    };
    use crate::operator::testing::unroutable_operator_pool;
    use crate::quote::verify_identity_signature;
    use crate::session_store::{InMemorySessionStore, Session, SessionStore};
    use crate::signer::testing::RecordingSparkSigner;
    use crate::ssp::{RetryConfig, ServiceProviderConfig};

    type Respond = Box<dyn Fn(&str, &Value) -> Value + Send + Sync>;

    /// Plays the SSP: answers each GraphQL operation by name and records every
    /// request body.
    struct ScriptedSsp {
        respond: Respond,
        bodies: Mutex<Vec<Value>>,
    }

    impl ScriptedSsp {
        fn operations(&self) -> Vec<(String, Value)> {
            self.bodies
                .lock()
                .unwrap()
                .iter()
                .map(|body| {
                    (
                        body["operationName"].as_str().unwrap().to_string(),
                        body["variables"]["input"].clone(),
                    )
                })
                .collect()
        }
    }

    #[macros::async_trait]
    impl HttpClient for ScriptedSsp {
        async fn get(
            &self,
            _url: String,
            _headers: Option<HashMap<String, String>>,
        ) -> Result<HttpResponse, HttpError> {
            unimplemented!("the SSP is only posted to")
        }

        async fn post(
            &self,
            _url: String,
            _headers: Option<HashMap<String, String>>,
            body: Option<String>,
        ) -> Result<HttpResponse, HttpError> {
            let body: Value = serde_json::from_str(&body.unwrap_or_default()).unwrap();
            let response = (self.respond)(body["operationName"].as_str().unwrap(), &body);
            self.bodies.lock().unwrap().push(body);
            Ok(HttpResponse {
                status: 200,
                body: response.to_string(),
                headers: HashMap::new(),
            })
        }

        async fn delete(
            &self,
            _url: String,
            _headers: Option<HashMap<String, String>>,
            _body: Option<String>,
        ) -> Result<HttpResponse, HttpError> {
            unimplemented!("the SSP is only posted to")
        }
    }

    const SSP_SEED: u8 = 0x51;

    fn key(seed: u8) -> PublicKey {
        SecretKey::from_slice(&[seed; 32])
            .unwrap()
            .public_key(&Secp256k1::new())
    }

    fn sats(value: u64) -> Option<ManifestAmount> {
        Some(ManifestAmount {
            amount: Some(Amount::Sats(value)),
        })
    }

    /// A 2_000 sat markup on a 100_000 sat net receive for `receiver`.
    fn net_quote(receiver: PublicKey, network: ProtoNetwork) -> Vec<u8> {
        quote_paying(receiver, network, 100_000)
    }

    /// A 2_000 sat markup on top of `receiver_sats` for `receiver`.
    fn quote_paying(receiver: PublicKey, network: ProtoNetwork, receiver_sats: u64) -> Vec<u8> {
        let ssp = key(SSP_SEED).serialize().to_vec();
        let affiliate = key(0x52).serialize().to_vec();
        let edge = |to: Vec<u8>, amount| ManifestEdge {
            sender_identity_public_key: ssp.clone(),
            receiver_identity_public_key: to,
            amount: sats(amount),
        };
        let expiry = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 600;
        TransferManifest {
            version: 1,
            transfer_id: "0197f9a0-0000-7000-8000-000000000001".to_string(),
            network: network.into(),
            edges: vec![
                edge(receiver.serialize().to_vec(), receiver_sats),
                edge(affiliate.clone(), 2_000),
            ],
            fees: vec![FeeComponent {
                source: FeeSource::PartnerMarkup.into(),
                role: FeeRole::Affiliate.into(),
                amount: sats(2_000),
                receiver_identity_public_key: affiliate,
            }],
            quote_expiry_time: Some(prost_types::Timestamp {
                seconds: i64::try_from(expiry).unwrap(),
                nanos: 0,
            }),
            ..Default::default()
        }
        .encode_to_vec()
    }

    fn quote_response(manifest: &[u8]) -> Value {
        json!({"data": {"lightning_receive_quote": {
            "issued_quote": {
                "serialized_manifest": hex::encode(manifest),
                "issuer_signature": "3044",
            },
            "attribution_status": "ATTRIBUTED",
        }}})
    }

    /// The SSP's invoice for whatever hash and amount the request names.
    fn receive_response(body: &Value) -> Value {
        let input = &body["variables"]["input"];
        let payment_hash = sha256::Hash::from_str(input["payment_hash"].as_str().unwrap()).unwrap();
        let amount_sats = input["amount_sats"].as_u64().unwrap();
        let secp = Secp256k1::new();
        let node_key = SecretKey::from_slice(&[0x11; 32]).unwrap();
        let invoice = InvoiceBuilder::new(Currency::Regtest)
            .description("test".to_string())
            .payment_hash(payment_hash)
            .payment_secret(PaymentSecret([0x22; 32]))
            .duration_since_epoch(Duration::from_secs(1_700_000_000))
            .min_final_cltv_expiry_delta(144)
            .expiry_time(Duration::from_secs(u64::from(DEFAULT_RECEIVE_EXPIRY_SECS)))
            .amount_milli_satoshis(amount_sats * 1000)
            .build_signed(|m| secp.sign_ecdsa_recoverable(m, &node_key))
            .unwrap();
        let amount = json!({"__typename": "CurrencyAmount", "original_value": amount_sats,
            "original_unit": "SATOSHI", "preferred_currency_unit": "USD",
            "preferred_currency_value_rounded": 0});
        json!({"data": {"request_lightning_receive": {"request": {
            "__typename": "LightningReceiveRequest", "id": "LightningReceiveRequest:1",
            "created_at": "2026-10-13T00:00:00Z", "updated_at": "2026-10-13T00:00:00Z",
            "network": "REGTEST", "lightning_request_status": "INVOICE_CREATED",
            "invoice": {"__typename": "Invoice", "encoded_invoice": invoice.to_string(),
                "bitcoin_network": "REGTEST", "payment_hash": payment_hash.to_string(),
                "amount": amount, "created_at": "2026-10-13T00:00:00Z",
                "invoice_expired_at": "2026-11-13T00:00:00Z", "memo": null},
            "transfer": null, "lightning_receive_payment_preimage": null,
            "receiver_identity_public_key": null}}}})
    }

    fn refusal() -> Value {
        json!({"data": null, "errors": [{"message": "malformed quote"}]})
    }

    async fn service(respond: Respond) -> (LightningService, Arc<ScriptedSsp>, PublicKey) {
        let signer: Arc<dyn SparkSigner> = Arc::new(RecordingSparkSigner::new());
        let identity = signer.get_identity_public_key().await.unwrap();
        let ssp_key = key(SSP_SEED);
        // A live session, so no request has to authenticate.
        let session_store = Arc::new(InMemorySessionStore::default());
        session_store
            .set_session(
                &ssp_key,
                Session {
                    token: "token".to_string(),
                    expiration: u64::MAX / 2,
                    refresh_token: None,
                    refresh_token_expiration: None,
                },
            )
            .await
            .unwrap();
        let ssp = Arc::new(ScriptedSsp {
            respond,
            bodies: Mutex::default(),
        });
        let ssp_client = Arc::new(ServiceProvider::new_with_client(
            ServiceProviderConfig {
                base_url: "http://ssp.invalid".to_string(),
                schema_endpoint: Some("graphql/spark/rc".to_string()),
                identity_public_key: ssp_key,
                user_agent: None,
                retry_config: RetryConfig {
                    max_retries: 0,
                    base_delay_ms: 1,
                    max_delay_ms: 1,
                },
            },
            Arc::clone(&signer),
            session_store,
            None,
            Arc::clone(&ssp) as Arc<dyn HttpClient>,
        ));
        let operator_pool = unroutable_operator_pool(&signer).await;
        let transfer_service = Arc::new(TransferService::new(
            Arc::clone(&signer),
            Network::Regtest,
            1,
            Arc::clone(&operator_pool),
            None,
            false,
        ));
        let service = LightningService::new(
            operator_pool,
            ssp_client,
            Network::Regtest,
            signer,
            transfer_service,
            1,
            None,
        );
        (service, ssp, identity)
    }

    async fn receive(service: &LightningService, basis: Option<ReceiveAmountBasis>) {
        // The pool's operator listens nowhere, so the receive stops at the
        // preimage share store, after every SSP call it makes.
        let result = service
            .create_lightning_invoice(100_000, None, None, None, false, None, basis)
            .await;
        assert!(
            matches!(result, Err(ServiceError::PreimageShareStoreFailed(_))),
            "{result:?}"
        );
    }

    #[async_test_all]
    async fn a_quoted_receive_invoices_the_gross_and_attests_to_it() {
        let identity = Arc::new(Mutex::new(None::<PublicKey>));
        let quote_for = Arc::clone(&identity);
        let (service, ssp, me) = service(Box::new(move |op, body| match op {
            "LightningReceiveQuote" => quote_response(&net_quote(
                quote_for.lock().unwrap().unwrap(),
                ProtoNetwork::Regtest,
            )),
            "RequestLightningReceive" => receive_response(body),
            other => panic!("unexpected {other}"),
        }))
        .await;
        *identity.lock().unwrap() = Some(me);

        receive(&service, Some(ReceiveAmountBasis::Net)).await;

        let operations = ssp.operations();
        assert_eq!(operations.len(), 2, "{operations:?}");
        let (quote_op, quote_input) = &operations[0];
        assert_eq!(quote_op, "LightningReceiveQuote");
        assert_eq!(quote_input["amount_basis"], "NET");
        assert_eq!(quote_input["receiver_identity_pubkey"], Value::Null);
        let (_, request) = &operations[1];
        assert_eq!(
            request["amount_sats"], 102_000,
            "the invoice is for the gross"
        );
        let committed = &request["committed_quote"];
        let manifest = hex::decode(committed["serialized_manifest"].as_str().unwrap()).unwrap();
        let payment_hash: [u8; 32] = hex::decode(request["payment_hash"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        verify_identity_signature(
            &receive_attestation_digest(&manifest, &payment_hash).unwrap(),
            &hex::decode(committed["attestor_signature"].as_str().unwrap()).unwrap(),
            &me,
        )
        .expect("the attestation verifies under the wallet's identity key");
    }

    #[async_test_all]
    async fn a_refused_quote_falls_back_to_an_unquoted_invoice() {
        let identity = Arc::new(Mutex::new(None::<PublicKey>));
        let quote_for = Arc::clone(&identity);
        let (service, ssp, me) = service(Box::new(move |op, body| match op {
            "LightningReceiveQuote" => quote_response(&net_quote(
                quote_for.lock().unwrap().unwrap(),
                ProtoNetwork::Regtest,
            )),
            "RequestLightningReceive"
                if !body["variables"]["input"]["committed_quote"].is_null() =>
            {
                refusal()
            }
            "RequestLightningReceive" => receive_response(body),
            other => panic!("unexpected {other}"),
        }))
        .await;
        *identity.lock().unwrap() = Some(me);

        receive(&service, Some(ReceiveAmountBasis::Net)).await;

        let operations = ssp.operations();
        assert_eq!(operations.len(), 3, "{operations:?}");
        let (_, unquoted) = &operations[2];
        assert!(unquoted.get("committed_quote").is_none());
        assert_eq!(
            unquoted["amount_sats"], 100_000,
            "the requested amount, unpriced"
        );
    }

    #[async_test_all]
    async fn a_quote_for_another_network_is_not_used() {
        let identity = Arc::new(Mutex::new(None::<PublicKey>));
        let quote_for = Arc::clone(&identity);
        let (service, ssp, me) = service(Box::new(move |op, body| match op {
            "LightningReceiveQuote" => quote_response(&net_quote(
                quote_for.lock().unwrap().unwrap(),
                ProtoNetwork::Mainnet,
            )),
            "RequestLightningReceive" => receive_response(body),
            other => panic!("unexpected {other}"),
        }))
        .await;
        *identity.lock().unwrap() = Some(me);

        receive(&service, Some(ReceiveAmountBasis::Net)).await;

        let operations = ssp.operations();
        assert_eq!(operations.len(), 2, "{operations:?}");
        assert!(operations[1].1.get("committed_quote").is_none());
        assert_eq!(operations[1].1["amount_sats"], 100_000);
    }

    #[async_test_all]
    async fn a_quote_the_ssp_cannot_issue_does_not_block_the_receive() {
        let (service, ssp, _) = service(Box::new(|op, body| match op {
            "LightningReceiveQuote" => refusal(),
            "RequestLightningReceive" => receive_response(body),
            other => panic!("unexpected {other}"),
        }))
        .await;

        receive(&service, Some(ReceiveAmountBasis::Gross)).await;

        let operations = ssp.operations();
        assert_eq!(operations.len(), 2, "{operations:?}");
        assert!(operations[1].1.get("committed_quote").is_none());
    }

    #[async_test_all]
    async fn a_receive_attested_for_another_payee_takes_the_fee_from_the_gross() {
        let payee = key(0x60);
        let (service, ssp, me) = service(Box::new(move |op, body| match op {
            "LightningReceiveQuote" => {
                quote_response(&quote_paying(payee, ProtoNetwork::Regtest, 98_000))
            }
            "RequestLightningReceive" => receive_response(body),
            other => panic!("unexpected {other}"),
        }))
        .await;

        let result = service
            .create_lightning_invoice(
                100_000,
                None,
                None,
                None,
                false,
                Some(payee),
                Some(ReceiveAmountBasis::Gross),
            )
            .await;
        assert!(
            matches!(result, Err(ServiceError::PreimageShareStoreFailed(_))),
            "{result:?}"
        );

        let operations = ssp.operations();
        assert_eq!(operations.len(), 2, "{operations:?}");
        let (_, quote_input) = &operations[0];
        assert_eq!(quote_input["amount_basis"], "GROSS");
        let payee_hex = hex::encode(payee.serialize());
        assert_eq!(quote_input["receiver_identity_pubkey"], payee_hex.as_str());
        let (_, request) = &operations[1];
        assert_eq!(request["amount_sats"], 100_000, "the invoice is the gross");
        assert_eq!(request["receiver_identity_pubkey"], payee_hex.as_str());
        let committed = &request["committed_quote"];
        let manifest = hex::decode(committed["serialized_manifest"].as_str().unwrap()).unwrap();
        let payment_hash: [u8; 32] = hex::decode(request["payment_hash"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        verify_identity_signature(
            &receive_attestation_digest(&manifest, &payment_hash).unwrap(),
            &hex::decode(committed["attestor_signature"].as_str().unwrap()).unwrap(),
            &me,
        )
        .expect("the attestor, not the payee, signs the attestation");
    }

    #[async_test_all]
    async fn a_quoted_hodl_receive_attests_over_the_callers_payment_hash() {
        let identity = Arc::new(Mutex::new(None::<PublicKey>));
        let quote_for = Arc::clone(&identity);
        let (service, ssp, me) = service(Box::new(move |op, body| match op {
            "LightningReceiveQuote" => quote_response(&net_quote(
                quote_for.lock().unwrap().unwrap(),
                ProtoNetwork::Regtest,
            )),
            "RequestLightningReceive" => receive_response(body),
            other => panic!("unexpected {other}"),
        }))
        .await;
        *identity.lock().unwrap() = Some(me);
        let payment_hash = sha256::Hash::hash(b"caller's preimage");

        // A HODL receive stores no preimage share, so it reaches no operator.
        let payment = service
            .create_hodl_lightning_invoice(
                100_000,
                None,
                payment_hash,
                None,
                Some(ReceiveAmountBasis::Net),
            )
            .await
            .unwrap();

        assert_eq!(
            payment.quote_fee,
            Some(ReceiveQuoteFee {
                flat_sats: 2_000,
                rate_bps: 0
            })
        );
        let operations = ssp.operations();
        assert_eq!(operations.len(), 2, "{operations:?}");
        let (_, request) = &operations[1];
        assert_eq!(request["amount_sats"], 102_000);
        assert_eq!(request["payment_hash"], payment_hash.to_string());
        let committed = &request["committed_quote"];
        let manifest = hex::decode(committed["serialized_manifest"].as_str().unwrap()).unwrap();
        verify_identity_signature(
            &receive_attestation_digest(&manifest, &payment_hash.to_byte_array()).unwrap(),
            &hex::decode(committed["attestor_signature"].as_str().unwrap()).unwrap(),
            &me,
        )
        .expect("the attestation covers the caller's payment hash");
    }

    #[async_test_all]
    async fn a_receive_with_a_spark_address_asks_for_no_quote() {
        let (service, ssp, _) = service(Box::new(|op, body| match op {
            "RequestLightningReceive" => receive_response(body),
            other => panic!("unexpected {other}"),
        }))
        .await;

        let _ = service
            .create_lightning_invoice(
                100_000,
                None,
                None,
                None,
                true,
                None,
                Some(ReceiveAmountBasis::Net),
            )
            .await;

        let operations = ssp.operations();
        assert_eq!(operations.len(), 1, "{operations:?}");
        assert!(operations[0].1.get("committed_quote").is_none());
        assert_eq!(operations[0].1["include_spark_address"], true);
    }

    #[async_test_all]
    async fn an_unquoted_receive_asks_for_no_quote() {
        let (service, ssp, _) = service(Box::new(|op, body| match op {
            "RequestLightningReceive" => receive_response(body),
            other => panic!("unexpected {other}"),
        }))
        .await;

        receive(&service, None).await;

        let operations = ssp.operations();
        assert_eq!(operations.len(), 1, "{operations:?}");
        assert!(operations[0].1.get("committed_quote").is_none());
    }

    // ─── quoted sends ─────────────────────────────────────────────────────

    const PARTNER_SEED: u8 = 0x53;
    const INVOICE_MSATS: u64 = 100_000_000;

    fn bolt11(payment_hash: sha256::Hash, amount_msats: Option<u64>) -> String {
        let secp = Secp256k1::new();
        let node_key = SecretKey::from_slice(&[0x11; 32]).unwrap();
        let builder = InvoiceBuilder::new(Currency::Regtest)
            .description("test".to_string())
            .payment_hash(payment_hash)
            .payment_secret(PaymentSecret([0x22; 32]))
            .duration_since_epoch(Duration::from_secs(1_700_000_000))
            .min_final_cltv_expiry_delta(144);
        match amount_msats {
            Some(msats) => builder
                .amount_milli_satoshis(msats)
                .build_signed(|m| secp.sign_ecdsa_recoverable(m, &node_key))
                .unwrap()
                .to_string(),
            None => builder
                .build_signed(|m| secp.sign_ecdsa_recoverable(m, &node_key))
                .unwrap()
                .to_string(),
        }
    }

    /// The SSP's split of a 100_000 sat send by `sender`: 100_500 to the SSP,
    /// covering the payment and its fees, and a 1_000 sat partner markup.
    fn send_manifest(sender: PublicKey, expires_in_secs: i64) -> Vec<u8> {
        let edge = |to: PublicKey, amount| ManifestEdge {
            sender_identity_public_key: sender.serialize().to_vec(),
            receiver_identity_public_key: to.serialize().to_vec(),
            amount: sats(amount),
        };
        let now = i64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        )
        .unwrap();
        TransferManifest {
            version: 1,
            transfer_id: "0197f9a0-0000-7000-8000-000000000002".to_string(),
            network: ProtoNetwork::Regtest.into(),
            edges: vec![edge(key(SSP_SEED), 100_500), edge(key(PARTNER_SEED), 1_000)],
            fees: vec![FeeComponent {
                source: FeeSource::PartnerMarkup.into(),
                role: FeeRole::Partner.into(),
                amount: sats(1_000),
                receiver_identity_public_key: key(PARTNER_SEED).serialize().to_vec(),
            }],
            quote_expiry_time: Some(prost_types::Timestamp {
                seconds: now + expires_in_secs,
                nanos: 0,
            }),
            ..Default::default()
        }
        .encode_to_vec()
    }

    /// The issuer signature the SSP makes over a send quote.
    fn issue(signer_seed: u8, manifest: &[u8], payment_hash: sha256::Hash, msats: u64) -> Vec<u8> {
        let digest = send_issuance_digest(manifest, &payment_hash.to_byte_array(), msats).unwrap();
        Secp256k1::new()
            .sign_ecdsa(
                &bitcoin::secp256k1::Message::from_digest(digest),
                &SecretKey::from_slice(&[signer_seed; 32]).unwrap(),
            )
            .serialize_der()
            .to_vec()
    }

    fn send_quote_response(manifest: &[u8], signature: &[u8], payment_msats: u64) -> Value {
        json!({"data": {"lightning_send_quote": {
            "issued_quote": {
                "serialized_manifest": hex::encode(manifest),
                "issuer_signature": hex::encode(signature),
            },
            "attribution_status": "ATTRIBUTED",
            "payment_msats": payment_msats,
        }}})
    }

    /// A service whose SSP answers a send quote with `manifest`, signed by the
    /// key seeded `signer_seed` for `signed_msats`.
    async fn quoting_service(
        manifest: impl Fn(PublicKey) -> Vec<u8> + Send + Sync + 'static,
        signer_seed: u8,
        signed_msats: u64,
    ) -> (LightningService, PublicKey, sha256::Hash) {
        let payment_hash = sha256::Hash::hash(b"quoted send");
        let identity = Arc::new(Mutex::new(None::<PublicKey>));
        let quote_for = Arc::clone(&identity);
        let (service, _, me) = service(Box::new(move |op, _| match op {
            "LightningSendQuote" => {
                let manifest = manifest(quote_for.lock().unwrap().unwrap());
                let signature = issue(signer_seed, &manifest, payment_hash, signed_msats);
                send_quote_response(&manifest, &signature, signed_msats)
            }
            other => panic!("unexpected {other}"),
        }))
        .await;
        *identity.lock().unwrap() = Some(me);
        (service, me, payment_hash)
    }

    #[async_test_all]
    async fn a_send_quote_the_ssp_signed_for_this_invoice_is_accepted() {
        let (service, _, payment_hash) =
            quoting_service(|me| send_manifest(me, 600), SSP_SEED, INVOICE_MSATS).await;

        let quote = service
            .fetch_lightning_send_quote(
                &bolt11(payment_hash, Some(INVOICE_MSATS)),
                SendQuoteAmount::Invoice,
                None,
            )
            .await
            .unwrap();

        assert_eq!(quote.amount_sats, 100_000);
        assert_eq!(quote.amount_to_send, None);
        assert_eq!(quote.total_sats, 101_500);
        assert_eq!(quote.payment_hash, payment_hash);
        assert_eq!(
            quote.transfer_id.to_string(),
            "0197f9a0-0000-7000-8000-000000000002"
        );
        let buckets: Vec<(PublicKey, u64)> = quote
            .buckets
            .iter()
            .map(|b| (b.receiver_identity_public_key, b.amount_sats))
            .collect();
        assert_eq!(
            buckets,
            vec![(key(SSP_SEED), 100_500), (key(PARTNER_SEED), 1_000)]
        );
    }

    async fn assert_quote_refused(
        manifest: impl Fn(PublicKey) -> Vec<u8> + Send + Sync + 'static,
        signer_seed: u8,
        signed_msats: u64,
        invoice_msats: Option<u64>,
        amount_sats: Option<u64>,
    ) {
        let (service, _, payment_hash) = quoting_service(manifest, signer_seed, signed_msats).await;
        let result = service
            .fetch_lightning_send_quote(
                &bolt11(payment_hash, invoice_msats),
                amount_sats.map_or(SendQuoteAmount::Invoice, SendQuoteAmount::Payment),
                None,
            )
            .await;
        assert!(result.is_err(), "{result:?}");
    }

    // The issuer signature binds the amount, so a quote priced for one amount
    // cannot be committed against another.
    #[async_test_all]
    async fn a_send_quote_signed_for_another_amount_is_refused() {
        assert_quote_refused(
            |me| send_manifest(me, 600),
            SSP_SEED,
            1_000,
            Some(INVOICE_MSATS),
            None,
        )
        .await;
    }

    #[async_test_all]
    async fn a_send_quote_signed_by_another_key_is_refused() {
        assert_quote_refused(
            |me| send_manifest(me, 600),
            0x77,
            INVOICE_MSATS,
            Some(INVOICE_MSATS),
            None,
        )
        .await;
    }

    #[async_test_all]
    async fn an_expired_send_quote_is_refused() {
        assert_quote_refused(
            |me| send_manifest(me, -1),
            SSP_SEED,
            INVOICE_MSATS,
            Some(INVOICE_MSATS),
            None,
        )
        .await;
    }

    #[async_test_all]
    async fn a_send_quote_debiting_another_wallet_is_refused() {
        assert_quote_refused(
            |_| send_manifest(key(0x78), 600),
            SSP_SEED,
            INVOICE_MSATS,
            Some(INVOICE_MSATS),
            None,
        )
        .await;
    }

    #[async_test_all]
    async fn an_amountless_invoice_is_not_quoted_without_an_amount() {
        assert_quote_refused(
            |me| send_manifest(me, 600),
            SSP_SEED,
            INVOICE_MSATS,
            None,
            None,
        )
        .await;
    }

    #[async_test_all]
    async fn an_amount_beside_the_invoices_own_is_refused() {
        assert_quote_refused(
            |me| send_manifest(me, 600),
            SSP_SEED,
            INVOICE_MSATS,
            Some(INVOICE_MSATS),
            Some(99_999),
        )
        .await;
    }

    /// A send quote whose SSP prices `payment_msats`, recording what each
    /// quote request asked for.
    async fn amountless_quoting_service(
        payment_msats: u64,
    ) -> (LightningService, Arc<ScriptedSsp>, sha256::Hash) {
        let payment_hash = sha256::Hash::hash(b"quoted send");
        let identity = Arc::new(Mutex::new(None::<PublicKey>));
        let quote_for = Arc::clone(&identity);
        let (service, ssp, me) = service(Box::new(move |op, _| match op {
            "LightningSendQuote" => {
                let manifest = send_manifest(quote_for.lock().unwrap().unwrap(), 600);
                let signature = issue(SSP_SEED, &manifest, payment_hash, payment_msats);
                send_quote_response(&manifest, &signature, payment_msats)
            }
            other => panic!("unexpected {other}"),
        }))
        .await;
        *identity.lock().unwrap() = Some(me);
        (service, ssp, payment_hash)
    }

    #[async_test_all]
    async fn a_total_is_quoted_gross_and_pays_what_is_left_after_fees() {
        let (service, ssp, payment_hash) = amountless_quoting_service(INVOICE_MSATS).await;

        let quote = service
            .fetch_lightning_send_quote(
                &bolt11(payment_hash, None),
                SendQuoteAmount::Total(101_500),
                None,
            )
            .await
            .unwrap();

        let (_, input) = &ssp.operations()[0];
        assert_eq!(input["amount_basis"], "GROSS");
        assert_eq!(input["amount_sats"], 101_500);
        assert_eq!(quote.total_sats, 101_500);
        assert_eq!(quote.amount_to_send, Some(100_000));
    }

    #[async_test_all]
    async fn a_quote_spending_other_than_the_total_is_refused() {
        let (service, _, payment_hash) = amountless_quoting_service(INVOICE_MSATS).await;

        let result = service
            .fetch_lightning_send_quote(
                &bolt11(payment_hash, None),
                SendQuoteAmount::Total(101_000),
                None,
            )
            .await;

        assert!(result.is_err(), "{result:?}");
    }

    #[async_test_all]
    async fn a_total_on_an_invoice_with_its_own_amount_pays_the_invoice() {
        let (service, ssp, payment_hash) = amountless_quoting_service(INVOICE_MSATS).await;

        let quote = service
            .fetch_lightning_send_quote(
                &bolt11(payment_hash, Some(INVOICE_MSATS)),
                SendQuoteAmount::Total(101_500),
                None,
            )
            .await
            .unwrap();

        let (_, input) = &ssp.operations()[0];
        assert_eq!(input["amount_basis"], "GROSS");
        assert_eq!(input["amount_sats"], 101_500);
        assert_eq!(quote.total_sats, 101_500);
        assert_eq!(quote.payment_msats, INVOICE_MSATS);
        assert_eq!(quote.amount_to_send, None);
    }

    #[async_test_all]
    async fn a_total_overpaying_an_invoice_with_its_own_amount_is_refused() {
        let (service, _, payment_hash) = amountless_quoting_service(INVOICE_MSATS).await;

        let result = service
            .fetch_lightning_send_quote(
                &bolt11(payment_hash, Some(INVOICE_MSATS - 1_000)),
                SendQuoteAmount::Total(101_500),
                None,
            )
            .await;

        assert!(result.is_err(), "{result:?}");
    }

    #[async_test_all]
    async fn a_quote_commits_the_transfer_asked_for() {
        let (service, ssp, payment_hash) = amountless_quoting_service(INVOICE_MSATS).await;
        let invoice = bolt11(payment_hash, Some(INVOICE_MSATS));
        let asked = TransferId::from_str("0197f9a0-0000-7000-8000-000000000002").unwrap();

        let quote = service
            .fetch_lightning_send_quote(&invoice, SendQuoteAmount::Invoice, Some(&asked))
            .await
            .unwrap();
        assert_eq!(quote.transfer_id, asked);
        assert_eq!(ssp.operations()[0].1["transfer_id"], asked.to_string());

        // The scripted SSP commits its own transfer whatever it is asked.
        let other = TransferId::generate();
        let result = service
            .fetch_lightning_send_quote(&invoice, SendQuoteAmount::Invoice, Some(&other))
            .await;
        assert!(result.is_err(), "{result:?}");
    }

    #[async_test_all]
    async fn an_amountless_invoice_is_quoted_for_the_amount_sent() {
        let payment_hash = sha256::Hash::hash(b"quoted send");
        let identity = Arc::new(Mutex::new(None::<PublicKey>));
        let quote_for = Arc::clone(&identity);
        let (service, ssp, me) = service(Box::new(move |op, _| match op {
            "LightningSendQuote" => {
                let manifest = send_manifest(quote_for.lock().unwrap().unwrap(), 600);
                let signature = issue(SSP_SEED, &manifest, payment_hash, INVOICE_MSATS);
                send_quote_response(&manifest, &signature, INVOICE_MSATS)
            }
            other => panic!("unexpected {other}"),
        }))
        .await;
        *identity.lock().unwrap() = Some(me);

        let quote = service
            .fetch_lightning_send_quote(
                &bolt11(payment_hash, None),
                SendQuoteAmount::Payment(100_000),
                None,
            )
            .await
            .unwrap();

        assert_eq!(ssp.operations()[0].1["amount_sats"], 100_000);
        assert_eq!(quote.amount_sats, 100_000);
        assert_eq!(quote.amount_to_send, Some(100_000));
        assert_eq!(quote.total_sats, 101_500);
    }

    fn leaf_to(id: &str, value: u64, receiver: PublicKey) -> (LeafKeyTweak, PublicKey) {
        let node = crate::tree::tests::create_test_tree_node(id, value);
        (
            LeafKeyTweak {
                signing_key: crate::signer::LeafSigningKey {
                    derived_from: node.id.clone(),
                },
                node,
            },
            receiver,
        )
    }

    fn buckets() -> Vec<SendManifestBucket> {
        vec![
            SendManifestBucket {
                receiver_identity_public_key: key(SSP_SEED),
                amount_sats: 100_500,
            },
            SendManifestBucket {
                receiver_identity_public_key: key(PARTNER_SEED),
                amount_sats: 1_000,
            },
        ]
    }

    #[test]
    fn leaves_funding_each_bucket_exactly_pass() {
        let leaves = vec![
            leaf_to("a", 100_000, key(SSP_SEED)),
            leaf_to("b", 500, key(SSP_SEED)),
            leaf_to("c", 1_000, key(PARTNER_SEED)),
        ];
        check_buckets_funded(&buckets(), &leaves).unwrap();
    }

    // The operators enforce exact cover per receiver, so leaves swapped between
    // two receivers must fail here rather than strand at their prepare step.
    #[test]
    fn leaves_misrouted_between_buckets_are_refused() {
        let leaves = vec![
            leaf_to("a", 100_500, key(PARTNER_SEED)),
            leaf_to("c", 1_000, key(SSP_SEED)),
        ];
        assert!(check_buckets_funded(&buckets(), &leaves).is_err());
        let short = vec![leaf_to("a", 100_500, key(SSP_SEED))];
        assert!(check_buckets_funded(&buckets(), &short).is_err());
    }
}
