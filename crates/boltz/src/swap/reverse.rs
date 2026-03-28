use std::sync::Arc;

use alloy_primitives::U256;

use crate::api::BoltzApiClient;
use crate::api::types::{EncodeRequest, QuoteResponse, ReversePairInfo};
use crate::api::ws::{SwapStatusSubscriber, SwapStatusUpdate};
use crate::config::{
    ARBITRUM_ERC20SWAP_ADDRESS, ARBITRUM_ERC20SWAP_DEPLOY_BLOCK, ARBITRUM_ROUTER_ADDRESS,
    ARBITRUM_TBTC_ADDRESS, ARBITRUM_USDT_ADDRESS, BoltzConfig, SATS_TO_TBTC_FACTOR, ZERO_ADDRESS,
};
use crate::error::BoltzError;
use crate::evm::alchemy::{AlchemyGasClient, EvmCall};
use crate::evm::contracts::{
    self, ClaimSendAuthorization, Erc20Claim, SendData, encode_claim_erc20_execute,
    encode_claim_erc20_execute_oft, parse_address, quote_calldata_to_call,
};
use crate::evm::oft::OftDeployments;
use crate::evm::provider::EvmProvider;
use crate::evm::signing::EvmSigner;
use crate::keys::EvmKeyManager;
use crate::models::{
    BoltzSwap, BoltzSwapStatus, Chain, ClaimedRecovery, CompletedSwap, CreatedSwap, PreparedSwap,
    RecoveryResult, SwapLimits,
};
use crate::recover::{self, RecoverableSwap};
use crate::store::BoltzStore;

/// Maximum claim retries (quote may go stale between encode and submit).
const MAX_CLAIM_RETRIES: u32 = 3;

/// Orchestrates the LN -> USDT reverse swap flow.
pub struct ReverseSwapExecutor {
    api_client: BoltzApiClient,
    ws_subscriber: SwapStatusSubscriber,
    key_manager: EvmKeyManager,
    alchemy_client: AlchemyGasClient,
    evm_provider: EvmProvider,
    oft_deployments: OftDeployments,
    store: Arc<dyn BoltzStore>,
    config: BoltzConfig,
}

impl ReverseSwapExecutor {
    #[expect(clippy::too_many_arguments)]
    pub fn new(
        api_client: BoltzApiClient,
        ws_subscriber: SwapStatusSubscriber,
        key_manager: EvmKeyManager,
        alchemy_client: AlchemyGasClient,
        evm_provider: EvmProvider,
        oft_deployments: OftDeployments,
        store: Arc<dyn BoltzStore>,
        config: BoltzConfig,
    ) -> Self {
        Self {
            api_client,
            ws_subscriber,
            key_manager,
            alchemy_client,
            evm_provider,
            oft_deployments,
            store,
            config,
        }
    }

    /// Get swap limits from the Boltz pairs endpoint.
    pub async fn get_limits(&self) -> Result<SwapLimits, BoltzError> {
        let tbtc_pair = self.fetch_tbtc_pair().await?;
        Ok(SwapLimits {
            min_sats: tbtc_pair.limits.minimal,
            max_sats: tbtc_pair.limits.maximal,
        })
    }

    /// Prepare a reverse swap quote. No side effects.
    ///
    /// Walks the route in reverse:
    /// 1. Get DEX quote: how much tBTC needed for `usdt_amount` USDT?
    /// 2. For cross-chain: estimate LZ messaging fee in tBTC and add it
    /// 3. Convert tBTC EVM units to sats
    /// 4. Apply Boltz fee to get total sats needed
    pub async fn prepare(
        &self,
        destination: &str,
        chain: Chain,
        usdt_amount: u64,
    ) -> Result<PreparedSwap, BoltzError> {
        if self.config.slippage_bps < 10 {
            return Err(BoltzError::Generic(
                "slippage_bps must be at least 10 (0.1%)".to_string(),
            ));
        }

        let tbtc_pair = self.fetch_tbtc_pair().await?;
        let mut tbtc_evm_units = self.fetch_quote_out_tbtc(usdt_amount).await?;

        // For cross-chain destinations, add the LZ messaging fee cost in tBTC.
        // Matches web app's `invertPostOftQuote` which inflates required sats.
        if !chain.is_source_chain() {
            let lz_fee_tbtc = self.estimate_lz_fee_in_tbtc(&chain, usdt_amount).await?;
            tbtc_evm_units = tbtc_evm_units.saturating_add(lz_fee_tbtc);
        }

        let fee_calc = compute_invoice_amount(&tbtc_pair, tbtc_evm_units)?;

        // Validate against Boltz swap limits
        if fee_calc.invoice_sats < tbtc_pair.limits.minimal
            || fee_calc.invoice_sats > tbtc_pair.limits.maximal
        {
            return Err(BoltzError::AmountOutOfRange {
                amount: fee_calc.invoice_sats,
                min: tbtc_pair.limits.minimal,
                max: tbtc_pair.limits.maximal,
            });
        }

        let now = current_unix_timestamp();
        Ok(PreparedSwap {
            destination_address: destination.to_string(),
            destination_chain: chain,
            usdt_amount,
            invoice_amount_sats: fee_calc.invoice_sats,
            boltz_fee_sats: fee_calc.boltz_fee_sats,
            estimated_onchain_amount: fee_calc.onchain_sats,
            estimated_usdt_output: usdt_amount,
            slippage_bps: self.config.slippage_bps,
            pair_hash: tbtc_pair.hash.clone(),
            expires_at: now.saturating_add(60),
        })
    }

