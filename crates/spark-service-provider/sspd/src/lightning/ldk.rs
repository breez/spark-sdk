/// Nested as siblings because the generated `api` and `events` modules refer to
/// `super::types`.
pub mod proto {
    // Generated code: only a subset of the ldk-server API types are used.
    #![allow(clippy::all, clippy::pedantic, dead_code)]
    pub mod types {
        tonic::include_proto!("types");
    }
    pub mod events {
        tonic::include_proto!("events");
    }
    pub mod api {
        tonic::include_proto!("api");
    }
}

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use bitcoin::hashes::{Hash, HashEngine, Hmac, HmacEngine, sha256};
use bitcoin::secp256k1::PublicKey;
use futures::StreamExt;
use prost::Message;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::wakeup::Wakeup;

use self::proto::{api, types};
use super::node::{
    DecodedInvoice, HeldPayment, IncomingPayment, InvoiceDescription, LightningNode,
    LightningNodeError, LightningPaymentId, OutgoingPayment, PaymentState,
};

const SERVICE_PREFIX: &str = "/api.LightningNode/";
/// The gRPC status ldk-server answers with when a payment failed to send.
const GRPC_STATUS_ABORTED: u32 = 10;
const GRPC_FRAME_HEADER_LEN: usize = 5;
const EVENT_RECONNECT_DELAY: Duration = Duration::from_secs(5);
const SUBSCRIPTION_OPEN_DELAY: Duration = Duration::from_secs(1);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Every call but the event stream, which stays open.
const CALL_TIMEOUT: Duration = Duration::from_secs(60);
/// Pings find a connection that died without closing, the event stream's included.
const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(30);
const KEEP_ALIVE_TIMEOUT: Duration = Duration::from_secs(20);

/// ldk-server does not replay events a subscriber missed, so held payments are also
/// read back from its payment list.
pub const PAID_HOLD_INVOICES_REFRESH_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// How long a payment failed back at its claim deadline stays a hint, for the
/// receive it paid to be cancelled.
const FAILED_BACK_HINT_BLOCKS: u32 = 144;

pub struct LdkServerNode {
    base_url: String,
    api_key: String,
    http: reqwest::Client,
    /// Claim deadlines by payment hash.
    paid_hold_invoices: Mutex<HashMap<[u8; 32], u32>>,
    send_wakeup: Wakeup,
    receive_wakeup: Wakeup,
}

impl LdkServerNode {
    /// `base_url` is `host:port`, without a scheme.
    pub fn new(
        base_url: String,
        api_key: String,
        server_cert_pem: &[u8],
        send_wakeup: Wakeup,
        receive_wakeup: Wakeup,
    ) -> Result<Self, LightningNodeError> {
        let cert = reqwest::Certificate::from_pem(server_cert_pem)
            .map_err(|e| LightningNodeError::Node(format!("invalid server certificate: {e}")))?;
        // The proxy-aware `platform_utils` client can neither trust a custom root
        // certificate nor carry binary bodies, and this connects to the SSP's own
        // ldk-server rather than an SDK endpoint.
        #[allow(clippy::disallowed_methods)]
        let http = reqwest::Client::builder()
            .tls_built_in_root_certs(false)
            .add_root_certificate(cert)
            .http2_prior_knowledge()
            .connect_timeout(CONNECT_TIMEOUT)
            .http2_keep_alive_interval(KEEP_ALIVE_INTERVAL)
            .http2_keep_alive_timeout(KEEP_ALIVE_TIMEOUT)
            .http2_keep_alive_while_idle(true)
            .build()
            .map_err(|e| LightningNodeError::Node(format!("failed to build HTTP client: {e}")))?;
        Ok(Self {
            base_url,
            api_key,
            http,
            paid_hold_invoices: Mutex::new(HashMap::new()),
            send_wakeup,
            receive_wakeup,
        })
    }

    fn auth_header(&self, body: &[u8]) -> String {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let mut engine = HmacEngine::<sha256::Hash>::new(self.api_key.as_bytes());
        engine.input(&timestamp.to_be_bytes());
        engine.input(body);
        let mac = Hmac::<sha256::Hash>::from_engine(engine);
        format!("HMAC {timestamp}:{mac}")
    }

    fn grpc_frame(proto_bytes: &[u8]) -> Result<Vec<u8>, LightningNodeError> {
        let len = u32::try_from(proto_bytes.len())
            .map_err(|_| LightningNodeError::Node("request exceeds gRPC frame size".to_string()))?;
        let mut body = Vec::with_capacity(GRPC_FRAME_HEADER_LEN.saturating_add(proto_bytes.len()));
        body.push(0u8);
        body.extend_from_slice(&len.to_be_bytes());
        body.extend_from_slice(proto_bytes);
        Ok(body)
    }

