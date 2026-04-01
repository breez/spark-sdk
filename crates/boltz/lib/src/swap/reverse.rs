use std::sync::Arc;

use alloy_primitives::U256;

use crate::api::BoltzApiClient;
use crate::api::types::{EncodeRequest, QuoteResponse, ReversePairInfo};
use crate::config::{
    ARBITRUM_ERC20SWAP_DEPLOY_BLOCK, ARBITRUM_ROUTER_ADDRESS, ARBITRUM_TBTC_ADDRESS,
    ARBITRUM_USDT_ADDRESS, BoltzConfig, MAX_SLIPPAGE_BPS, SATS_TO_TBTC_FACTOR, ZERO_ADDRESS,
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
    BoltzSwap, BoltzSwapStatus, Chain, ClaimedRecovery, CreatedSwap, PreparedSwap, RecoveryResult,
    SwapLimits,
};
use crate::recover::{self, RecoverableSwap};
use crate::store::BoltzStorage;

/// Maximum claim retries (quote may go stale between encode and submit).
const MAX_CLAIM_RETRIES: u32 = 3;

/// Orchestrates the LN -> USDT reverse swap flow.
pub(crate) struct ReverseSwapExecutor {
    api_client: BoltzApiClient,
    pub(crate) key_manager: EvmKeyManager,
    alchemy_client: AlchemyGasClient,
    pub(crate) evm_provider: EvmProvider,
    oft_deployments: OftDeployments,
    pub(crate) store: Arc<dyn BoltzStorage>,
    pub(crate) config: BoltzConfig,
    pub(crate) erc20swap_address: String,
}

impl ReverseSwapExecutor {
    #[expect(clippy::too_many_arguments)]
    pub fn new(
        api_client: BoltzApiClient,
        key_manager: EvmKeyManager,
        alchemy_client: AlchemyGasClient,
        evm_provider: EvmProvider,
        oft_deployments: OftDeployments,
        store: Arc<dyn BoltzStorage>,
        config: BoltzConfig,
        erc20swap_address: String,
    ) -> Self {
        Self {
            api_client,
            key_manager,
            alchemy_client,
            evm_provider,
            oft_deployments,
            store,
            config,
            erc20swap_address,
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
    /// Matches the web app's `calculateSendAmount` flow, working backwards:
    /// 1. For cross-chain: invert OFT (find USDT on Arbitrum needed to deliver target on destination)
    /// 2. Add OFT messaging fee cost (in USDT)
    /// 3. DEX quote: how much tBTC for the total USDT needed
    /// 4. Apply Boltz fee to get total sats needed
    pub async fn prepare(
        &self,
        destination: &str,
        chain: Chain,
        usdt_amount: u64,
    ) -> Result<PreparedSwap, BoltzError> {
        if self.config.slippage_bps < 10 || self.config.slippage_bps > MAX_SLIPPAGE_BPS {
            return Err(BoltzError::Generic(format!(
                "slippage_bps must be >= 10 and <= {MAX_SLIPPAGE_BPS}"
            )));
        }

        // Validate destination is a well-formed EVM address before committing to a swap
        parse_address(destination)?;

        let tbtc_pair = self.fetch_tbtc_pair().await?;

        // Step 1+2: Determine total USDT needed on Arbitrum
        let total_usdt_on_arb = if chain.is_source_chain() {
            u128::from(usdt_amount)
        } else {
            // Matching web app's invertPostOftQuote:
            // Find how much USDT on Arbitrum is needed to deliver usdt_amount on destination
            let required_usdt = self
                .estimate_oft_required_send_amount(&chain, u128::from(usdt_amount))
                .await?;
            // Get messaging fee and convert to USDT cost
            let (msg_fee_native, _) = self.quote_oft_messaging_fee(&chain, required_usdt).await?;
            let msg_fee_usdt = if msg_fee_native == 0 {
                0u128
            } else {
                self.fetch_quote_out_usdt_for_eth(msg_fee_native).await?
            };
            // Total = OFT required amount + messaging fee cost (both in USDT)
            required_usdt
                .checked_add(msg_fee_usdt)
                .ok_or_else(|| BoltzError::Generic("USDT amount overflow".into()))?
        };

        // Step 3: DEX quote for the total USDT needed → tBTC
        let total_usdt_u64 = u64::try_from(total_usdt_on_arb)
            .map_err(|_| BoltzError::Generic("USDT amount overflow".into()))?;
        let tbtc_evm_units = self.fetch_quote_out_tbtc(total_usdt_u64).await?;

        // Step 4: Apply Boltz fee
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
            slippage_bps: self.config.slippage_bps,
            pair_hash: tbtc_pair.hash.clone(),
            expires_at: now.saturating_add(60),
        })
    }

