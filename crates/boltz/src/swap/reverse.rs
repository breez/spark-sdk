use std::sync::Arc;

use alloy_primitives::U256;

use crate::api::BoltzApiClient;
use crate::api::types::{EncodeRequest, ReversePairInfo};
use crate::api::ws::{SwapStatusSubscriber, SwapStatusUpdate};
use crate::config::{
    ARBITRUM_ERC20SWAP_ADDRESS, ARBITRUM_ROUTER_ADDRESS, ARBITRUM_TBTC_ADDRESS,
    ARBITRUM_USDT_ADDRESS, BoltzConfig, SATS_TO_TBTC_FACTOR,
};
use crate::error::BoltzError;
use crate::evm::alchemy::{AlchemyGasClient, EvmCall};
use crate::evm::contracts::{
    self, Erc20Claim, encode_claim_erc20_execute, parse_address, quote_calldata_to_call,
};
use crate::evm::provider::EvmProvider;
use crate::evm::signing::EvmSigner;
use crate::keys::EvmKeyManager;
use crate::models::{
    BoltzSwap, BoltzSwapStatus, Chain, CompletedSwap, CreatedSwap, PreparedSwap, SwapLimits,
};
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
    store: Arc<dyn BoltzStore>,
    config: BoltzConfig,
}

impl ReverseSwapExecutor {
    pub fn new(
        api_client: BoltzApiClient,
        ws_subscriber: SwapStatusSubscriber,
        key_manager: EvmKeyManager,
        alchemy_client: AlchemyGasClient,
        evm_provider: EvmProvider,
        store: Arc<dyn BoltzStore>,
        config: BoltzConfig,
    ) -> Self {
        Self {
            api_client,
            ws_subscriber,
            key_manager,
            alchemy_client,
            evm_provider,
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
    /// 2. Convert tBTC EVM units to sats
    /// 3. Apply Boltz fee to get total sats needed
    pub async fn prepare(
        &self,
        destination: &str,
        chain: Chain,
        usdt_amount: u64,
    ) -> Result<PreparedSwap, BoltzError> {
        if chain != Chain::Arbitrum {
            return Err(BoltzError::Generic(
                "Only Arbitrum destination is supported in v1".to_string(),
            ));
        }
        if self.config.slippage_bps < 10 {
            return Err(BoltzError::Generic(
                "slippage_bps must be at least 10 (0.1%)".to_string(),
            ));
        }

        let tbtc_pair = self.fetch_tbtc_pair().await?;
        let tbtc_evm_units = self.fetch_quote_out_tbtc(usdt_amount).await?;
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
        let preimage_hash = self.key_manager.derive_preimage_hash(chain_id_u32, key_index)?;
        let preimage_key = self.key_manager.derive_preimage_key(chain_id_u32, key_index)?;

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

    /// Resume all active (non-final) swaps. Returns list of swap IDs being resumed.
    pub async fn resume_active_swaps(&self) -> Result<Vec<String>, BoltzError> {
        let active = self.store.list_active_swaps().await?;
        for swap in &active {
            tracing::info!(
                swap_id = swap.id,
                boltz_id = swap.boltz_id,
                status = ?swap.status,
                "Resuming active swap"
            );
        }
        Ok(active.into_iter().map(|s| s.id).collect())
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
        let quote = quotes.first().ok_or_else(|| BoltzError::Api {
            reason: "No DEX quote returned".to_string(),
            code: None,
        })?;
        let amount: u128 = quote.quote.parse().map_err(|_| BoltzError::Api {
            reason: format!("Invalid quote amount: {}", quote.quote),
            code: None,
        })?;
        if amount == 0 {
            return Err(BoltzError::InvalidQuote("DEX quote returned zero tBTC".to_string()));
        }
        Ok(amount)
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
        let erc20swap_version = self.fetch_erc20swap_version(&swap.erc20swap_address).await?;

        let addrs = ClaimAddresses::parse(swap)?;
        let tbtc_evm_amount = U256::from(swap.onchain_amount).saturating_mul(U256::from(SATS_TO_TBTC_FACTOR));
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

        swap.status = BoltzSwapStatus::Claiming;
        swap.updated_at = current_unix_timestamp();
        self.store.update_swap(swap).await?;

        let evm_call = EvmCall {
            to: swap.router_address.clone(),
            value: None,
            data: Some(format!("0x{}", hex::encode(&calldata))),
        };

        let result = self
            .alchemy_client
            .send_sponsored_calls(vec![evm_call], swap.chain_id)
            .await?;

        tracing::info!(tx_hash = result.tx_hash, swap_id = swap.boltz_id, "Claim confirmed");
        Ok(result.tx_hash)
    }

    async fn fetch_erc20swap_version(&self, erc20swap_address: &str) -> Result<String, BoltzError> {
        let calldata = contracts::encode_version_call();
        let result = self.evm_provider.eth_call(erc20swap_address, &calldata).await?;
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
            .get_quote_in("ARB", ARBITRUM_TBTC_ADDRESS, ARBITRUM_USDT_ADDRESS, amount_in)
            .await?;
        let quote = quotes.first().ok_or_else(|| BoltzError::Api {
            reason: "No DEX quote returned".to_string(),
            code: None,
        })?;

        let amount_out: u128 = quote.quote.parse().map_err(|_| BoltzError::Api {
            reason: format!("Invalid quote amount: {}", quote.quote),
            code: None,
        })?;
        if amount_out == 0 {
            return Err(BoltzError::InvalidQuote("DEX quote returned zero USDT".to_string()));
        }
        let slippage_factor = 10000 - u128::from(self.config.slippage_bps);
        let min_amount_out_u128 = amount_out * slippage_factor / 10000;
        let min_amount_out = U256::from(min_amount_out_u128);

        let encode_req = EncodeRequest {
            recipient: router_address.to_string(),
            amount_in,
            amount_out_min: min_amount_out_u128,
            data: quote.data.clone(),
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
#[expect(clippy::arithmetic_side_effects, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn compute_invoice_amount(pair: &ReversePairInfo, tbtc_evm_units: u128) -> Result<FeeCalc, BoltzError> {
    let sats_factor = u128::from(SATS_TO_TBTC_FACTOR);
    let tbtc_sats = tbtc_evm_units / sats_factor;

    let miner_fees = u128::from(pair.fees.miner_fees.claim) + u128::from(pair.fees.miner_fees.lockup);
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

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
async fn sleep_1s() {
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
}

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
async fn sleep_1s() {
    futures_util::future::pending::<()>().await;
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
}