    async fn call<Rq: Message, Rs: Message + Default>(
        &self,
        method: &str,
        request: &Rq,
    ) -> Result<Rs, LightningNodeError> {
        let body = Self::grpc_frame(&request.encode_to_vec())?;
        let url = format!("https://{}{SERVICE_PREFIX}{method}", self.base_url);
        let auth = self.auth_header(&body);

        let response = self
            .http
            .post(&url)
            .header("content-type", "application/grpc+proto")
            .header("te", "trailers")
            .header("x-auth", auth)
            .body(body)
            .timeout(CALL_TIMEOUT)
            .send()
            .await
            .map_err(|e| {
                LightningNodeError::Node(format!(
                    "{method} request failed: {}",
                    error_chain(&e.without_url())
                ))
            })?;

        // A trailers-only error response carries grpc-status in the headers.
        if let Some(code) = response
            .headers()
            .get("grpc-status")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u32>().ok())
            && code != 0
        {
            let message = response
                .headers()
                .get("grpc-message")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            if code == GRPC_STATUS_ABORTED {
                return Err(LightningNodeError::PaymentSendingFailed(message));
            }
            return Err(LightningNodeError::Node(format!(
                "{method} failed (grpc {code}): {message}"
            )));
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| LightningNodeError::Node(format!("{method} response read failed: {e}")))?;
        let payload = unframe(&bytes).ok_or_else(|| {
            LightningNodeError::Node(format!("{method} returned a malformed frame"))
        })?;
        Rs::decode(payload)
            .map_err(|e| LightningNodeError::Node(format!("{method} response decode failed: {e}")))
    }

    #[cfg(any(test, feature = "test-utils"))]
    pub async fn call_for_test<Rq: Message, Rs: Message + Default>(
        &self,
        method: &str,
        request: &Rq,
    ) -> Result<Rs, LightningNodeError> {
        self.call(method, request).await
    }

    async fn payment(
        &self,
        payment_id: &str,
    ) -> Result<Option<types::Payment>, LightningNodeError> {
        let resp: api::GetPaymentDetailsResponse = self
            .call(
                "GetPaymentDetails",
                &api::GetPaymentDetailsRequest {
                    payment_id: payment_id.to_string(),
                },
            )
            .await?;
        Ok(resp.payment)
    }

    /// `GetPaymentDetails` answers with the outgoing side of a payment the node made
    /// to its own invoice, so the incoming side is then read from the payment list.
    async fn inbound_payment(
        &self,
        payment_hash: &[u8; 32],
    ) -> Result<Option<types::Payment>, LightningNodeError> {
        let id = hex::encode(payment_hash);
        match self.payment(&id).await? {
            Some(payment) if payment.direction() == types::PaymentDirection::Outbound => {
                Ok(self.list_payments().await?.into_iter().find(|payment| {
                    payment.id == id && payment.direction() == types::PaymentDirection::Inbound
                }))
            }
            payment => Ok(payment),
        }
    }

    pub async fn node_id(&self) -> Result<PublicKey, LightningNodeError> {
        let info: api::GetNodeInfoResponse = self
            .call("GetNodeInfo", &api::GetNodeInfoRequest {})
            .await?;
        PublicKey::from_str(&info.node_id)
            .map_err(|e| LightningNodeError::Node(format!("node reported an unreadable id: {e}")))
    }

    pub async fn network(&self) -> Result<bitcoin::Network, LightningNodeError> {
        let info: api::GetNodeInfoResponse = self
            .call("GetNodeInfo", &api::GetNodeInfoRequest {})
            .await?;
        Ok(match info.network() {
            types::Network::Bitcoin => bitcoin::Network::Bitcoin,
            types::Network::Testnet => bitcoin::Network::Testnet,
            types::Network::Testnet4 => bitcoin::Network::Testnet4,
            types::Network::Signet => bitcoin::Network::Signet,
            types::Network::Regtest => bitcoin::Network::Regtest,
        })
    }

    pub async fn run_event_listener(&self, token: CancellationToken) {
        info!("Starting ldk-server event listener");
        loop {
            tokio::select! {
                () = token.cancelled() => {
                    info!("ldk-server event listener cancelled");
                    return;
                }
                result = self.consume_events() => match result {
                    Ok(()) => warn!("ldk-server event stream ended; reconnecting"),
                    Err(e) => warn!("ldk-server event stream failed: {e}; reconnecting"),
                },
            }
            tokio::select! {
                () = token.cancelled() => return,
                () = tokio::time::sleep(EVENT_RECONNECT_DELAY) => {}
            }
        }
    }

    async fn consume_events(&self) -> Result<(), LightningNodeError> {
        let body = Self::grpc_frame(&api::SubscribeEventsRequest {}.encode_to_vec())?;
        let url = format!("https://{}{SERVICE_PREFIX}SubscribeEvents", self.base_url);
        let auth = self.auth_header(&body);
        let subscribe = self
            .http
            .post(&url)
            .header("content-type", "application/grpc+proto")
            .header("te", "trailers")
            .header("x-auth", auth)
            .body(body)
            .send();
        // Events sent while no subscription was open are gone, so what they would
        // have announced is read back once the subscription has had time to open.
        // The subscription's response can wait for the first event, so the read-back
        // runs alongside it.
        let catch_up = async {
            tokio::time::sleep(SUBSCRIPTION_OPEN_DELAY).await;
            if let Err(e) = self.refresh_paid_hold_invoices().await {
                warn!("could not read the payments ldk-server holds: {e}");
            }
            self.send_wakeup.wake();
            self.receive_wakeup.wake();
        };
        let (response, ()) = tokio::join!(subscribe, catch_up);
        let response = response.map_err(|e| {
            LightningNodeError::Node(format!(
                "SubscribeEvents failed: {}",
                error_chain(&e.without_url())
            ))
        })?;
        if let Some(status) = response
            .headers()
            .get("grpc-status")
            .and_then(|v| v.to_str().ok())
            .filter(|status| *status != "0")
        {
            let message = response
                .headers()
                .get("grpc-message")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            return Err(LightningNodeError::Node(format!(
                "SubscribeEvents refused (grpc {status}): {message}"
            )));
        }

        let mut stream = response.bytes_stream();
        let mut buffer: Vec<u8> = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk =
                chunk.map_err(|e| LightningNodeError::Node(format!("event stream error: {e}")))?;
            buffer.extend_from_slice(&chunk);
            while let Some((frame_len, payload)) = take_frame(&buffer) {
                if let Ok(envelope) = proto::events::EventEnvelope::decode(payload) {
                    self.handle_event(envelope);
                }
                buffer.drain(..frame_len);
            }
        }
        Ok(())
    }

    fn handle_event(&self, envelope: proto::events::EventEnvelope) {
        use proto::events::event_envelope::Event;
        let Some(event) = envelope.event else { return };
        match event {
            Event::PaymentClaimable(e) => {
                if let Some(hash) = e.payment.as_ref().and_then(bolt11_hash) {
                    self.lock_paid_hold_invoices()
                        .insert(hash, e.claim_deadline.unwrap_or(u32::MAX));
                }
                self.receive_wakeup.wake();
            }
            Event::PaymentReceived(e) => {
                self.forget_paid_hold_invoice(e.payment.as_ref());
                self.receive_wakeup.wake();
            }
            Event::PaymentSuccessful(_) => self.send_wakeup.wake(),
            Event::PaymentFailed(e) => {
                self.forget_paid_hold_invoice(e.payment.as_ref());
                self.send_wakeup.wake();
                self.receive_wakeup.wake();
            }
            _ => {}
        }
    }

    fn lock_paid_hold_invoices(&self) -> std::sync::MutexGuard<'_, HashMap<[u8; 32], u32>> {
        self.paid_hold_invoices
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn forget_paid_hold_invoice(&self, payment: Option<&types::Payment>) {
        if let Some(hash) = payment.and_then(bolt11_hash) {
            self.lock_paid_hold_invoices().remove(&hash);
        }
    }

    pub async fn run_paid_hold_invoices_refresh(&self, token: CancellationToken) {
        loop {
            tokio::select! {
                () = token.cancelled() => return,
                () = tokio::time::sleep(PAID_HOLD_INVOICES_REFRESH_INTERVAL) => {}
            }
            match self.refresh_paid_hold_invoices().await {
                Ok(()) => self.receive_wakeup.wake(),
                Err(e) => warn!("could not read the payments ldk-server holds: {e}"),
            }
        }
    }

    /// Adds the held payments the node's list still reports as pending, and drops
    /// those whose claim deadline passed [`FAILED_BACK_HINT_BLOCKS`] ago. The node
    /// fails a payment back at its claim deadline without an event.
    async fn refresh_paid_hold_invoices(&self) -> Result<(), LightningNodeError> {
        let held: Vec<([u8; 32], u32)> = self
            .list_payments()
            .await?
            .iter()
            .filter(|payment| {
                payment.direction() == types::PaymentDirection::Inbound
                    && payment.status() == types::PaymentStatus::Pending
            })
            .filter_map(|payment| {
                let bolt11 = bolt11(payment)?;
                Some((decode_hash32(&bolt11.hash)?, bolt11.claim_deadline?))
            })
            .collect();
        let tip = self.current_block_height().await?;
        let mut paid = self.lock_paid_hold_invoices();
        paid.extend(held);
        paid.retain(|_, deadline| deadline.saturating_add(FAILED_BACK_HINT_BLOCKS) > tip);
        Ok(())
    }

    async fn list_payments(&self) -> Result<Vec<types::Payment>, LightningNodeError> {
        let mut payments = Vec::new();
        let mut page_token = None;
        loop {
            let resp: api::ListPaymentsResponse = self
                .call("ListPayments", &api::ListPaymentsRequest { page_token })
                .await?;
            payments.extend(resp.payments);
            match resp.next_page_token {
                Some(next) => page_token = Some(next),
                None => return Ok(payments),
            }
        }
    }
}