    /// Create the swap on Boltz. Returns the hold invoice to pay.
    pub async fn create(&self, prepared: &PreparedSwap) -> Result<CreatedSwap, BoltzError> {
        if current_unix_timestamp() >= prepared.expires_at {
            return Err(BoltzError::QuoteExpired);
        }
        let chain_id_u32 = to_chain_id_u32(self.config.chain_id)?;
        let key_index = self.store.increment_key_index(self.config.chain_id).await?;

        let gas_signer = self.key_manager.derive_gas_signer(chain_id_u32)?;
        let preimage_hash = self
            .key_manager
            .derive_preimage_hash(chain_id_u32, key_index)?;
        let preimage_key = self
            .key_manager
            .derive_preimage_key(chain_id_u32, key_index)?;

        let create_req = crate::api::types::CreateReverseSwapRequest {
            from: "BTC".to_string(),
            to: "TBTC".to_string(),
            preimage_hash: hex::encode(preimage_hash),
            claim_address: gas_signer.address_hex(),
            invoice_amount: prepared.invoice_amount_sats,
            pair_hash: prepared.pair_hash.clone(),
            referral_id: self.config.referral_id.clone(),
            claim_public_key: hex::encode(&preimage_key.public_key),
            description: None,
            invoice_expiry: None,
        };

        let resp = self.api_client.create_reverse_swap(&create_req).await?;

        let swap_id = generate_swap_id();
        let now = current_unix_timestamp();
        let swap = BoltzSwap {
            id: swap_id.clone(),
            boltz_id: resp.id.clone(),
            status: BoltzSwapStatus::Created,
            claim_key_index: key_index,
            chain_id: self.config.chain_id,
            claim_address: gas_signer.address_hex(),
            destination_address: prepared.destination_address.clone(),
            destination_chain: prepared.destination_chain.clone(),
            refund_address: resp.refund_address.ok_or_else(|| BoltzError::Api {
                reason: "Missing refund_address in swap response".to_string(),
                code: None,
            })?,
            erc20swap_address: ARBITRUM_ERC20SWAP_ADDRESS.to_string(),
            router_address: ARBITRUM_ROUTER_ADDRESS.to_string(),
            invoice: resp.invoice.clone(),
            invoice_amount_sats: prepared.invoice_amount_sats,
            onchain_amount: resp.onchain_amount,
            expected_usdt_amount: prepared.usdt_amount,
            timeout_block_height: resp.timeout_block_height,
            lockup_tx_id: None,
            claim_tx_hash: None,
            created_at: now,
            updated_at: now,
        };
        self.store.insert_swap(&swap).await?;

        Ok(CreatedSwap {
            swap_id,
            boltz_id: resp.id,
            invoice: resp.invoice,
            invoice_amount_sats: prepared.invoice_amount_sats,
            timeout_block_height: resp.timeout_block_height,
        })
    }

    /// After the invoice is paid, monitor and complete the swap.
    /// Blocks until USDT is delivered or swap fails.
    pub async fn complete(&self, swap_id: &str) -> Result<CompletedSwap, BoltzError> {
        let mut swap = self
            .store
            .get_swap(swap_id)
            .await?
            .ok_or_else(|| BoltzError::Store(format!("Swap not found: {swap_id}")))?;

        let mut rx = self.ws_subscriber.subscribe(&swap.boltz_id).await?;

        if swap.status == BoltzSwapStatus::Created || swap.status == BoltzSwapStatus::InvoicePaid {
            swap = self.wait_for_lockup(&mut swap, &mut rx).await?;
        }
        if swap.status == BoltzSwapStatus::TbtcLocked {
            swap = self.claim_and_swap(&mut swap).await?;
        }

        self.ws_subscriber.unsubscribe(&swap.boltz_id).await;

        if swap.status == BoltzSwapStatus::Completed {
            Ok(CompletedSwap {
                swap_id: swap.id,
                claim_tx_hash: swap.claim_tx_hash.unwrap_or_default(),
                usdt_delivered: swap.expected_usdt_amount,
                destination_address: swap.destination_address,
                destination_chain: swap.destination_chain,
            })
        } else {
            tracing::error!(
                swap_id = swap.id,
                boltz_id = swap.boltz_id,
                status = ?swap.status,
                "Swap completed in non-success state"
            );
            Err(BoltzError::SwapFailed {
                swap_id: swap.id,
                reason: format!("Swap ended in status: {:?}", swap.status),
            })
        }
    }

    /// Resume all active (non-final) swaps by re-driving them to completion.
    /// Errors are logged per-swap; one failing swap does not abort the others.
    pub async fn resume_active_swaps(&self) -> Result<Vec<String>, BoltzError> {
        let active = self.store.list_active_swaps().await?;
        let mut resumed = Vec::with_capacity(active.len());
        for swap in &active {
            tracing::info!(
                swap_id = swap.id,
                boltz_id = swap.boltz_id,
                status = ?swap.status,
                "Resuming active swap"
            );
            match self.complete(&swap.id).await {
                Ok(completed) => {
                    tracing::info!(
                        swap_id = completed.swap_id,
                        claim_tx = completed.claim_tx_hash,
                        "Resumed swap completed successfully"
                    );
                    resumed.push(completed.swap_id);
                }
                Err(e) => {
                    tracing::error!(
                        swap_id = swap.id,
                        boltz_id = swap.boltz_id,
                        error = %e,
                        "Failed to resume swap"
                    );
                }
            }
        }
        Ok(resumed)
    }

    /// Recover unclaimed swaps by scanning the blockchain.
    ///
    /// Matches the Boltz web app's EVM recovery flow: scan `ERC20Swap` contract
    /// Lockup events, match against derived preimage hashes, claim any still-locked swaps.
    pub async fn recover(&self, destination_address: &str) -> Result<RecoveryResult, BoltzError> {
        let chain_id_u32 = to_chain_id_u32(self.config.chain_id)?;

        let (recoverable, stats) = recover::scan_for_recoverable_swaps(
            &self.evm_provider,
            &self.key_manager,
            chain_id_u32,
            ARBITRUM_ERC20SWAP_ADDRESS,
            ARBITRUM_ERC20SWAP_DEPLOY_BLOCK,
        )
        .await?;

        // Sync key index past all discovered indices
        if let Some(highest) = stats.highest_key_index {
            self.store
                .set_key_index_if_higher(self.config.chain_id, highest.saturating_add(1))
                .await?;
        }

        // Claim each recoverable swap
        let mut claimed = Vec::new();
        for swap in &recoverable {
            match self.claim_recovered_swap(swap, destination_address).await {
                Ok(tx_hash) => {
                    claimed.push(ClaimedRecovery {
                        key_index: swap.key_index,
                        preimage_hash: swap.preimage_hash,
                        claim_tx_hash: tx_hash,
                    });
                }
                Err(e) => {
                    tracing::error!(
                        key_index = swap.key_index,
                        tx = swap.lockup_tx_hash,
                        error = %e,
                        "Failed to claim recovered swap"
                    );
                }
            }
        }

        Ok(RecoveryResult {
            claimed,
            already_settled: stats.already_settled,
            total_events_scanned: stats.total_events,
            highest_key_index: stats.highest_key_index,
        })
    }

