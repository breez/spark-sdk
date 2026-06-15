//! Boltz reverse-swap cross-chain provider.
//!
//! Implements [`CrossChainService`] for Boltz's sats → USDT reverse swap.
//! Routing/quoting happens via the inner [`boltz_client::BoltzService`];
//! payment rows are written at send time only (after the lightning leg
//! succeeds) and updated silently by [`super::boltz_event_listener`] as the
//! WebSocket drives the swap to a terminal state.

use std::sync::Arc;
use std::time::Duration;

use boltz_client::{
    BoltzError, BoltzService as BoltzClient,
    config::{BoltzConfig as BoltzClientConfig, MAX_SLIPPAGE_BPS},
    models::{Asset, PreparedSwap},
};
use platform_utils::time::{SystemTime, UNIX_EPOCH};
use spark_wallet::SparkWallet;
use tracing::{debug, error, info};

use super::{
    CrossChainFeeMode, CrossChainPrepared, CrossChainProvider, CrossChainProviderContext,
    CrossChainRouteFilter, CrossChainRoutePair, CrossChainService, SourceAsset,
    derive_btc_leg_transfer_id,
};
use crate::{
    ConversionInfo, ConversionStatus, Network, PaymentMetadata, PaymentStatus, Storage,
    error::SdkError,
    sdk::LightningSender,
    utils::{
        payments::insert_or_cache_payment_metadata,
        polling::{PollSchedule, poll_until},
    },
};

// Polling cadence for the outbound LN payment leg waiting for terminal status
// after `lightning_sender::pay_and_persist_lightning_invoice` returns and its
// background SSP poll runs.
const SEND_POLL_INITIAL_DELAY_MS: u64 = 500;
const SEND_POLL_MAX_DELAY_MS: u64 = 2000;
const SEND_POLL_TIMEOUT_SECS: u64 = 60;

/// Hardened derivation index reserved for encrypting the Boltz instance handle
/// at rest. `1112430164` == ASCII "BOLT", distinct from the session store's
/// "SESN" path, `RTSyncSigner`'s indices, and the `KeySet` master keys, so this
/// scope can never collide with another subsystem deriving from the same
/// identity master key. No per-network variant is needed: the signer's
/// `identity_master_key` is already derived under a network-specific account
/// number, so mainnet and regtest yield distinct encryption keys regardless.
const BOLTZ_INSTANCE_ENCRYPTION_PATH: &str = "m/1112430164'/0'/0'/0/0";

#[derive(serde::Serialize, serde::Deserialize)]
struct BoltzInstanceHandle {
    instance_id: String,
    seed_hex: String,
}

/// Loads or generates the device-local Boltz instance handle (random 32-byte
/// seed + instance id). The seed is a long-lived secret that derives the Boltz
/// swap claim/refund keys, so the serialized handle is encrypted at rest via
/// the signer (ECIES under a dedicated derivation path): an attacker with
/// read-only storage access never sees the seed in cleartext.
///
/// The seed is random rather than derived from the wallet identity so two
/// devices restored from the same mnemonic never share a Boltz instance seed.
///
/// In v1 this is kept local only. Cross-device recovery of swaps lands with
/// the v2 submarine-swap feature.
///
/// Cross-device consequence in v1: a user who restores from mnemonic on a
/// second device cannot claim destination-chain payouts for reverse swaps
/// initiated on the first device. Funds are not at risk (Boltz's hold-invoice
/// timeout refunds the lightning leg), but the second device is blind to the
/// in-flight swap until it terminates on Boltz's side. v2 is expected to
/// retroactively publish the existing local seed on first boot so new devices
/// can bootstrap from rtsync.
async fn load_or_create_boltz_instance(
    storage: &Arc<dyn Storage>,
    signer: &Arc<dyn crate::signer::BreezSigner>,
) -> Result<BoltzInstanceHandle, SdkError> {
    use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
    use bitcoin::bip32::DerivationPath;
    use bitcoin::secp256k1::rand::{RngCore, thread_rng};

    const BOLTZ_INSTANCE_KEY: &str = "boltz_instance_current";

    let encryption_path: DerivationPath = BOLTZ_INSTANCE_ENCRYPTION_PATH
        .parse()
        .map_err(|e| SdkError::Generic(format!("Invalid Boltz instance encryption path: {e}")))?;

    if let Some(raw) = storage
        .get_cached_item(BOLTZ_INSTANCE_KEY.to_string())
        .await
        .map_err(|e| SdkError::Generic(format!("Failed to read Boltz instance: {e}")))?
    {
        // A decrypt or parse failure here means the stored blob predates
        // encryption-at-rest (or is otherwise unreadable). The seed is
        // device-local and regenerable, so we fall through and mint a fresh
        // one rather than failing connect; the only cost is abandoning any
        // swap in flight on this device.
        match decrypt_boltz_instance(&raw, signer, &encryption_path).await {
            Ok(handle) => return Ok(handle),
            Err(e) => debug!("Discarding unreadable Boltz instance handle, regenerating: {e}"),
        }
    }

    let mut seed = [0u8; 32];
    thread_rng().fill_bytes(&mut seed);
    let handle = BoltzInstanceHandle {
        instance_id: uuid::Uuid::new_v4().to_string(),
        seed_hex: hex::encode(seed),
    };
    let serialized = serde_json::to_vec(&handle)
        .map_err(|e| SdkError::Generic(format!("Failed to serialize Boltz instance: {e}")))?;
    let ciphertext = signer
        .encrypt_ecies(&serialized, &encryption_path)
        .await
        .map_err(|e| SdkError::Generic(format!("Failed to encrypt Boltz instance: {e}")))?;
    storage
        .set_cached_item(BOLTZ_INSTANCE_KEY.to_string(), BASE64.encode(ciphertext))
        .await
        .map_err(|e| SdkError::Generic(format!("Failed to persist Boltz instance: {e}")))?;
    Ok(handle)
}