fn unframe(bytes: &[u8]) -> Option<&[u8]> {
    take_frame(bytes).map(|(_, payload)| payload)
}

/// The first gRPC frame in `buffer`, with the length of the buffer it spans.
fn take_frame(buffer: &[u8]) -> Option<(usize, &[u8])> {
    let len: [u8; 4] = buffer.get(1..GRPC_FRAME_HEADER_LEN)?.try_into().ok()?;
    let end = GRPC_FRAME_HEADER_LEN.checked_add(usize::try_from(u32::from_be_bytes(len)).ok()?)?;
    Some((end, buffer.get(GRPC_FRAME_HEADER_LEN..end)?))
}

fn bolt11(payment: &types::Payment) -> Option<&types::Bolt11> {
    use proto::types::payment_kind::Kind;
    match payment.kind.as_ref()?.kind.as_ref()? {
        Kind::Bolt11(bolt11) => Some(bolt11),
        _ => None,
    }
}

fn bolt11_preimage(payment: &types::Payment) -> Option<[u8; 32]> {
    decode_hash32(bolt11(payment)?.preimage.as_deref()?)
}

fn bolt11_hash(payment: &types::Payment) -> Option<[u8; 32]> {
    decode_hash32(&bolt11(payment)?.hash)
}

fn decode_hash32(hex_str: &str) -> Option<[u8; 32]> {
    hex::decode(hex_str).ok()?.try_into().ok()
}