    /// Prepare a reverse swap quote starting from input sats.
    ///
    /// Walks the route forward:
    /// 1. Apply Boltz fee to get onchain tBTC sats
    /// 2. Convert sats to tBTC EVM units
    /// 3. For cross-chain: subtract LZ messaging fee cost
    /// 4. Get DEX quote: how much USDT for the remaining tBTC?
    pub async fn prepare_from_sats(
        &self,
        destination: &str,
        chain: Chain,
        invoice_amount_sats: u64,
    ) -> Result<PreparedSwap, BoltzError> {
        if self.config.slippage_bps < 10 || self.config.slippage_bps > MAX_SLIPPAGE_BPS {
            return Err(BoltzError::Generic(format!(
                "slippage_bps must be >= 10 and <= {MAX_SLIPPAGE_BPS}"
            )));
        }

        parse_address(destination)?;

        let tbtc_pair = self.fetch_tbtc_pair().await?;

        // Validate against Boltz swap limits
        if invoice_amount_sats < tbtc_pair.limits.minimal
            || invoice_amount_sats > tbtc_pair.limits.maximal
        {
            return Err(BoltzError::AmountOutOfRange {
                amount: invoice_amount_sats,
                min: tbtc_pair.limits.minimal,
                max: tbtc_pair.limits.maximal,
            });
        }

        let fee_calc = compute_onchain_amount(&tbtc_pair, invoice_amount_sats)?;

        // Convert onchain sats to tBTC EVM units
        let tbtc_evm_units = u128::from(fee_calc.onchain_sats)
            .checked_mul(u128::from(SATS_TO_TBTC_FACTOR))
            .ok_or_else(|| BoltzError::Generic("tBTC amount overflow".into()))?;

        let usdt_output = if chain.is_source_chain() {
            // Same-chain: simple DEX quote
            self.fetch_quote_in_usdt(tbtc_evm_units).await?
        } else {
            // Cross-chain: matching web app's applyPostOftQuote exactly.
            // 1. DEX quote with full tBTC → initial USDT
            let initial_usdt = self.fetch_quote_in_usdt(tbtc_evm_units).await?;
            // 2. Quote OFT with that USDT to get messaging fee
            let (msg_fee_native, _) = self
                .quote_oft_messaging_fee(&chain, u128::from(initial_usdt))
                .await?;
            // 3. Convert messaging fee (ETH) to USDT cost
            let fee_usdt = if msg_fee_native == 0 {
                0u128
            } else {
                self.fetch_quote_out_usdt_for_eth(msg_fee_native).await?
            };
            // 4. Subtract USDT cost from USDT amount
            let adjusted_usdt =
                u128::from(initial_usdt)
                    .checked_sub(fee_usdt)
                    .ok_or_else(|| {
                        BoltzError::Generic(
                            "Amount too small to cover OFT cross-chain messaging fee".into(),
                        )
                    })?;
            if adjusted_usdt == 0 {
                return Err(BoltzError::Generic(
                    "Amount too small to cover OFT cross-chain messaging fee".into(),
                ));
            }
            // 5. Re-quote OFT with adjusted USDT to get final received amount
            let (_, oft_received) = self.quote_oft_messaging_fee(&chain, adjusted_usdt).await?;
            u64::try_from(oft_received)
                .map_err(|_| BoltzError::Generic("USDT amount overflow".into()))?
        };

        let now = current_unix_timestamp();
        Ok(PreparedSwap {
            destination_address: destination.to_string(),
            destination_chain: chain,
            usdt_amount: usdt_output,
            invoice_amount_sats,
            boltz_fee_sats: fee_calc.boltz_fee_sats,
            estimated_onchain_amount: fee_calc.onchain_sats,

            slippage_bps: self.config.slippage_bps,
            pair_hash: tbtc_pair.hash.clone(),
            expires_at: now.saturating_add(60),
        })
    }