    /// Claim a single recovered swap by constructing a synthetic `BoltzSwap`
    /// and reusing the existing claim pipeline.
    async fn claim_recovered_swap(
        &self,
        recoverable: &RecoverableSwap,
        destination_address: &str,
    ) -> Result<String, BoltzError> {
        // Convert tBTC EVM amount (18 decimals) back to sats (8 decimals)
        let onchain_sats: u64 = recoverable
            .amount
            .checked_div(U256::from(SATS_TO_TBTC_FACTOR))
            .unwrap_or(U256::ZERO)
            .try_into()
            .map_err(|_| BoltzError::Generic("tBTC amount too large for u64".into()))?;

        let timelock: u64 = recoverable
            .timelock
            .try_into()
            .map_err(|_| BoltzError::Generic("Timelock too large for u64".into()))?;

        let now = current_unix_timestamp();
        let swap_id = generate_swap_id();

        let mut swap = BoltzSwap {
            id: swap_id,
            boltz_id: String::new(), // No Boltz ID for recovered swaps
            status: BoltzSwapStatus::TbtcLocked,
            claim_key_index: recoverable.key_index,
            chain_id: self.config.chain_id,
            claim_address: format!("0x{}", hex::encode(recoverable.claim_address.as_slice())),
            destination_address: destination_address.to_string(),
            destination_chain: Chain::Arbitrum, // Recovery always claims to Arbitrum
            refund_address: format!("0x{}", hex::encode(recoverable.refund_address.as_slice())),
            erc20swap_address: ARBITRUM_ERC20SWAP_ADDRESS.to_string(),
            router_address: ARBITRUM_ROUTER_ADDRESS.to_string(),
            invoice: String::new(),
            invoice_amount_sats: 0,
            onchain_amount: onchain_sats,
            expected_usdt_amount: 0, // Unknown for recovered swaps
            timeout_block_height: timelock,
            lockup_tx_id: Some(recoverable.lockup_tx_hash.clone()),
            claim_tx_hash: None,
            created_at: now,
            updated_at: now,
        };

        // Insert into store so claim_and_swap can track it
        self.store.insert_swap(&swap).await?;

        let completed = self.claim_and_swap(&mut swap).await?;
        Ok(completed.claim_tx_hash.unwrap_or_default())
    }

    // ─── Internal ────────────────────────────────────────────────────────

    async fn fetch_tbtc_pair(&self) -> Result<ReversePairInfo, BoltzError> {
        let pairs = self.api_client.get_reverse_swap_pairs().await?;
        pairs
            .0
            .get("BTC")
            .and_then(|m| m.get("TBTC"))
            .cloned()
            .ok_or_else(|| BoltzError::Api {
                reason: "BTC/TBTC pair not found. Is referral header configured?".to_string(),
                code: None,
            })
    }

    async fn fetch_quote_out_tbtc(&self, usdt_amount: u64) -> Result<u128, BoltzError> {
        let quotes = self
            .api_client
            .get_quote_out(
                "ARB",
                ARBITRUM_TBTC_ADDRESS,
                ARBITRUM_USDT_ADDRESS,
                u128::from(usdt_amount),
            )
            .await?;
        // "out" direction: pick lowest amount (least input needed for desired output),
        // matching web app's sortDexQuotes("out") which sorts ascending.
        let quote = pick_best_quote(&quotes, QuoteDirection::Out)?;
        if quote == 0 {
            return Err(BoltzError::InvalidQuote(
                "DEX quote returned zero tBTC".to_string(),
            ));
        }
        Ok(quote)
    }

    async fn wait_for_lockup(
        &self,
        swap: &mut BoltzSwap,
        rx: &mut tokio::sync::mpsc::Receiver<SwapStatusUpdate>,
    ) -> Result<BoltzSwap, BoltzError> {
        loop {
            let update = rx.recv().await.ok_or_else(|| {
                BoltzError::WebSocket("WS channel closed while waiting for lockup".to_string())
            })?;

            tracing::info!(
                swap_id = swap.boltz_id,
                status = update.status,
                "Swap status update"
            );

            match update.status.as_str() {
                "transaction.confirmed" => {
                    swap.status = BoltzSwapStatus::TbtcLocked;
                    if let Some(tx) = &update.transaction {
                        swap.lockup_tx_id = Some(tx.id.clone());
                    }
                    swap.updated_at = current_unix_timestamp();
                    self.store.update_swap(swap).await?;
                    return Ok(swap.clone());
                }
                "transaction.mempool" => {
                    if let Some(tx) = &update.transaction {
                        swap.lockup_tx_id = Some(tx.id.clone());
                        swap.updated_at = current_unix_timestamp();
                        self.store.update_swap(swap).await?;
                    }
                    tracing::info!(swap_id = swap.boltz_id, "tBTC lockup in mempool");
                }
                "invoice.settled" => {
                    swap.status = BoltzSwapStatus::InvoicePaid;
                    swap.updated_at = current_unix_timestamp();
                    self.store.update_swap(swap).await?;
                }
                "invoice.expired" | "swap.expired" => {
                    swap.status = BoltzSwapStatus::Expired;
                    swap.updated_at = current_unix_timestamp();
                    self.store.update_swap(swap).await?;
                    return Err(BoltzError::SwapExpired {
                        swap_id: swap.id.clone(),
                    });
                }
                "invoice.failedToPay" | "transaction.lockupFailed" | "transaction.refunded" => {
                    let reason = update
                        .failure_reason
                        .unwrap_or_else(|| update.status.clone());
                    swap.status = BoltzSwapStatus::Failed {
                        reason: reason.clone(),
                    };
                    swap.updated_at = current_unix_timestamp();
                    self.store.update_swap(swap).await?;
                    return Err(BoltzError::SwapFailed {
                        swap_id: swap.id.clone(),
                        reason,
                    });
                }
                _ => {
                    tracing::debug!(
                        swap_id = swap.boltz_id,
                        status = update.status,
                        "Unknown swap status, ignoring"
                    );
                }
            }
        }
    }