fn payment_state(status: types::PaymentStatus) -> PaymentState {
    match status {
        types::PaymentStatus::Pending => PaymentState::Pending,
        types::PaymentStatus::Succeeded => PaymentState::Succeeded,
        types::PaymentStatus::Failed => PaymentState::Failed,
    }
}

#[async_trait]
impl LightningNode for LdkServerNode {
    async fn decode_invoice(&self, invoice: &str) -> Result<DecodedInvoice, LightningNodeError> {
        let resp: api::DecodeInvoiceResponse = self
            .call(
                "DecodeInvoice",
                &api::DecodeInvoiceRequest {
                    invoice: invoice.to_string(),
                },
            )
            .await?;
        let payment_hash = decode_hash32(&resp.payment_hash)
            .ok_or_else(|| LightningNodeError::Decode("invalid payment hash".to_string()))?;
        Ok(DecodedInvoice {
            payment_hash,
            amount_msat: resp.amount_msat,
        })
    }

    async fn pay_invoice(
        &self,
        invoice: &str,
        amount_msat: Option<u64>,
        max_total_cltv_expiry_delta: u32,
        max_routing_fee_msat: Option<u64>,
    ) -> Result<LightningPaymentId, LightningNodeError> {
        let route_parameters = Some(types::RouteParametersConfig {
            max_total_routing_fee_msat: max_routing_fee_msat,
            max_total_cltv_expiry_delta,
            // ldk-server's documented defaults for the remaining knobs.
            max_path_count: 10,
            max_channel_saturation_power_of_half: 2,
        });
        let resp: api::Bolt11SendResponse = self
            .call(
                "Bolt11Send",
                &api::Bolt11SendRequest {
                    invoice: invoice.to_string(),
                    amount_msat,
                    route_parameters,
                },
            )
            .await?;
        Ok(LightningPaymentId(resp.payment_id))
    }