    /// Maximum retries when encountering duplicate preimage errors. Handles
    /// races between concurrent instances; the consumer is responsible for
    /// syncing the key index on restore / cold start.
    const MAX_DUPLICATE_RETRIES: u32 = 10;

    /// Create the swap on Boltz. Returns the hold invoice to pay.
    ///
    /// If Boltz rejects the preimage hash as already used, the method bumps the
    /// key index and retries up to [`Self::MAX_DUPLICATE_RETRIES`] times. This
    /// handles races between concurrent instances sharing the same seed. For
    /// larger index gaps (e.g. restoring from mnemonic after many unpaid swaps),
    /// the consumer should sync the key index via
    /// [`BoltzStorage::set_key_index_if_higher`] before calling this method.
    pub async fn create(&self, prepared: &PreparedSwap) -> Result<CreatedSwap, BoltzError> {
        if current_unix_timestamp() >= prepared.expires_at {
            return Err(BoltzError::QuoteExpired);
        }
        let chain_id_u32 = to_chain_id_u32(self.config.chain_id)?;
        let gas_signer = self.key_manager.derive_gas_signer(chain_id_u32)?;

        let mut last_err = None;
        for _ in 0..Self::MAX_DUPLICATE_RETRIES {
            let key_index = self.store.increment_key_index(self.config.chain_id).await?;

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

            match self.api_client.create_reverse_swap(&create_req).await {
                Ok(resp) => {
                    return self
                        .finalize_swap(prepared, resp, key_index, &gas_signer)
                        .await;
                }
                Err(e) if e.is_duplicate_preimage() => {
                    tracing::warn!(
                        key_index,
                        "Preimage hash already used, bumping key index and retrying"
                    );
                    last_err = Some(e);
                }
                Err(e) => return Err(e),
            }
        }

        Err(last_err
            .unwrap_or_else(|| BoltzError::Generic("Exhausted duplicate preimage retries".into())))
    }

    /// Validate the Boltz response, persist the swap, and return the result.
    async fn finalize_swap(
        &self,
        prepared: &PreparedSwap,
        resp: crate::api::types::CreateReverseSwapResponse,
        key_index: u32,
        gas_signer: &crate::keys::EvmKeyPair,
    ) -> Result<CreatedSwap, BoltzError> {
        if resp.onchain_amount != prepared.estimated_onchain_amount {
            return Err(BoltzError::Generic(format!(
                "Boltz onchain_amount ({}) differs from prepared estimate ({})",
                resp.onchain_amount, prepared.estimated_onchain_amount,
            )));
        }

        let current_block = self.evm_provider.eth_block_number().await?;
        if resp.timeout_block_height <= current_block {
            return Err(BoltzError::Generic(format!(
                "Boltz returned expired timeout: block {} <= current {current_block}",
                resp.timeout_block_height,
            )));
        }

        let now = current_unix_timestamp();
        let swap = BoltzSwap {
            id: resp.id.clone(),
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
            erc20swap_address: self.erc20swap_address.clone(),
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
            swap_id: resp.id,
            invoice: resp.invoice,
            invoice_amount_sats: prepared.invoice_amount_sats,
            timeout_block_height: resp.timeout_block_height,
        })
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
            &self.erc20swap_address,
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