async fn decrypt_boltz_instance(
    raw: &str,
    signer: &Arc<dyn crate::signer::BreezSigner>,
    encryption_path: &bitcoin::bip32::DerivationPath,
) -> Result<BoltzInstanceHandle, SdkError> {
    use base64::{Engine, engine::general_purpose::STANDARD as BASE64};

    let ciphertext = BASE64
        .decode(raw.as_bytes())
        .map_err(|e| SdkError::Generic(format!("Invalid base64 Boltz instance: {e}")))?;
    let plaintext = signer.decrypt_ecies(&ciphertext, encryption_path).await?;
    serde_json::from_slice(&plaintext)
        .map_err(|e| SdkError::Generic(format!("Corrupted Boltz instance handle: {e}")))
}

pub(crate) struct BoltzService {
    client: Arc<BoltzClient>,
    spark_wallet: Arc<SparkWallet>,
    storage: Arc<dyn Storage>,
    /// Shared helper that owns the "pay LN invoice + persist Payment row +
    /// poll SSP" sequence. Reused by `send_bolt11_invoice` on the SDK and
    /// by this provider so Boltz hold-invoice pays behave identically to
    /// direct LN sends.
    lightning_sender: Arc<LightningSender>,
}

impl BoltzService {
    /// Construct the SDK-side wrapper. Does not perform I/O; the caller is
    /// expected to construct the inner [`BoltzClient`] (which owns the
    /// WebSocket + background monitor) and pass it in already initialized.
    pub(crate) fn new(
        client: Arc<BoltzClient>,
        spark_wallet: Arc<SparkWallet>,
        storage: Arc<dyn Storage>,
        lightning_sender: Arc<LightningSender>,
    ) -> Self {
        info!("Boltz service initialized");
        Self {
            client,
            spark_wallet,
            storage,
            lightning_sender,
        }
    }

    /// Best-effort helper to build a boltz-client [`BoltzClientConfig`] for
    /// the given network. Returns `None` on non-mainnet networks since Boltz
    /// only supports mainnet today.
    pub(crate) fn default_client_config(network: Network) -> Option<BoltzClientConfig> {
        const BREEZ_REFERRAL_ID: &str = "breez-sdk";
        match network {
            Network::Mainnet => Some(BoltzClientConfig::mainnet(BREEZ_REFERRAL_ID.to_string())),
            Network::Regtest => None,
        }
    }

    /// Initializes the Boltz reverse-swap cross-chain provider: loads or creates
    /// the local instance seed, constructs the inner [`BoltzClient`], registers
    /// the event listener, resumes any active swaps, and returns an SDK-side
    /// wrapper ready to be inserted into the provider registry. Returns `None`
    /// when the network has no default configuration.
    pub(crate) async fn build(
        network: Network,
        spark_wallet: Arc<SparkWallet>,
        storage: Arc<dyn Storage>,
        signer: Arc<dyn crate::signer::BreezSigner>,
        lightning_sender: Arc<LightningSender>,
    ) -> Result<Option<Arc<dyn CrossChainService>>, SdkError> {
        let Some(client_config) = Self::default_client_config(network) else {
            return Ok(None);
        };

        let handle = load_or_create_boltz_instance(&storage, &signer).await?;
        let seed = hex::decode(&handle.seed_hex)
            .map_err(|e| SdkError::Generic(format!("Invalid Boltz instance seed hex: {e}")))?;

        let adapter = Arc::new(super::boltz_storage_adapter::BoltzStorageAdapter::new(
            Arc::clone(&storage),
            handle.instance_id.clone(),
        ));

        let client = Arc::new(
            BoltzClient::new(client_config, &seed, adapter)
                .await
                .map_err(|e| SdkError::Generic(format!("Failed to construct Boltz client: {e}")))?,
        );

        let listener = Box::new(super::boltz_event_listener::BoltzSdkEventListener::new(
            Arc::clone(&storage),
        ));
        client.add_event_listener(listener).await;

        if let Err(e) = client.resume_swaps().await {
            tracing::warn!("Boltz resume_swaps failed on startup: {e:?}");
        }

        // Defense-in-depth: heal any conversion whose terminal swap event was
        // dropped (see `reconcile_pending_boltz_conversions`). Spawned so a
        // large payment history doesn't add latency to connect; it only reads
        // local storage and the local swap rows.
        platform_utils::tokio::spawn({
            let client = Arc::clone(&client);
            let storage = Arc::clone(&storage);
            async move {
                super::boltz_event_listener::reconcile_pending_boltz_conversions(&client, &storage)
                    .await;
            }
        });

        Ok(Some(Arc::new(Self::new(
            client,
            spark_wallet,
            storage,
            lightning_sender,
        ))))
    }