    async fn outgoing_payment(
        &self,
        payment_hash: &[u8; 32],
    ) -> Result<Option<OutgoingPayment>, LightningNodeError> {
        // A BOLT11 payment's id is its payment hash.
        Ok(self
            .payment(&hex::encode(payment_hash))
            .await?
            .filter(|payment| payment.direction() == types::PaymentDirection::Outbound)
            .map(|payment| OutgoingPayment {
                state: payment_state(payment.status()),
                preimage: bolt11_preimage(&payment),
            }))
    }

    async fn create_hold_invoice(
        &self,
        payment_hash: &[u8; 32],
        amount_msat: Option<u64>,
        expiry_secs: u32,
        description: &InvoiceDescription,
        min_final_cltv_expiry_delta: u16,
    ) -> Result<String, LightningNodeError> {
        let kind = match description {
            InvoiceDescription::Memo(memo) => {
                types::bolt11_invoice_description::Kind::Direct(memo.clone())
            }
            InvoiceDescription::Hash(hash) => {
                types::bolt11_invoice_description::Kind::Hash(hex::encode(hash))
            }
        };
        let resp: api::Bolt11ReceiveForHashResponse = self
            .call(
                "Bolt11ReceiveForHash",
                &api::Bolt11ReceiveForHashRequest {
                    amount_msat,
                    description: Some(types::Bolt11InvoiceDescription { kind: Some(kind) }),
                    expiry_secs,
                    payment_hash: hex::encode(payment_hash),
                    min_final_cltv_expiry_delta: Some(u32::from(min_final_cltv_expiry_delta)),
                },
            )
            .await?;
        Ok(resp.invoice)
    }

    fn paid_hold_invoices(&self) -> Vec<[u8; 32]> {
        self.lock_paid_hold_invoices().keys().copied().collect()
    }

    async fn incoming_payment(
        &self,
        payment_hash: &[u8; 32],
    ) -> Result<Option<IncomingPayment>, LightningNodeError> {
        Ok(self
            .inbound_payment(payment_hash)
            .await?
            .map(|payment| IncomingPayment {
                state: payment_state(payment.status()),
                // `claimable_amount_msat` is what arrived: an amountless invoice's
                // payment has no other amount.
                held: bolt11(&payment).and_then(|bolt11| {
                    Some(HeldPayment {
                        amount_msat: bolt11.claimable_amount_msat.or(payment.amount_msat)?,
                        claim_deadline: bolt11.claim_deadline?,
                    })
                }),
            }))
    }

    async fn current_block_height(&self) -> Result<u32, LightningNodeError> {
        let resp: api::GetNodeInfoResponse = self
            .call("GetNodeInfo", &api::GetNodeInfoRequest {})
            .await?;
        resp.current_best_block
            .map(|block| block.height)
            .ok_or_else(|| LightningNodeError::Node("node reported no best block".to_string()))
    }

    async fn settle_hold_invoice(
        &self,
        payment_hash: &[u8; 32],
        preimage: &[u8; 32],
    ) -> Result<(), LightningNodeError> {
        let payment_id = hex::encode(payment_hash);
        let request = api::Bolt11ClaimForHashRequest {
            payment_hash: Some(payment_id.clone()),
            claimable_amount_msat: None,
            preimage: hex::encode(preimage),
        };
        match self
            .call::<_, api::Bolt11ClaimForHashResponse>("Bolt11ClaimForHash", &request)
            .await
        {
            Ok(_) => Ok(()),
            Err(e) => match self.inbound_payment(payment_hash).await {
                Ok(Some(payment)) if payment.status() == types::PaymentStatus::Succeeded => Ok(()),
                _ => Err(e),
            },
        }
    }

    async fn cancel_hold_invoice(&self, payment_hash: &[u8; 32]) -> Result<(), LightningNodeError> {
        let payment_id = hex::encode(payment_hash);
        let request = api::Bolt11FailForHashRequest {
            payment_hash: payment_id.clone(),
        };
        match self
            .call::<_, api::Bolt11FailForHashResponse>("Bolt11FailForHash", &request)
            .await
        {
            Ok(_) => Ok(()),
            Err(e) => match self.inbound_payment(payment_hash).await {
                Ok(Some(payment)) if payment.status() == types::PaymentStatus::Failed => Ok(()),
                _ => Err(e),
            },
        }
    }
}

/// reqwest's own message for a transport failure is only "error sending request for
/// url (...)", without the cause underneath it.
fn error_chain(error: &(dyn std::error::Error + 'static)) -> String {
    use std::fmt::Write;

    let mut out = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let _ = write!(out, ": {cause}");
        source = cause.source();
    }
    out
}