    async fn claim_and_swap(&self, swap: &mut BoltzSwap) -> Result<BoltzSwap, BoltzError> {
        let chain_id_u32 = to_chain_id_u32(swap.chain_id)?;
        let preimage = self
            .key_manager
            .derive_preimage(chain_id_u32, swap.claim_key_index)?;
        let gas_key_pair = self.key_manager.derive_gas_signer(chain_id_u32)?;
        let gas_signer = EvmSigner::new(&gas_key_pair, swap.chain_id);
        let erc20swap_version = self
            .fetch_erc20swap_version(&swap.erc20swap_address)
            .await?;

        let addrs = ClaimAddresses::parse(swap)?;
        let tbtc_evm_amount =
            U256::from(swap.onchain_amount).saturating_mul(U256::from(SATS_TO_TBTC_FACTOR));
        let timelock = U256::from(swap.timeout_block_height);

        for attempt in 0..MAX_CLAIM_RETRIES {
            if attempt > 0 {
                tracing::info!(attempt, swap_id = swap.boltz_id, "Retrying claim");
            }

            let result = self
                .try_claim(
                    swap,
                    &gas_signer,
                    &erc20swap_version,
                    &preimage,
                    &addrs,
                    tbtc_evm_amount,
                    timelock,
                )
                .await;

            match result {
                Ok(tx_hash) => {
                    swap.claim_tx_hash = Some(tx_hash);
                    swap.status = BoltzSwapStatus::Completed;
                    swap.updated_at = current_unix_timestamp();
                    self.store.update_swap(swap).await?;
                    return Ok(swap.clone());
                }
                Err(e) if attempt < MAX_CLAIM_RETRIES.saturating_sub(1) => {
                    tracing::warn!(attempt, swap_id = swap.boltz_id, error = %e, "Claim failed");
                    sleep_1s().await;
                }
                Err(e) => {
                    swap.status = BoltzSwapStatus::Failed {
                        reason: format!("Claim failed after {MAX_CLAIM_RETRIES} attempts: {e}"),
                    };
                    swap.updated_at = current_unix_timestamp();
                    self.store.update_swap(swap).await?;
                    return Err(e);
                }
            }
        }
        unreachable!("loop exits via return")
    }

    #[expect(clippy::too_many_arguments)]
    async fn try_claim(
        &self,
        swap: &mut BoltzSwap,
        gas_signer: &EvmSigner,
        erc20swap_version: &str,
        preimage: &[u8; 32],
        addrs: &ClaimAddresses,
        tbtc_evm_amount: U256,
        timelock: U256,
    ) -> Result<String, BoltzError> {
        if swap.destination_chain == Chain::Arbitrum {
            self.try_claim_same_chain(
                swap,
                gas_signer,
                erc20swap_version,
                preimage,
                addrs,
                tbtc_evm_amount,
                timelock,
            )
            .await
        } else {
            self.try_claim_cross_chain(
                swap,
                gas_signer,
                erc20swap_version,
                preimage,
                addrs,
                tbtc_evm_amount,
                timelock,
            )
            .await
        }
    }

    /// Same-chain claim: claim tBTC + DEX swap to USDT + sweep to destination on Arbitrum.
    #[expect(clippy::too_many_arguments)]
    async fn try_claim_same_chain(
        &self,
        swap: &mut BoltzSwap,
        gas_signer: &EvmSigner,
        erc20swap_version: &str,
        preimage: &[u8; 32],
        addrs: &ClaimAddresses,
        tbtc_evm_amount: U256,
        timelock: U256,
    ) -> Result<String, BoltzError> {
        let (dex_calls, min_amount_out) = self
            .fetch_and_encode_dex_quote(tbtc_evm_amount, &addrs.router.to_string())
            .await?;

        let erc20swap_sig = gas_signer.sign_eip712_erc20swap_claim(
            addrs.erc20swap,
            erc20swap_version,
            swap.chain_id,
            preimage,
            tbtc_evm_amount,
            addrs.tbtc,
            addrs.refund,
            timelock,
            addrs.router,
        )?;

        let router_sig = gas_signer.sign_eip712_router_claim(
            addrs.router,
            swap.chain_id,
            preimage,
            addrs.usdt,
            min_amount_out,
            addrs.destination,
        )?;

        let erc20_claim = Erc20Claim {
            preimage: (*preimage).into(),
            amount: tbtc_evm_amount,
            tokenAddress: addrs.tbtc,
            refundAddress: addrs.refund,
            timelock,
            v: erc20swap_sig.v,
            r: erc20swap_sig.r.into(),
            s: erc20swap_sig.s.into(),
        };

        let calldata = encode_claim_erc20_execute(
            &erc20_claim,
            &dex_calls,
            addrs.usdt,
            min_amount_out,
            addrs.destination,
            router_sig.v,
            router_sig.r,
            router_sig.s,
        );

        self.submit_claim(swap, &swap.router_address.clone(), &calldata)
            .await
    }