    /// One-shot prepare for `FeesExcluded`: `amount` is the provider invoice
    /// target. The wallet pays `amount + ln_fee_sats` in total at send time.
    async fn prepare_fees_excluded(
        &self,
        recipient_address: &str,
        route: &CrossChainRoutePair,
        chain: &str,
        asset: Asset,
        invoice_amount_sats: u64,
        max_slippage_bps: Option<u32>,
    ) -> Result<CrossChainPrepared, SdkError> {
        debug!(
            "Boltz: preparing reverse swap (FeesExcluded) to {recipient_address} on {}, amount {invoice_amount_sats} sats",
            route.chain
        );

        let (prepared, created) = self
            .create_swap(
                recipient_address,
                chain,
                asset,
                invoice_amount_sats,
                max_slippage_bps,
            )
            .await?;

        let ln_fee_sats = self.fetch_ln_fee(&created.invoice).await?;

        Ok(Self::build_prepared(
            route,
            recipient_address,
            &prepared,
            created,
            ln_fee_sats,
            max_slippage_bps,
            CrossChainFeeMode::FeesExcluded,
        ))
    }

    /// Two-phase prepare for `FeesIncluded`: size the real invoice to
    /// `amount - ln_fee_probe_sats` so `invoice_sats + ln_fee_probe <= amount`.
    ///
    /// Mirrors LNURL pay's `FeesIncluded` pattern at `lnurl.rs`. Phase 1 uses
    /// boltz-client's lightweight probe-invoice API — random preimage, short
    /// server-side expiry, no HD index burn / DB row / WS subscription — so
    /// the probe doesn't pile up persistent state. The probe value is stored
    /// on the prepare as `source_transfer_fee_sats` and enforced as a hard
    /// cap at send time.
    async fn prepare_fees_included(
        &self,
        recipient_address: &str,
        route: &CrossChainRoutePair,
        chain: &str,
        asset: Asset,
        total_sats: u64,
        max_slippage_bps: Option<u32>,
    ) -> Result<CrossChainPrepared, SdkError> {
        debug!(
            "Boltz: preparing reverse swap (FeesIncluded) to {recipient_address} on {}, total {total_sats} sats",
            route.chain
        );

        // Phase 1: throwaway probe invoice at `total_sats` to probe LN fee.
        let probe_invoice = self
            .fetch_probe_invoice(
                recipient_address,
                chain,
                asset,
                total_sats,
                max_slippage_bps,
            )
            .await?;
        let ln_fee_probe_sats = self.fetch_ln_fee(&probe_invoice).await?;

        let real_invoice_sats = fees_included_real_invoice_sats(total_sats, ln_fee_probe_sats)?;

        // Phase 2: real invoice sized to leave room for the probed fee.
        // Override `AmountOutOfRange` with a message that names the user's
        // `total_sats` and the probed fee — the raw Boltz error references
        // `real_invoice_sats`, a number the caller never chose. The phase-2
        // prepare validates against the Boltz pair limits before
        // `create_reverse_swap` is called, so a failure here commits no state.
        let prepared = self
            .client
            .prepare_reverse_swap_from_sats(
                recipient_address,
                chain,
                asset,
                real_invoice_sats,
                max_slippage_bps,
            )
            .await
            .map_err(|e| match &e {
                BoltzError::AmountOutOfRange { min, .. } => SdkError::InvalidInput(format!(
                    "Amount {total_sats} sats too small for cross-chain send: \
                    after subtracting LN fee ({ln_fee_probe_sats} sats), the remaining \
                    invoice ({real_invoice_sats} sats) is below Boltz minimum ({min} sats)."
                )),
                _ => e.into(),
            })?;
        let created = self.client.create_reverse_swap(&prepared).await?;
        let ln_fee_final_sats = self.fetch_ln_fee(&created.invoice).await?;

        // Mirrors LNURL's guard: if fee moved between queries, fail so caller retries.
        if ln_fee_final_sats > ln_fee_probe_sats {
            return Err(SdkError::Generic(
                "Boltz LN fee increased between prepare queries. Please retry.".to_string(),
            ));
        }

        // Store the probe (not the final) as the budget — matches LNURL's
        // `fee_sats = first_fee` and keeps `invoice_sats + max_fee <= amount`.
        Ok(Self::build_prepared(
            route,
            recipient_address,
            &prepared,
            created,
            ln_fee_probe_sats,
            max_slippage_bps,
            CrossChainFeeMode::FeesIncluded,
        ))
    }