        let mut swap = BoltzSwap {
            id: format!("recovery-{}-{}", recoverable.key_index, now),
            status: BoltzSwapStatus::TbtcLocked,
            claim_key_index: recoverable.key_index,
            chain_id: self.config.chain_id,
            claim_address: format!("0x{}", hex::encode(recoverable.claim_address.as_slice())),
            destination_address: destination_address.to_string(),
            destination_chain: Chain::Arbitrum, // Recovery always claims to Arbitrum
            refund_address: format!("0x{}", hex::encode(recoverable.refund_address.as_slice())),
            erc20swap_address: self.erc20swap_address.clone(),
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

        // Recovery swaps have no creation-time quote — skip drift check.
        let completed = self.claim_and_swap(&mut swap, true).await?;
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

    async fn fetch_quote_in_usdt(&self, tbtc_evm_units: u128) -> Result<u64, BoltzError> {
        let quotes = self
            .api_client
            .get_quote_in(
                "ARB",
                ARBITRUM_TBTC_ADDRESS,
                ARBITRUM_USDT_ADDRESS,
                tbtc_evm_units,
            )
            .await?;
        let amount = pick_best_quote(&quotes, QuoteDirection::In)?;
        if amount == 0 {
            return Err(BoltzError::InvalidQuote(
                "DEX quote returned zero USDT".to_string(),
            ));
        }
        amount
            .try_into()
            .map_err(|_| BoltzError::Generic("USDT amount overflow".into()))
    }

    /// Claim tBTC locked on-chain and swap to USDT.
    /// On success the swap transitions to `Claiming` with `claim_tx_hash` set.
    /// The caller (`SwapManager`) is responsible for waiting for on-chain
    /// confirmation and transitioning to `Completed`.
    pub(crate) async fn claim_and_swap(
        &self,
        swap: &mut BoltzSwap,
        skip_drift_check: bool,
    ) -> Result<BoltzSwap, BoltzError> {
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
        let tbtc_evm_amount = U256::from(swap.onchain_amount)
            .checked_mul(U256::from(SATS_TO_TBTC_FACTOR))
            .ok_or_else(|| BoltzError::Generic("tBTC EVM amount overflow".into()))?;
        let timelock = U256::from(swap.timeout_block_height);

        // Verify the timelock hasn't expired before attempting the claim.
        // The on-chain contract enforces this too, but checking locally avoids
        // wasted gas and gives a clearer error.
        let current_block = self.evm_provider.eth_block_number().await?;
        if current_block >= swap.timeout_block_height {
            return Err(BoltzError::Generic(format!(
                "Swap timelock expired: current block {current_block} >= timeout {}",
                swap.timeout_block_height
            )));
        }

        for attempt in 0..MAX_CLAIM_RETRIES {
            if attempt > 0 {
                tracing::info!(attempt, swap_id = swap.id, "Retrying claim");
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
                    skip_drift_check,
                )
                .await;

            match result {
                Ok(_tx_hash) => {
                    return Ok(swap.clone());
                }
                Err(e) => {
                    // Quote drift is not transient — retrying immediately
                    // won't help. Return without marking Failed so the swap
                    // stays TbtcLocked and the consumer can accept the new
                    // rate via `accept_degraded_quote`.
                    if matches!(e, BoltzError::QuoteDegradedBeyondSlippage { .. }) {
                        return Err(e);
                    }

                    tracing::warn!(attempt, swap_id = swap.id, error = %e, "Claim attempt failed");

                    // Check if funds are still locked on-chain. If not, stop
                    // retrying — the swap was either claimed by another instance
                    // or refunded by Boltz. Don't mark success or failure here;
                    // the WS update will determine the final state.
                    match recover::is_swap_still_locked_by_swap(
                        &self.evm_provider,
                        swap,
                        &self.key_manager,
                    )
                    .await
                    {
                        Ok(false) => {
                            tracing::info!(
                                swap_id = swap.id,
                                "Funds no longer locked on-chain, stopping retries"
                            );
                            return Ok(swap.clone());
                        }
                        Ok(true) => {} // Still locked, worth retrying.
                        Err(check_err) => {
                            tracing::warn!(
                                swap_id = swap.id,
                                error = %check_err,
                                "On-chain lock check failed, continuing with retry"
                            );
                        }
                    }

                    if attempt < MAX_CLAIM_RETRIES.saturating_sub(1) {
                        sleep_1s().await;
                    } else {
                        swap.status = BoltzSwapStatus::Failed {
                            reason: format!("Claim failed after {MAX_CLAIM_RETRIES} attempts: {e}"),
                        };
                        swap.updated_at = current_unix_timestamp();
                        self.store.update_swap(swap).await?;
                        return Err(e);
                    }
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
        skip_drift_check: bool,
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
                skip_drift_check,
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
                skip_drift_check,
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
        skip_drift_check: bool,
    ) -> Result<String, BoltzError> {
        let (dex_calls, min_amount_out, raw_quote_usdt) = self
            .fetch_and_encode_dex_quote(tbtc_evm_amount, &addrs.router.to_string())
            .await?;

        if !skip_drift_check {
            check_quote_drift(swap.expected_usdt_amount, raw_quote_usdt, self.config.slippage_bps)?;
        }

        let erc20swap_sig = gas_signer.sign_eip712_erc20swap_claim(
            addrs.erc20swap,
            erc20swap_version,
            preimage,
            tbtc_evm_amount,
            addrs.tbtc,
            addrs.refund,
            timelock,
            addrs.router,
        )?;

        let router_sig = gas_signer.sign_eip712_router_claim(
            addrs.router,
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
        skip_drift_check: bool,
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
        if min_usdt_out == 0 {
            return Err(BoltzError::Generic(
                "Amount too small: slippage-adjusted USDT minimum is zero".into(),
            ));
        }

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

        // Drift check: compare what the user would receive on the destination
        // chain against the creation-time estimate (expected_usdt_amount).
        if !skip_drift_check {
            check_quote_drift(swap.expected_usdt_amount, min_amount_ld_raw, self.config.slippage_bps)?;
        }

        #[expect(clippy::arithmetic_side_effects)]
        let min_amount_ld_slipped = min_amount_ld_raw * slippage_factor / 10000;
        if min_amount_ld_slipped == 0 {
            return Err(BoltzError::Generic(
                "Amount too small: cross-chain slippage-adjusted minimum is zero".into(),
            ));
        }

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
            preimage,
            tbtc_evm_amount,
            addrs.tbtc,
            addrs.refund,
            timelock,
            addrs.router,
        )?;

        let router_sig = gas_signer.sign_eip712_router_claim_send(
            addrs.router,
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
        // Set Claiming BEFORE the Alchemy call so that on crash we know a
        // claim was attempted (even if we don't yet have the tx hash).
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

        // Persist the tx hash immediately so that on crash after this point
        // we can poll the chain for the receipt.
        swap.claim_tx_hash = Some(result.tx_hash.clone());
        swap.updated_at = current_unix_timestamp();
        self.store.update_swap(swap).await?;

        tracing::info!(
            tx_hash = result.tx_hash,
            swap_id = swap.id,
            "Claim submitted"
        );
        Ok(result.tx_hash)
    }

    // ─── OFT fee estimation (for prepare-time quoting) ─────────────────

    /// Find the OFT send amount required to deliver `target_amount` on the destination chain.
    /// Matches the web app's `quoteOftAmountInForAmountOut` binary search.
    async fn estimate_oft_required_send_amount(
        &self,
        chain: &Chain,
        target_amount: u128,
    ) -> Result<u128, BoltzError> {
        if target_amount == 0 {
            return Ok(0);
        }

        // Binary search: find the minimum send amount where OFT receive >= target
        let mut low = target_amount;
        let mut high = target_amount;

        // Phase 1: find upper bound
        // Safety: `attempts` is bounded to 32 iterations, and `low`/`high`
        // use checked arithmetic. The unchecked `+= 1` on a u32 capped at
        // 32 cannot overflow.
        let mut attempts = 0u32;
        loop {
            let (_, received) = self.quote_oft_messaging_fee(chain, high).await?;
            if received >= target_amount {
                break;
            }
            low = high
                .checked_add(1)
                .ok_or_else(|| BoltzError::Generic("OFT amount search overflow".into()))?;
            high = high
                .checked_mul(2)
                .ok_or_else(|| BoltzError::Generic("OFT amount search overflow".into()))?;
            #[expect(clippy::arithmetic_side_effects)]
            {
                attempts += 1;
            }
            if attempts > 32 {
                return Err(BoltzError::Generic(
                    "Could not find OFT send amount for target".into(),
                ));
            }
        }

        // Phase 2: binary search
        // Safety: `high >= low` is guaranteed by the while condition, so
        // `high - low` cannot underflow. `mid` is between `low` and `high`,
        // so `mid + 1 <= high` which fits in u128.
        #[expect(clippy::arithmetic_side_effects)]
        while low < high {
            let mid = low + (high - low) / 2;
            let (_, received) = self.quote_oft_messaging_fee(chain, mid).await?;
            if received >= target_amount {
                high = mid;
            } else {
                low = mid + 1;
            }
        }

        Ok(low)
    }

    /// Quote OFT messaging fee and received amount for a given USDT amount and destination chain.
    /// Returns `(native_fee, amount_received_on_destination)`.
    async fn quote_oft_messaging_fee(
        &self,
        chain: &Chain,
        usdt_amount: u128,
    ) -> Result<(u128, u128), BoltzError> {
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

        let send_param = contracts::build_oft_send_param(
            dst_info.lz_eid,
            alloy_primitives::Address::ZERO,
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
        let amount_received: u128 = receipt
            .amountReceivedLD
            .try_into()
            .map_err(|_| BoltzError::Generic("OFT amount too large".into()))?;

        Ok((native_fee, amount_received))
    }

    /// DEX quote: how much USDT needed to buy the given ETH amount.
    /// Used to convert LZ messaging fee (in ETH) to USDT cost.
    async fn fetch_quote_out_usdt_for_eth(&self, eth_amount: u128) -> Result<u128, BoltzError> {
        let quotes = self
            .api_client
            .get_quote_out("ARB", ARBITRUM_USDT_ADDRESS, ZERO_ADDRESS, eth_amount)
            .await?;
        pick_best_quote(&quotes, QuoteDirection::Out)
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

    /// Fetch a DEX quote for `tbtc_evm_amount` → USDT and encode it into
    /// calldata. Returns `(calls, min_amount_out, raw_quote_amount)` where
    /// `raw_quote_amount` is the best quote *before* slippage is applied
    /// (used for drift detection).
    #[expect(clippy::arithmetic_side_effects)]
    async fn fetch_and_encode_dex_quote(
        &self,
        tbtc_evm_amount: U256,
        router_address: &str,
    ) -> Result<(Vec<contracts::Call>, U256, u128), BoltzError> {
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
        let raw_quote_amount = best.amount;
        let slippage_factor = 10000 - u128::from(self.config.slippage_bps);
        let min_amount_out_u128 = best.amount * slippage_factor / 10000;
        if min_amount_out_u128 == 0 {
            return Err(BoltzError::Generic(
                "Amount too small: slippage-adjusted minimum is zero".into(),
            ));
        }
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

        Ok((calls, min_amount_out, raw_quote_amount))
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
///
/// Matches the Boltz web app formula for reverse swaps (using integer math):
///   `invoiceAmount = ceil((receiveAmount + minerFee) / (1 - percentage/100))`
///
/// The percentage from the API (e.g. `0.25` for 0.25%) is parsed from its string
/// representation to avoid floating-point imprecision.
fn compute_invoice_amount(
    pair: &ReversePairInfo,
    tbtc_evm_units: u128,
) -> Result<FeeCalc, BoltzError> {
    let sats_factor = u128::from(SATS_TO_TBTC_FACTOR);
    // Division by constant factor cannot fail
    let tbtc_sats = tbtc_evm_units.checked_div(sats_factor).unwrap_or(0);

    let miner_fees = u128::from(pair.fees.miner_fees.claim)
        .checked_add(u128::from(pair.fees.miner_fees.lockup))
        .ok_or_else(|| BoltzError::Generic("Miner fees overflow".into()))?;

    // Parse percentage from f64 to integer basis points to avoid floating-point imprecision.
    // The API returns values like 0.25 (meaning 0.25%). We need basis points of 100%,
    // i.e., 0.25% → 25 out of 10000.
    let pct_bps = parse_percentage_to_bps(pair.fees.percentage)?;

    // Web app formula: invoiceAmount = ceil((receiveAmount + minerFee) / (1 - pct/100))
    // In integer form: ceil((base * 10000) / (10000 - pct_bps))
    let base = tbtc_sats
        .checked_add(miner_fees)
        .ok_or_else(|| BoltzError::Generic("Fee base overflow".to_string()))?;
    let denominator = 10000u64
        .checked_sub(pct_bps)
        .ok_or_else(|| BoltzError::Generic("Invalid fee percentage (>= 100%)".to_string()))?;
    if denominator == 0 {
        return Err(BoltzError::Generic(
            "Invalid fee percentage (100%)".to_string(),
        ));
    }

    // ceil(base * 10000 / denominator)
    let numerator = base
        .checked_mul(10000)
        .ok_or_else(|| BoltzError::Generic("Invoice computation overflow".to_string()))?;
    let invoice = numerator.div_ceil(u128::from(denominator));
    let boltz_fee = invoice
        .checked_sub(base)
        .ok_or_else(|| BoltzError::Generic("Fee computation underflow".to_string()))?;
    let onchain = invoice
        .checked_sub(boltz_fee)
        .and_then(|v| v.checked_sub(miner_fees))
        .ok_or_else(|| BoltzError::Generic("Onchain amount underflow".to_string()))?;

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

/// Parse a fee percentage (e.g. 0.25 meaning 0.25%) to basis points of 100% (e.g. 25).
/// Uses string formatting to avoid floating-point imprecision when converting to integer.
fn parse_percentage_to_bps(percentage: f64) -> Result<u64, BoltzError> {
    // Format with enough precision to capture the API value, then parse as integer.
    // percentage * 100 gives basis points (0.25% * 100 = 25 bps).
    let s = format!("{:.4}", percentage * 100.0);
    let parts: Vec<&str> = s.split('.').collect();
    let whole: u64 = parts[0]
        .parse()
        .map_err(|_| BoltzError::Generic(format!("Invalid fee percentage: {percentage}")))?;
    // Check if fractional part is non-zero (meaning the percentage has sub-bps precision)
    if parts.len() > 1 && !parts[1].trim_end_matches('0').is_empty() {
        return Err(BoltzError::Generic(format!(
            "Fee percentage {percentage} has sub-basis-point precision, cannot represent exactly"
        )));
    }
    Ok(whole)
}

/// Compute the onchain amount from invoice sats (forward direction).
///
/// Matches the Boltz web app formula for reverse swaps:
///   `receiveAmount = sendAmount - ceil(sendAmount * percentage / 100) - minerFee`
fn compute_onchain_amount(
    pair: &ReversePairInfo,
    invoice_sats: u64,
) -> Result<FeeCalc, BoltzError> {
    let invoice = u128::from(invoice_sats);

    let pct_bps = parse_percentage_to_bps(pair.fees.percentage)?;
    let miner_fees = u128::from(pair.fees.miner_fees.claim)
        .checked_add(u128::from(pair.fees.miner_fees.lockup))
        .ok_or_else(|| BoltzError::Generic("Miner fees overflow".into()))?;

    // boltz_fee = ceil(invoice * pct_bps / 10000)
    let boltz_fee = invoice
        .checked_mul(u128::from(pct_bps))
        .ok_or_else(|| BoltzError::Generic("Fee computation overflow".into()))?
        .div_ceil(10000);

    let onchain = invoice
        .checked_sub(boltz_fee)
        .and_then(|v| v.checked_sub(miner_fees))
        .ok_or_else(|| BoltzError::Generic("Invoice amount too small to cover fees".into()))?;

    let to_u64 = |v: u128, name: &str| -> Result<u64, BoltzError> {
        v.try_into()
            .map_err(|_| BoltzError::Generic(format!("{name} overflow")))
    };

    Ok(FeeCalc {
        invoice_sats,
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

pub(crate) fn current_unix_timestamp() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs(),
        Err(e) => {
            tracing::error!("System clock before UNIX epoch: {e}, returning 0");
            0
        }
    }
}

async fn sleep_1s() {
    platform_utils::tokio::time::sleep(platform_utils::time::Duration::from_secs(1)).await;
}

/// Check that the fresh DEX quote hasn't degraded beyond the slippage
/// tolerance compared to the creation-time estimate. Mirrors the Boltz web
/// app's `isOutsideSlippage` check in `TransactionConfirmed.tsx`.
#[expect(clippy::arithmetic_side_effects)]
fn check_quote_drift(
    expected_usdt: u64,
    fresh_quote_usdt: u128,
    slippage_bps: u32,
) -> Result<(), BoltzError> {
    let threshold =
        u128::from(expected_usdt) * (10000 - u128::from(slippage_bps)) / 10000;
    if fresh_quote_usdt < threshold {
        let quoted = fresh_quote_usdt.try_into().unwrap_or(u64::MAX);
        return Err(BoltzError::QuoteDegradedBeyondSlippage {
            expected_usdt,
            quoted_usdt: quoted,
        });
    }
    Ok(())
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
    fn test_parse_percentage_to_bps() {
        assert_eq!(parse_percentage_to_bps(0.25).unwrap(), 25);
        assert_eq!(parse_percentage_to_bps(0.5).unwrap(), 50);
        assert_eq!(parse_percentage_to_bps(1.0).unwrap(), 100);
        assert_eq!(parse_percentage_to_bps(0.0).unwrap(), 0);
        assert_eq!(parse_percentage_to_bps(0.1).unwrap(), 10);
    }

    #[test]
    fn test_parse_percentage_to_bps_sub_bps_rejected() {
        // 0.125% = 12.5 bps — sub-bps precision
        assert!(parse_percentage_to_bps(0.125).is_err());
    }

    fn test_pair(percentage: f64) -> ReversePairInfo {
        ReversePairInfo {
            hash: "abc".to_string(),
            rate: 1.0,
            limits: crate::api::types::PairLimits {
                minimal: 10000,
                maximal: 25_000_000,
            },
            fees: crate::api::types::ReversePairFees {
                percentage,
                miner_fees: crate::api::types::MinerFees {
                    claim: 2,
                    lockup: 6,
                },
            },
        }
    }

    #[test]
    fn test_compute_onchain_amount() {
        let pair = test_pair(0.25);
        let result = compute_onchain_amount(&pair, 10000).unwrap();

        // boltz_fee = ceil(10000 * 25 / 10000) = 25
        assert_eq!(result.boltz_fee_sats, 25);
        // onchain = 10000 - 25 - 8 = 9967
        assert_eq!(result.onchain_sats, 9967);
        assert_eq!(result.invoice_sats, 10000);
    }

    #[test]
    fn test_compute_onchain_amount_too_small() {
        let pair = test_pair(0.25);
        // Amount too small to cover miner fees
        assert!(compute_onchain_amount(&pair, 5).is_err());
    }

    #[test]
    fn test_compute_invoice_and_onchain_roundtrip() {
        // Verify that compute_invoice_amount and compute_onchain_amount are consistent:
        // compute_onchain_amount(compute_invoice_amount(x).invoice_sats).onchain_sats
        // should be close to the original tbtc_sats (within rounding).
        let pair = test_pair(0.25);
        let tbtc_evm_units: u128 = 100_000 * u128::from(SATS_TO_TBTC_FACTOR);
        let invoice = compute_invoice_amount(&pair, tbtc_evm_units).unwrap();
        let back = compute_onchain_amount(&pair, invoice.invoice_sats).unwrap();

        // onchain_sats from roundtrip should match the original (100_000)
        // Allow 1 sat tolerance for ceiling rounding
        let diff = invoice.onchain_sats.abs_diff(back.onchain_sats);
        assert!(
            diff <= 1,
            "roundtrip diff={diff}, invoice_onchain={}, back_onchain={}",
            invoice.onchain_sats,
            back.onchain_sats
        );
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

    #[test]
    fn test_check_quote_drift_within_tolerance() {
        // Expected 1000 USDT, got 995 (0.5% drop), slippage 1% → OK
        assert!(check_quote_drift(1_000_000, 995_000, 100).is_ok());
    }

    #[test]
    fn test_check_quote_drift_at_boundary() {
        // Expected 1000 USDT, got 990 (exactly 1% drop), slippage 1% → OK
        assert!(check_quote_drift(1_000_000, 990_000, 100).is_ok());
    }

    #[test]
    fn test_check_quote_drift_beyond_tolerance() {
        // Expected 1000 USDT, got 980 (2% drop), slippage 1% → error
        let err = check_quote_drift(1_000_000, 980_000, 100).unwrap_err();
        assert!(matches!(
            err,
            BoltzError::QuoteDegradedBeyondSlippage {
                expected_usdt: 1_000_000,
                quoted_usdt: 980_000,
            }
        ));
    }

    #[test]
    fn test_check_quote_drift_better_quote_ok() {
        // Expected 1000 USDT, got 1050 (better!) → always OK
        assert!(check_quote_drift(1_000_000, 1_050_000, 100).is_ok());
    }

    #[test]
    fn test_check_quote_drift_zero_expected() {
        // Recovery swaps have expected=0, should always pass
        // (but in practice skip_drift_check=true is used for recovery)
        assert!(check_quote_drift(0, 500_000, 100).is_ok());
    }
}