    /// Cross-chain claim: claim tBTC + DEX swap to USDT + OFT bridge to destination chain.
    ///
    /// Two-pass approach matching the Boltz web app (`TransactionConfirmed.tsx`):
    /// - Pass 1: estimate `LayerZero` messaging fee cost in tBTC
    /// - Pass 2: re-quote with adjusted tBTC split (trade vs fee)
    #[expect(clippy::too_many_arguments, clippy::too_many_lines)]
    async fn try_claim_cross_chain(
        &self,
        swap: &mut BoltzSwap,
        gas_signer: &EvmSigner,
        erc20swap_version: &str,
        preimage: &[u8; 32],
        addrs: &ClaimAddresses,
        tbtc_evm_amount: U256,
        timelock: U256,
    ) -> Result<String, BoltzError> {
        let dst_chain_id = swap.destination_chain.evm_chain_id();
        let dst_info = self.oft_deployments.get(dst_chain_id).ok_or_else(|| {
            BoltzError::Generic(format!(
                "No OFT deployment for destination chain ID {dst_chain_id}"
            ))
        })?;
        let dst_eid = dst_info.lz_eid;

        let source_oft_address = self
            .oft_deployments
            .source_oft_address(self.config.chain_id)
            .ok_or_else(|| {
                BoltzError::Generic("No OFT deployment for source chain (Arbitrum)".into())
            })?;
        let oft_addr = parse_address(source_oft_address)?;

        let tbtc_amount: u128 = tbtc_evm_amount
            .try_into()
            .map_err(|_| BoltzError::Generic("tBTC amount too large".into()))?;
        let router_str = addrs.router.to_string();

        // ─── Pass 1: estimate LZ fee cost ─────────────────────────────
        // Get initial DEX quote with full tBTC to estimate USDT output
        let initial_trade = pick_best_quote_with_data(
            &self
                .api_client
                .get_quote_in(
                    "ARB",
                    ARBITRUM_TBTC_ADDRESS,
                    ARBITRUM_USDT_ADDRESS,
                    tbtc_amount,
                )
                .await?,
            QuoteDirection::In,
        )?;

        // Quote OFT with initial USDT amount to get messaging fee
        let initial_send_param = contracts::build_oft_send_param(
            dst_eid,
            addrs.destination,
            U256::from(initial_trade.amount),
            U256::ZERO,
        );
        let (_, initial_receipt) = self
            .quote_oft(source_oft_address, &initial_send_param)
            .await?;

        let mut quoted_send_param = initial_send_param.clone();
        quoted_send_param.minAmountLD = initial_receipt.amountReceivedLD;
        let msg_fee = self
            .quote_send(source_oft_address, &quoted_send_param)
            .await?;

        // Apply slippage buffer to messaging fee
        let native_fee: u128 = msg_fee
            .nativeFee
            .try_into()
            .map_err(|_| BoltzError::Generic("LZ fee too large".into()))?;
        let fee_with_slippage = apply_slippage_up(native_fee, u128::from(self.config.slippage_bps));

        // Quote DEX: how much tBTC for the ETH messaging fee
        let fee_dex = pick_best_quote_with_data(
            &self
                .api_client
                .get_quote_out(
                    "ARB",
                    ARBITRUM_TBTC_ADDRESS,
                    ZERO_ADDRESS,
                    fee_with_slippage,
                )
                .await?,
            QuoteDirection::Out,
        )?;

        // ─── Pass 2: final quotes with adjusted tBTC split ────────────
        let fee_tbtc = fee_dex.amount;
        let trade_tbtc = tbtc_amount.checked_sub(fee_tbtc).ok_or_else(|| {
            BoltzError::Generic("Amount too small to cover OFT cross-chain messaging fee".into())
        })?;
        if trade_tbtc == 0 {
            return Err(BoltzError::Generic(
                "Amount too small to cover OFT cross-chain messaging fee".into(),
            ));
        }

        // Final trade DEX quote: trade_tbtc -> USDT
        let trade_best = pick_best_quote_with_data(
            &self
                .api_client
                .get_quote_in(
                    "ARB",
                    ARBITRUM_TBTC_ADDRESS,
                    ARBITRUM_USDT_ADDRESS,
                    trade_tbtc,
                )
                .await?,
            QuoteDirection::In,
        )?;
        if trade_best.amount == 0 {
            return Err(BoltzError::InvalidQuote("DEX returned zero USDT".into()));
        }
        #[expect(clippy::arithmetic_side_effects)]
        let slippage_factor = 10000 - u128::from(self.config.slippage_bps);
        #[expect(clippy::arithmetic_side_effects)]
        let min_usdt_out = trade_best.amount * slippage_factor / 10000;

        // Final OFT quote with the actual USDT amount
        let final_send_param = contracts::build_oft_send_param(
            dst_eid,
            addrs.destination,
            U256::from(min_usdt_out),
            U256::ZERO,
        );
        let (_, final_receipt) = self
            .quote_oft(source_oft_address, &final_send_param)
            .await?;

        let mut final_quoted_param = final_send_param.clone();
        final_quoted_param.minAmountLD = final_receipt.amountReceivedLD;
        let final_msg_fee = self
            .quote_send(source_oft_address, &final_quoted_param)
            .await?;

        // Apply slippage to the OFT min receive amount
        let min_amount_ld_raw: u128 = final_receipt
            .amountReceivedLD
            .try_into()
            .map_err(|_| BoltzError::Generic("OFT amount too large".into()))?;
        #[expect(clippy::arithmetic_side_effects)]
        let min_amount_ld_slipped = min_amount_ld_raw * slippage_factor / 10000;

        // ─── Encode DEX calls ─────────────────────────────────────────
        // Trade calls: tBTC -> USDT
        let trade_encode = self
            .api_client
            .encode_quote(
                "ARB",
                &EncodeRequest {
                    recipient: router_str.clone(),
                    amount_in: trade_tbtc,
                    amount_out_min: min_usdt_out,
                    data: trade_best.data,
                },
            )
            .await?;
        let trade_calls: Vec<contracts::Call> = trade_encode
            .calls
            .iter()
            .map(quote_calldata_to_call)
            .collect::<Result<Vec<_>, _>>()?;

        // Fee calls: tBTC -> ETH (for LZ messaging)
        let fee_encode = self
            .api_client
            .encode_quote(
                "ARB",
                &EncodeRequest {
                    recipient: router_str.clone(),
                    amount_in: fee_tbtc,
                    amount_out_min: native_fee,
                    data: fee_dex.data,
                },
            )
            .await?;
        let fee_calls: Vec<contracts::Call> = fee_encode
            .calls
            .iter()
            .map(quote_calldata_to_call)
            .collect::<Result<Vec<_>, _>>()?;

        // Combine all DEX calls
        let mut all_calls = trade_calls;
        all_calls.extend(fee_calls);

        // ─── Build SendData + hash ────────────────────────────────────
        let send_data = SendData {
            dstEid: dst_eid,
            to: contracts::address_to_bytes32(addrs.destination),
            extraOptions: vec![].into(),
            composeMsg: vec![].into(),
            oftCmd: vec![].into(),
        };

        let typehash = self.fetch_typehash_send_data(&swap.router_address).await?;
        let send_data_hash = contracts::hash_send_data(typehash, &send_data);

        // ─── Sign ─────────────────────────────────────────────────────
        let erc20swap_sig = gas_signer.sign_eip712_erc20swap_claim(
            addrs.erc20swap,
            erc20swap_version,
            swap.chain_id,
            preimage,
            tbtc_evm_amount,
            addrs.tbtc,
            addrs.refund,
            timelock,
            addrs.router,
        )?;

        let router_sig = gas_signer.sign_eip712_router_claim_send(
            addrs.router,
            swap.chain_id,
            preimage,
            addrs.usdt,
            oft_addr,
            send_data_hash,
            U256::from(min_amount_ld_slipped),
            final_msg_fee.lzTokenFee,
            addrs.refund,
        )?;

        // ─── Encode calldata ──────────────────────────────────────────
        let erc20_claim = Erc20Claim {
            preimage: (*preimage).into(),
            amount: tbtc_evm_amount,
            tokenAddress: addrs.tbtc,
            refundAddress: addrs.refund,
            timelock,
            v: erc20swap_sig.v,
            r: erc20swap_sig.r.into(),
            s: erc20swap_sig.s.into(),
        };

        let auth = ClaimSendAuthorization {
            minAmountLd: U256::from(min_amount_ld_slipped),
            lzTokenFee: final_msg_fee.lzTokenFee,
            refundAddress: addrs.refund,
            v: router_sig.v,
            r: router_sig.r.into(),
            s: router_sig.s.into(),
        };

        let calldata = encode_claim_erc20_execute_oft(
            &erc20_claim,
            &all_calls,
            addrs.usdt,
            oft_addr,
            &send_data,
            &auth,
        );

        self.submit_claim(swap, &swap.router_address.clone(), &calldata)
            .await
    }