    async fn create_swap(
        &self,
        recipient_address: &str,
        chain: &str,
        asset: Asset,
        invoice_amount_sats: u64,
        max_slippage_bps: Option<u32>,
    ) -> Result<(PreparedSwap, boltz_client::models::CreatedSwap), SdkError> {
        let prepared: PreparedSwap = self
            .client
            .prepare_reverse_swap_from_sats(
                recipient_address,
                chain,
                asset,
                invoice_amount_sats,
                max_slippage_bps,
            )
            .await?;

        // `create_reverse_swap` commits a HD key index, POSTs to Boltz to
        // create the swap on the server, and writes a `BoltzSwap` row into
        // the adapter cache KV. After this call Boltz is holding the swap
        // state, so the only path back to a clean state is a timeout.
        let created = self.client.create_reverse_swap(&prepared).await?;

        Ok((prepared, created))
    }

    /// Fetch a throwaway hold invoice for LN-fee estimation only.
    ///
    /// Both calls in this path are stateless on our side:
    /// - `prepare_reverse_swap_from_sats` is a pure quote (HTTP fetch only,
    ///   no HD index burn, no DB row, no WS subscription).
    /// - `create_probe_invoice` uses a random preimage, sets a short
    ///   server-side expiry, and likewise does not increment the HD key
    ///   index, persist to local storage, or open a WS subscription.
    ///
    /// The returned invoice MUST NOT be paid: the preimage is discarded,
    /// so any payment cannot be claimed.
    async fn fetch_probe_invoice(
        &self,
        recipient_address: &str,
        chain: &str,
        asset: Asset,
        invoice_amount_sats: u64,
        max_slippage_bps: Option<u32>,
    ) -> Result<String, SdkError> {
        let prepared: PreparedSwap = self
            .client
            .prepare_reverse_swap_from_sats(
                recipient_address,
                chain,
                asset,
                invoice_amount_sats,
                max_slippage_bps,
            )
            .await?;

        self.client
            .create_probe_invoice(&prepared)
            .await
            .map_err(Into::into)
    }

    async fn fetch_ln_fee(&self, invoice: &str) -> Result<u64, SdkError> {
        self.spark_wallet
            .fetch_lightning_send_fee_estimate(invoice, None)
            .await
            .map_err(|e| {
                SdkError::Generic(format!(
                    "Failed to fetch lightning send fee estimate for Boltz invoice: {e}"
                ))
            })
    }

    #[allow(clippy::too_many_arguments)]
    fn build_prepared(
        route: &CrossChainRoutePair,
        recipient_address: &str,
        prepared: &PreparedSwap,
        created: boltz_client::models::CreatedSwap,
        ln_fee_sats: u64,
        max_slippage_bps: Option<u32>,
        fee_mode: CrossChainFeeMode,
    ) -> CrossChainPrepared {
        // `fee_amount` is the Boltz spread only (invoice sats - on-chain sats
        // paid out). The LN routing budget is exposed separately as
        // `source_transfer_fee_sats` — not double-counted here.
        let boltz_spread_sats = created
            .invoice_amount_sats
            .saturating_sub(prepared.estimated_onchain_amount);
        let fee_amount = u128::from(boltz_spread_sats);
        let estimated_out = u128::from(prepared.output_amount);
        let invoice_amount_sats = created.invoice_amount_sats;
        let resolved_slippage = max_slippage_bps.unwrap_or(prepared.slippage_bps);

        let provider_context = CrossChainProviderContext::Boltz {
            swap_id: created.swap_id.clone(),
            invoice: created.invoice,
            max_slippage_bps: resolved_slippage,
        };

        CrossChainPrepared {
            amount_in: u128::from(invoice_amount_sats),
            estimated_out,
            fee_amount,
            fee_asset: None,
            source_transfer_fee_sats: ln_fee_sats,
            fee_mode,
            expires_at: prepared.expires_at.to_string(),
            pair: route.clone(),
            recipient_address: recipient_address.to_string(),
            token_identifier: None,
            provider_context,
        }
    }
}

#[macros::async_trait]
#[allow(clippy::too_many_lines)]
impl CrossChainService for BoltzService {
    async fn get_routes(
        &self,
        filter: &CrossChainRouteFilter,
    ) -> Result<Vec<CrossChainRoutePair>, SdkError> {
        let address_details = match filter {
            CrossChainRouteFilter::Send { address_details } => address_details,
            // v1 Boltz is reverse-swap only (BTC/sats -> external). Submarine
            // swaps (USDT -> LN) are out of scope for v1 and will populate
            // this branch when they land.
            CrossChainRouteFilter::Receive { .. } => return Ok(Vec::new()),
        };

        // `destinations_accepting` validates the raw recipient address against
        // every destination's transport and returns only those whose parser
        // accepts it. This automatically picks up every supported asset/chain
        // /bridge combination (USDT0 via OFT, USDC via CCTP, Arbitrum-direct).
        let routes = self
            .client
            .destinations_accepting(&address_details.address)
            .iter()
            .map(destination_to_route_pair)
            .collect();
        Ok(routes)
    }

    async fn prepare(
        &self,
        recipient_address: &str,
        route: &CrossChainRoutePair,
        amount: u128,
        token_identifier: Option<String>,
        max_slippage_bps: u32,
        fee_mode: CrossChainFeeMode,
    ) -> Result<CrossChainPrepared, SdkError> {
        // v1 Boltz is BTC-only. Tokens must be rejected before we commit any
        // state on Boltz's side.
        if token_identifier.is_some() {
            return Err(SdkError::InvalidInput(
                "Boltz does not support token sends in v1".to_string(),
            ));
        }

        let total_sats = u64::try_from(amount).map_err(|_| {
            SdkError::InvalidInput(format!(
                "Amount {amount} exceeds u64::MAX sats for Boltz reverse swap"
            ))
        })?;

        // The route carries the destination's orthogonal `(chain, asset)`
        // identity; Boltz selects by that pair, so map the asset ticker back to
        // its enum and pass both through (no opaque destination handle).
        let asset = Asset::try_from(route.asset.as_str()).map_err(|()| {
            SdkError::InvalidInput(format!(
                "Boltz does not support asset '{}' on {}",
                route.asset, route.chain
            ))
        })?;

        if max_slippage_bps > MAX_SLIPPAGE_BPS {
            return Err(SdkError::InvalidInput(format!(
                "max_slippage_bps {max_slippage_bps} exceeds Boltz maximum {MAX_SLIPPAGE_BPS}"
            )));
        }

        let slippage = Some(max_slippage_bps);
        match fee_mode {
            CrossChainFeeMode::FeesExcluded => {
                self.prepare_fees_excluded(
                    recipient_address,
                    route,
                    &route.chain,
                    asset,
                    total_sats,
                    slippage,
                )
                .await
            }
            CrossChainFeeMode::FeesIncluded => {
                self.prepare_fees_included(
                    recipient_address,
                    route,
                    &route.chain,
                    asset,
                    total_sats,
                    slippage,
                )
                .await
            }
        }
    }