    /// Submit encoded calldata via Alchemy gas abstraction.
    async fn submit_claim(
        &self,
        swap: &mut BoltzSwap,
        router_address: &str,
        calldata: &[u8],
    ) -> Result<String, BoltzError> {
        swap.status = BoltzSwapStatus::Claiming;
        swap.updated_at = current_unix_timestamp();
        self.store.update_swap(swap).await?;

        let evm_call = EvmCall {
            to: router_address.to_string(),
            value: None,
            data: Some(format!("0x{}", hex::encode(calldata))),
        };

        let result = self
            .alchemy_client
            .send_sponsored_calls(vec![evm_call], swap.chain_id)
            .await?;

        tracing::info!(
            tx_hash = result.tx_hash,
            swap_id = swap.boltz_id,
            "Claim confirmed"
        );
        Ok(result.tx_hash)
    }

    // ─── OFT fee estimation (for prepare-time quoting) ─────────────────

    /// Estimate the LZ messaging fee cost in tBTC EVM units.
    /// Used at prepare time to inflate the invoice to cover cross-chain fees.
    /// Matches the web app's `invertPostOftQuote` → `quoteMessagingFeeCost` flow.
    async fn estimate_lz_fee_in_tbtc(
        &self,
        chain: &Chain,
        usdt_amount: u64,
    ) -> Result<u128, BoltzError> {
        let dst_chain_id = chain.evm_chain_id();
        let dst_info = self.oft_deployments.get(dst_chain_id).ok_or_else(|| {
            BoltzError::Generic(format!(
                "No OFT deployment for destination chain ID {dst_chain_id}"
            ))
        })?;
        let source_oft_address = self
            .oft_deployments
            .source_oft_address(self.config.chain_id)
            .ok_or_else(|| {
                BoltzError::Generic("No OFT deployment for source chain (Arbitrum)".into())
            })?;

        // Quote OFT to get the messaging fee
        let send_param = contracts::build_oft_send_param(
            dst_info.lz_eid,
            alloy_primitives::Address::ZERO, // placeholder recipient for quoting
            U256::from(usdt_amount),
            U256::ZERO,
        );
        let (_, receipt) = self.quote_oft(source_oft_address, &send_param).await?;

        let mut quoted_param = send_param;
        quoted_param.minAmountLD = receipt.amountReceivedLD;
        let msg_fee = self.quote_send(source_oft_address, &quoted_param).await?;

        let native_fee: u128 = msg_fee
            .nativeFee
            .try_into()
            .map_err(|_| BoltzError::Generic("LZ fee too large".into()))?;

        if native_fee == 0 {
            return Ok(0);
        }

        // Apply slippage buffer upward to the fee
        let fee_with_slippage = apply_slippage_up(native_fee, u128::from(self.config.slippage_bps));

        // DEX quote: how much tBTC to get the required ETH for the LZ fee
        let fee_dex = pick_best_quote(
            &self
                .api_client
                .get_quote_out(
                    "ARB",
                    ARBITRUM_TBTC_ADDRESS,
                    ZERO_ADDRESS,
                    fee_with_slippage,
                )
                .await?,
            QuoteDirection::Out,
        )?;

        Ok(fee_dex)
    }

    // ─── OFT quoting helpers ──────────────────────────────────────────

    /// Call `quoteOFT` on the OFT contract via `eth_call`.
    async fn quote_oft(
        &self,
        oft_address: &str,
        send_param: &contracts::OftSendParam,
    ) -> Result<(contracts::OftLimit, contracts::OftReceipt), BoltzError> {
        let calldata = contracts::encode_quote_oft(send_param);
        let result = self.evm_provider.eth_call(oft_address, &calldata).await?;
        let (limit, _fees, receipt) = contracts::decode_quote_oft_return(&result)?;
        Ok((limit, receipt))
    }

    /// Call `quoteSend` on the OFT contract via `eth_call`.
    async fn quote_send(
        &self,
        oft_address: &str,
        send_param: &contracts::OftSendParam,
    ) -> Result<contracts::MessagingFee, BoltzError> {
        let calldata = contracts::encode_quote_send(send_param, false);
        let result = self.evm_provider.eth_call(oft_address, &calldata).await?;
        contracts::decode_quote_send_return(&result)
    }

    /// Fetch `TYPEHASH_SEND_DATA` from the Router contract.
    async fn fetch_typehash_send_data(&self, router_address: &str) -> Result<[u8; 32], BoltzError> {
        let calldata = contracts::encode_typehash_send_data_call();
        let result = self
            .evm_provider
            .eth_call(router_address, &calldata)
            .await?;
        contracts::decode_typehash_send_data(&result)
    }

    async fn fetch_erc20swap_version(&self, erc20swap_address: &str) -> Result<String, BoltzError> {
        let calldata = contracts::encode_version_call();
        let result = self
            .evm_provider
            .eth_call(erc20swap_address, &calldata)
            .await?;
        let version = contracts::decode_version_return(&result)?;
        Ok(version.to_string())
    }

    #[expect(clippy::arithmetic_side_effects)]
    async fn fetch_and_encode_dex_quote(
        &self,
        tbtc_evm_amount: U256,
        router_address: &str,
    ) -> Result<(Vec<contracts::Call>, U256), BoltzError> {
        let amount_in: u128 = tbtc_evm_amount
            .try_into()
            .map_err(|_| BoltzError::Generic("tBTC amount too large".to_string()))?;

        let quotes = self
            .api_client
            .get_quote_in(
                "ARB",
                ARBITRUM_TBTC_ADDRESS,
                ARBITRUM_USDT_ADDRESS,
                amount_in,
            )
            .await?;
        // "in" direction: pick highest output (best return for our input),
        // matching web app's sortDexQuotes("in") which sorts descending.
        let best = pick_best_quote_with_data(&quotes, QuoteDirection::In)?;
        if best.amount == 0 {
            return Err(BoltzError::InvalidQuote(
                "DEX quote returned zero USDT".to_string(),
            ));
        }
        let slippage_factor = 10000 - u128::from(self.config.slippage_bps);
        let min_amount_out_u128 = best.amount * slippage_factor / 10000;
        let min_amount_out = U256::from(min_amount_out_u128);

        let encode_req = EncodeRequest {
            recipient: router_address.to_string(),
            amount_in,
            amount_out_min: min_amount_out_u128,
            data: best.data.clone(),
        };
        let encode_resp = self.api_client.encode_quote("ARB", &encode_req).await?;

        let calls = encode_resp
            .calls
            .iter()
            .map(quote_calldata_to_call)
            .collect::<Result<Vec<_>, _>>()?;

        Ok((calls, min_amount_out))
    }
}

// ─── Parsed addresses for claim ──────────────────────────────────────────

struct ClaimAddresses {
    erc20swap: alloy_primitives::Address,
    router: alloy_primitives::Address,
    tbtc: alloy_primitives::Address,
    usdt: alloy_primitives::Address,
    refund: alloy_primitives::Address,
    destination: alloy_primitives::Address,
}

impl ClaimAddresses {
    fn parse(swap: &BoltzSwap) -> Result<Self, BoltzError> {
        Ok(Self {
            erc20swap: parse_address(&swap.erc20swap_address)?,
            router: parse_address(&swap.router_address)?,
            tbtc: parse_address(ARBITRUM_TBTC_ADDRESS)?,
            usdt: parse_address(ARBITRUM_USDT_ADDRESS)?,
            refund: parse_address(&swap.refund_address)?,
            destination: parse_address(&swap.destination_address)?,
        })
    }
}

// ─── Fee computation ─────────────────────────────────────────────────────

struct FeeCalc {
    invoice_sats: u64,
    boltz_fee_sats: u64,
    onchain_sats: u64,
}

/// Compute the total sats needed from tBTC EVM units and Boltz pair info.
#[expect(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn compute_invoice_amount(
    pair: &ReversePairInfo,
    tbtc_evm_units: u128,
) -> Result<FeeCalc, BoltzError> {
    let sats_factor = u128::from(SATS_TO_TBTC_FACTOR);
    let tbtc_sats = tbtc_evm_units / sats_factor;

    let miner_fees =
        u128::from(pair.fees.miner_fees.claim) + u128::from(pair.fees.miner_fees.lockup);
    let base = tbtc_sats + miner_fees;

    // percentage is e.g. 0.25 for 0.25%. Convert to basis points (25).
    let pct_bps = (pair.fees.percentage * 100.0).round() as u128;
    let numerator = base * 10000;
    let denominator = 10000u128.saturating_sub(pct_bps);
    if denominator == 0 {
        return Err(BoltzError::Generic("Invalid fee percentage".to_string()));
    }
    let invoice = numerator.div_ceil(denominator);
    let boltz_fee = invoice - base;
    let onchain = invoice - boltz_fee - miner_fees;

    let to_u64 = |v: u128, name: &str| -> Result<u64, BoltzError> {
        v.try_into()
            .map_err(|_| BoltzError::Generic(format!("{name} overflow")))
    };

    Ok(FeeCalc {
        invoice_sats: to_u64(invoice, "Invoice amount")?,
        boltz_fee_sats: to_u64(boltz_fee, "Boltz fee")?,
        onchain_sats: to_u64(onchain, "Onchain amount")?,
    })
}