    #[allow(clippy::large_futures)]
    async fn send(
        &self,
        prepared: &CrossChainPrepared,
        idempotency_key: Option<String>,
    ) -> Result<crate::Payment, SdkError> {
        let CrossChainProviderContext::Boltz {
            swap_id,
            invoice,
            max_slippage_bps,
        } = &prepared.provider_context
        else {
            return Err(SdkError::Generic(
                "Boltz send called with non-Boltz provider context".to_string(),
            ));
        };

        validate_quote_expiry(&prepared.expires_at)?;

        let transfer_id = Some(derive_btc_leg_transfer_id(
            idempotency_key.as_deref(),
            &format!("cross_chain:boltz:{swap_id}"),
        )?);

        let invoice_amount_sats = u64::try_from(prepared.amount_in)
            .map_err(|e| SdkError::Generic(format!("Boltz invoice amount exceeds u64: {e}")))?;

        let ln_fee_budget = prepared.source_transfer_fee_sats;

        // Compute the LN payment amount based on fee_mode. For FeesIncluded,
        // mirror LNURL's overpayment logic so the wallet actually consumes the
        // user's budget when current_fee < ln_fee_probe.
        let ln_amount_sats = match prepared.fee_mode {
            CrossChainFeeMode::FeesExcluded => {
                // Pay the invoice as-is; `max_fee_sat = ln_fee_budget` protects
                // against fee drift (validated downstream).
                None
            }
            CrossChainFeeMode::FeesIncluded => {
                let current_fee = self
                    .spark_wallet
                    .fetch_lightning_send_fee_estimate(invoice, None)
                    .await
                    .map_err(|e| {
                        SdkError::Generic(format!(
                            "Failed to re-estimate Boltz LN fee at send: {e}"
                        ))
                    })?;

                let overpayment = crate::utils::fees::fee_overpayment(ln_fee_budget, current_fee)?;

                Some(invoice_amount_sats.saturating_add(overpayment))
            }
        };

        // Delegate the LN leg to the shared helper. It pays the hold
        // invoice, builds the Payment row, persists it, and spawns SSP-side
        // polling — the same path `send_bolt11_invoice` takes. On failure
        // the hold invoice eventually times out on Boltz's side; no payment
        // row is written, so there is nothing to reconcile on ours.
        let sdk_payment = self
            .lightning_sender
            .pay_and_persist_lightning_invoice(
                invoice,
                ln_amount_sats,
                ln_fee_budget,
                false,
                prepared.amount_in,
                transfer_id,
                0,
            )
            .await
            .map_err(|e| SdkError::Generic(format!("Boltz lightning payment failed: {e}")))?;
        let spark_payment_id = sdk_payment.id.clone();

        debug!("Boltz: lightning payment {spark_payment_id} sent for swap {swap_id}");

        let conversion_info = ConversionInfo::Boltz {
            swap_id: swap_id.clone(),
            chain: prepared.pair.chain.clone(),
            chain_id: prepared.pair.chain_id.clone(),
            asset: prepared.pair.asset.clone(),
            recipient_address: prepared.recipient_address.clone(),
            invoice: invoice.clone(),
            invoice_amount_sats,
            estimated_out: prepared.estimated_out,
            delivered_amount: None,
            bridge_ref: None,
            status: ConversionStatus::Pending,
            fee: Some(prepared.fee_amount),
            max_slippage_bps: *max_slippage_bps,
            quote_degraded: false,
            asset_decimals: u32::from(prepared.pair.decimals),
            asset_contract: prepared.pair.contract_address.clone(),
        };
        let metadata = PaymentMetadata {
            conversion_info: Some(conversion_info.clone()),
            ..Default::default()
        };

        let payment_id = insert_or_cache_payment_metadata(
            &spark_payment_id,
            metadata,
            &self.spark_wallet,
            &self.storage,
            true,
        )
        .await
        .unwrap_or_else(|e| {
            error!("Failed to persist Boltz metadata for payment {spark_payment_id}: {e:?}");
            spark_payment_id.clone()
        });

        // Read-after-write reconcile. The boltz-client WS task drives the swap
        // independently and may have reached a terminal state before (or
        // during) the `ConversionInfo` write above. Such a terminal
        // `SwapUpdated` event would have been dropped by the listener (no
        // `ConversionInfo` to update yet), and `resume_swaps` won't replay it
        // (terminal swaps are pruned from the active set). boltz-client
        // persists the terminal swap row *before* emitting the event, and this
        // read is sequenced after the metadata write, so no terminal transition
        // can slip through: it is either visible here, or it lands after the
        // write and the WS event finds the `ConversionInfo`.
        match self.client.get_swap(swap_id).await {
            Ok(Some(swap)) if swap.status.is_terminal() => {
                if let Some(updated) =
                    super::boltz_event_listener::boltz_metadata_from_swap(conversion_info, &swap)
                {
                    match self
                        .storage
                        .insert_payment_metadata(payment_id.clone(), updated)
                        .await
                    {
                        Ok(()) => info!(
                            "Boltz: swap {swap_id} already terminal at send; applied {:?} to payment {payment_id}",
                            swap.status
                        ),
                        Err(e) => error!(
                            "Boltz: failed to persist send-time terminal update for {payment_id}: {e:?}"
                        ),
                    }
                }
            }
            Ok(_) => {}
            Err(e) => debug!("Boltz: read-after-write get_swap({swap_id}) failed: {e:?}"),
        }

        // `lightning_sender::pay_and_persist_lightning_invoice` returns
        // immediately with a Pending payment and spawns a background SSP
        // poll. Wait here for storage to surface a terminal status so we can
        // return a terminal `Payment` to the caller. If the timeout fires
        // we surface the pending payment we already have — the background
        // poll continues and will emit a `PaymentSucceeded` event later.
        let schedule = PollSchedule {
            initial_delay: Duration::from_millis(SEND_POLL_INITIAL_DELAY_MS),
            max_delay: Duration::from_millis(SEND_POLL_MAX_DELAY_MS),
            timeout: Duration::from_secs(SEND_POLL_TIMEOUT_SECS),
        };
        Ok(poll_to_terminal_or_fallback(
            Arc::clone(&self.storage),
            payment_id,
            sdk_payment,
            schedule,
        )
        .await)
    }
}

/// Polls storage for a terminal status on `payment_id`. Returns the terminal
/// `Payment` on success; on timeout or storage error returns `fallback` (the
/// pending payment we already have in hand). The background SSP poll continues
/// after we return.
async fn poll_to_terminal_or_fallback(
    storage: Arc<dyn Storage>,
    payment_id: String,
    fallback: crate::Payment,
    schedule: PollSchedule,
) -> crate::Payment {
    let polled = poll_until(schedule, None, || {
        let storage = Arc::clone(&storage);
        let payment_id = payment_id.clone();
        async move {
            match storage.get_payment_by_id(payment_id.clone()).await {
                Ok(payment) if payment.status != PaymentStatus::Pending => Ok(Some(payment)),
                Ok(_) => Ok(None),
                Err(e) => Err(SdkError::Generic(format!(
                    "Failed to fetch Boltz payment {payment_id}: {e}"
                ))),
            }
        }
    })
    .await;

    match polled {
        Ok(payment) => payment,
        Err(e) => {
            debug!(
                "Boltz: terminal status not reached within timeout: {e}; returning pending payment"
            );
            fallback
        }
    }
}

/// Boltz quotes carry `expires_at` as a Unix epoch seconds string. Reject
/// at send time if the wall clock has passed it so the user sees a clean
/// "quote expired, re-prepare" rather than a server-side error after the LN
/// pay attempt.
fn validate_quote_expiry(expires_at: &str) -> Result<(), SdkError> {
    let exp_secs: u64 = expires_at
        .parse()
        .map_err(|e| SdkError::Generic(format!("Boltz: invalid expires_at {expires_at:?}: {e}")))?;
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| SdkError::Generic("Failed to read current time".to_string()))?
        .as_secs();
    if now_secs >= exp_secs {
        return Err(SdkError::InvalidInput(
            "Cross-chain quote has expired. Please re-prepare.".to_string(),
        ));
    }
    Ok(())
}