// ─── Helpers ─────────────────────────────────────────────────────────────

fn to_chain_id_u32(chain_id: u64) -> Result<u32, BoltzError> {
    chain_id
        .try_into()
        .map_err(|_| BoltzError::Generic("Chain ID overflow".to_string()))
}

fn generate_swap_id() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::time::{SystemTime, UNIX_EPOCH};

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    // Mix in stack address for cheap per-call entropy
    let mut hasher = DefaultHasher::new();
    let stack_addr = &raw const nanos as usize;
    (nanos, stack_addr).hash(&mut hasher);
    let suffix = hasher.finish();
    format!("boltz-{nanos:x}-{suffix:08x}")
}

fn current_unix_timestamp() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

async fn sleep_1s() {
    platform_utils::tokio::time::sleep(platform_utils::time::Duration::from_secs(1)).await;
}

/// Apply slippage upward (for fees that might increase).
/// Returns `amount * (10000 + slippage_bps) / 10000`.
#[expect(clippy::arithmetic_side_effects)]
fn apply_slippage_up(amount: u128, slippage_bps: u128) -> u128 {
    amount * (10000 + slippage_bps) / 10000
}

// ─── DEX quote selection ─────────────────────────────────────────────────
// Matches the web app's `sortDexQuotes` logic:
// - "in" direction (quoting by input):  pick highest output (best return)
// - "out" direction (quoting by output): pick lowest input  (cheapest route)

#[derive(Clone, Copy)]
enum QuoteDirection {
    In,
    Out,
}

struct ParsedQuote {
    amount: u128,
    data: serde_json::Value,
}

fn pick_best_quote(
    quotes: &[QuoteResponse],
    direction: QuoteDirection,
) -> Result<u128, BoltzError> {
    Ok(pick_best_quote_with_data(quotes, direction)?.amount)
}

fn pick_best_quote_with_data(
    quotes: &[QuoteResponse],
    direction: QuoteDirection,
) -> Result<ParsedQuote, BoltzError> {
    if quotes.is_empty() {
        return Err(BoltzError::Api {
            reason: "No DEX quote returned".to_string(),
            code: None,
        });
    }

    let mut best: Option<ParsedQuote> = None;
    for q in quotes {
        let amount: u128 = q.quote.parse().map_err(|_| BoltzError::Api {
            reason: format!("Invalid quote amount: {}", q.quote),
            code: None,
        })?;
        let is_better = match best {
            None => true,
            Some(ref b) => match direction {
                QuoteDirection::In => amount > b.amount,
                QuoteDirection::Out => amount < b.amount,
            },
        };
        if is_better {
            best = Some(ParsedQuote {
                amount,
                data: q.data.clone(),
            });
        }
    }

    best.ok_or_else(|| BoltzError::Api {
        reason: "No DEX quote returned".to_string(),
        code: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_swap_id() {
        let id1 = generate_swap_id();
        let id2 = generate_swap_id();
        assert!(id1.starts_with("boltz-"));
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_current_unix_timestamp() {
        let ts = current_unix_timestamp();
        assert!(ts > 1_704_067_200);
    }

    #[test]
    fn test_compute_invoice_amount() {
        let pair = ReversePairInfo {
            hash: "abc".to_string(),
            rate: 1.0,
            limits: crate::api::types::PairLimits {
                minimal: 10000,
                maximal: 25_000_000,
            },
            fees: crate::api::types::ReversePairFees {
                percentage: 0.25,
                miner_fees: crate::api::types::MinerFees {
                    claim: 170,
                    lockup: 171,
                },
            },
        };

        // 0.001 BTC = 100_000 sats = 100_000 * 10^10 EVM units
        let tbtc_evm_units: u128 = 100_000 * 10_000_000_000;
        let result = compute_invoice_amount(&pair, tbtc_evm_units).unwrap();

        // invoice should be > base (100_000 + 170 + 171 = 100_341)
        assert!(result.invoice_sats > 100_341);
        assert!(result.boltz_fee_sats > 0);
        assert!(result.onchain_sats > 0);
    }

    fn make_quote(amount: &str) -> QuoteResponse {
        QuoteResponse {
            quote: amount.to_string(),
            data: serde_json::json!({"type": "test"}),
        }
    }

    #[test]
    fn test_pick_best_quote_in_direction() {
        // "in" direction: highest output wins
        let quotes = vec![make_quote("100"), make_quote("300"), make_quote("200")];
        let best = pick_best_quote(&quotes, QuoteDirection::In).unwrap();
        assert_eq!(best, 300);
    }

    #[test]
    fn test_pick_best_quote_out_direction() {
        // "out" direction: lowest input wins
        let quotes = vec![make_quote("300"), make_quote("100"), make_quote("200")];
        let best = pick_best_quote(&quotes, QuoteDirection::Out).unwrap();
        assert_eq!(best, 100);
    }

    #[test]
    fn test_pick_best_quote_single() {
        let quotes = vec![make_quote("42")];
        assert_eq!(pick_best_quote(&quotes, QuoteDirection::In).unwrap(), 42);
        assert_eq!(pick_best_quote(&quotes, QuoteDirection::Out).unwrap(), 42);
    }

    #[test]
    fn test_pick_best_quote_empty() {
        let quotes: Vec<QuoteResponse> = vec![];
        assert!(pick_best_quote(&quotes, QuoteDirection::In).is_err());
    }

    #[test]
    fn test_pick_best_quote_preserves_data() {
        let quotes = vec![
            QuoteResponse {
                quote: "100".to_string(),
                data: serde_json::json!({"route": "A"}),
            },
            QuoteResponse {
                quote: "200".to_string(),
                data: serde_json::json!({"route": "B"}),
            },
        ];
        let best = pick_best_quote_with_data(&quotes, QuoteDirection::In).unwrap();
        assert_eq!(best.amount, 200);
        assert_eq!(best.data, serde_json::json!({"route": "B"}));
    }
}