/// Phase-1 check for the `FeesIncluded` path: returns the size to use for the
/// real invoice, or rejects if the probed LN fee already eats the budget.
fn fees_included_real_invoice_sats(
    total_sats: u64,
    ln_fee_probe_sats: u64,
) -> Result<u64, SdkError> {
    if total_sats <= ln_fee_probe_sats {
        return Err(SdkError::InvalidInput(format!(
            "Amount too small for cross-chain send: {total_sats} sats <= LN fee {ln_fee_probe_sats} sats."
        )));
    }
    Ok(total_sats.saturating_sub(ln_fee_probe_sats))
}

/// Build a [`CrossChainRoutePair`] from a Boltz [`DestinationOption`].
///
/// Mirrors Orchestra's orthogonal model: `chain` is the human chain label
/// (`"Arbitrum One"`, `"Base"`, `"Solana"`) and `asset` the delivered
/// stablecoin (`"USDT"` / `"USDT0"` / `"USDC"`). The `(chain, asset)` pair is
/// the destination identity Boltz selects by at prepare time.
///
/// `chain_id` (EVM chain id as a decimal string) and `contract_address` (the
/// destination token contract) come from the destination's `evm_chain_id` /
/// `dest_token_address`; non-EVM transports (Solana, Tron) expose no chain id.
fn destination_to_route_pair(
    dest: &boltz_client::models::DestinationOption,
) -> CrossChainRoutePair {
    CrossChainRoutePair {
        provider: CrossChainProvider::Boltz,
        chain: dest.chain_label.clone(),
        chain_id: dest.evm_chain_id.map(|id| id.to_string()),
        asset: dest.asset.as_str().to_string(),
        contract_address: dest.dest_token_address.clone(),
        decimals: 6,
        exact_out_eligible: false,
        supported_sources: vec![SourceAsset::Bitcoin],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use boltz_client::models::{Asset, BridgeKind, DestinationOption, NetworkTransport};
    use macros::test_all;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    fn test_destination(
        chain_label: &str,
        asset: Asset,
        transport: NetworkTransport,
        evm_chain_id: Option<u64>,
        dest_token_address: Option<&str>,
        bridge_kind: BridgeKind,
    ) -> DestinationOption {
        DestinationOption {
            chain_label: chain_label.to_string(),
            asset,
            transport,
            evm_chain_id,
            dest_token_address: dest_token_address.map(str::to_string),
            bridge_kind,
        }
    }

    #[test_all]
    fn destination_to_pair_maps_evm_chain_id_to_decimal_string() {
        let dest = test_destination(
            "Polygon PoS",
            Asset::Usdt0,
            NetworkTransport::Evm,
            Some(137),
            Some("0xtoken"),
            BridgeKind::Oft,
        );
        let pair = destination_to_route_pair(&dest);

        assert_eq!(pair.provider, CrossChainProvider::Boltz);
        assert_eq!(
            pair.chain_id,
            Some("137".to_string()),
            "EVM chain id should render as a decimal string"
        );
        assert_eq!(pair.chain, "Polygon PoS", "chain carries the human label");
        assert_eq!(pair.asset, "USDT0");
        assert_eq!(pair.contract_address.as_deref(), Some("0xtoken"));
        assert_eq!(pair.decimals, 6);
        assert!(!pair.exact_out_eligible);
    }

    #[test_all]
    fn destination_to_pair_surfaces_usdc_asset() {
        let dest = test_destination(
            "Base",
            Asset::Usdc,
            NetworkTransport::Evm,
            Some(8453),
            Some("0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"),
            BridgeKind::Cctp,
        );
        let pair = destination_to_route_pair(&dest);

        assert_eq!(pair.asset, "USDC");
        assert_eq!(pair.chain, "Base", "chain carries the human label");
        assert_eq!(pair.chain_id, Some("8453".to_string()));
    }

    #[test_all]
    fn destination_to_pair_preserves_none_for_non_evm_transports() {
        let dest = test_destination(
            "Solana",
            Asset::Usdt,
            NetworkTransport::Solana,
            None,
            None,
            BridgeKind::Oft,
        );
        let pair = destination_to_route_pair(&dest);

        assert_eq!(
            pair.chain_id, None,
            "Non-EVM transports (Solana, Tron) expose no chain_id"
        );
    }

    #[test_all]
    fn fees_included_real_invoice_sats_subtracts_probe_fee() {
        let real = fees_included_real_invoice_sats(10_000, 250).expect("fits within budget");
        assert_eq!(
            real, 9_750,
            "real invoice should leave room for the probed LN fee"
        );
    }

    #[test_all]
    fn fees_included_real_invoice_sats_rejects_when_fee_eats_budget() {
        // Fee exactly equals total: no room for any invoice → reject.
        let err = fees_included_real_invoice_sats(500, 500)
            .expect_err("equal probe fee leaves zero invoice");
        match err {
            SdkError::InvalidInput(msg) => {
                assert!(msg.contains("500"), "error should report the figures");
                assert!(msg.contains("Amount too small"));
            }
            other => panic!("expected InvalidInput, got {other:?}"),
        }

        // Fee exceeds total → also reject.
        let err =
            fees_included_real_invoice_sats(100, 250).expect_err("probe fee greater than total");
        assert!(matches!(err, SdkError::InvalidInput(_)));
    }

    // ---- poll_to_terminal_or_fallback ----

    #[cfg(feature = "sqlite")]
    mod poll_to_terminal_tests {
        use super::*;

        fn create_temp_dir(name: &str) -> std::path::PathBuf {
            let mut path = std::env::temp_dir();
            path.push(format!(
                "breez-test-boltz-{}-{}",
                name,
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&path).unwrap();
            path
        }

        fn make_pending_payment(id: &str) -> crate::Payment {
            crate::Payment {
                id: id.to_string(),
                payment_type: crate::PaymentType::Send,
                status: PaymentStatus::Pending,
                amount: 1_000,
                fees: 10,
                timestamp: 1,
                method: crate::PaymentMethod::Lightning,
                details: None,
                conversion_details: None,
            }
        }

        fn fast_schedule() -> PollSchedule {
            PollSchedule {
                initial_delay: Duration::from_millis(10),
                max_delay: Duration::from_millis(20),
                timeout: Duration::from_millis(100),
            }
        }

        #[tokio::test]
        async fn poll_to_terminal_returns_terminal_when_status_settles() {
            let dir = create_temp_dir("poll_settles");
            let storage: Arc<dyn Storage> =
                Arc::new(crate::persist::sqlite::SqliteStorage::new(&dir).unwrap());

            let pending = make_pending_payment("pay_settles");
            storage.apply_payment_update(pending.clone()).await.unwrap();

            let mut completed = pending.clone();
            completed.status = PaymentStatus::Completed;

            // Settle the payment mid-poll.
            let storage_w = Arc::clone(&storage);
            let completed_w = completed.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(20)).await;
                storage_w.apply_payment_update(completed_w).await.unwrap();
            });

            let fallback = pending.clone();
            let result = poll_to_terminal_or_fallback(
                Arc::clone(&storage),
                "pay_settles".to_string(),
                fallback,
                fast_schedule(),
            )
            .await;

            assert_eq!(result.status, PaymentStatus::Completed);
        }

        #[tokio::test]
        async fn poll_to_terminal_returns_fallback_on_timeout() {
            let dir = create_temp_dir("poll_timeout");
            let storage: Arc<dyn Storage> =
                Arc::new(crate::persist::sqlite::SqliteStorage::new(&dir).unwrap());

            let pending = make_pending_payment("pay_timeout");
            storage.apply_payment_update(pending.clone()).await.unwrap();

            let mut fallback = pending.clone();
            // Sentinel field on the fallback to prove the returned value is the
            // fallback we passed in, not a fresh read from storage.
            fallback.timestamp = 99_999;

            let result = poll_to_terminal_or_fallback(
                Arc::clone(&storage),
                "pay_timeout".to_string(),
                fallback,
                fast_schedule(),
            )
            .await;

            assert_eq!(
                result.timestamp, 99_999,
                "timeout should surface the supplied fallback payment as-is"
            );
            assert_eq!(result.status, PaymentStatus::Pending);
        }

        #[tokio::test]
        async fn poll_to_terminal_returns_fallback_on_storage_error() {
            // `get_payment_by_id` returns an error when the row is missing.
            let dir = create_temp_dir("poll_missing");
            let storage: Arc<dyn Storage> =
                Arc::new(crate::persist::sqlite::SqliteStorage::new(&dir).unwrap());

            // No payment inserted — get_payment_by_id will error every probe,
            // poll_until propagates the last error, and we fall back.
            let mut fallback = make_pending_payment("pay_missing");
            fallback.timestamp = 42_424;

            let result = poll_to_terminal_or_fallback(
                Arc::clone(&storage),
                "pay_missing".to_string(),
                fallback,
                fast_schedule(),
            )
            .await;

            assert_eq!(
                result.timestamp, 42_424,
                "storage errors must fall through to the fallback"
            );
        }
    }

    // ---- validate_quote_expiry ----

    #[test_all]
    fn validate_quote_expiry_accepts_future_unix_secs() {
        use platform_utils::time::{SystemTime, UNIX_EPOCH};
        let future = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .saturating_add(600);
        assert!(validate_quote_expiry(&future.to_string()).is_ok());
    }

    #[test_all]
    fn validate_quote_expiry_rejects_past_unix_secs() {
        let err = validate_quote_expiry("1000000000").unwrap_err();
        assert!(matches!(err, SdkError::InvalidInput(ref m) if m.contains("expired")));
    }

    #[test_all]
    fn validate_quote_expiry_rejects_malformed() {
        let err = validate_quote_expiry("not-a-number").unwrap_err();
        assert!(matches!(err, SdkError::Generic(ref m) if m.contains("invalid expires_at")));
    }
}
