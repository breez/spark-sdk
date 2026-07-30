//! Mainnet cross-chain send itests (env-gated).
//!
//! Performs a real cross-chain send from the funded test account ("Alice") to a
//! **deterministic EVM address derived from the test mnemonic**, then verifies
//! receipt *independently* by reading the recipient's ERC-20 balance over
//! JSON-RPC. This exercises the fees-excluded / target-overpay guarantee: the
//! recipient should land at or above the requested target.
//!
//! Target chain is **Arbitrum One**, the only chain offering a USD-stable asset
//! on both providers: Orchestra `USDB→USDC` (exercises the USD-stable
//! target-overpay path) and Boltz `BTC→USDT`.
//!
//! Funds are **not** swept back (the SDK has no Spark-inbound path from EVM).
//! They accumulate at the deterministic recipient, recoverable by importing the
//! mnemonic at `m/44'/60'/0'/0/0`; each run logs the address + balance. Keep the
//! source-amount overrides below small to bound accumulation.
//!
//! NOTE: the cross-chain `amount` is denominated in the **source** asset, not the
//! destination: USDB base units for the Orchestra case, **sats** for the Boltz
//! (BTC source) case. The two are sized via separate env vars accordingly.
//!
//! # Required environment variables
//! - `MAINNET_TEST_MNEMONIC` — mnemonic of the funded test account. Primary gate.
//! - `BREEZ_API_KEY` — API key the mainnet SDK requires.
//!
//! # Optional
//! - `MAINNET_TEST_TOKEN_ID` — USD-stable source token; defaults to USDB.
//! - `MAINNET_TEST_CROSS_CHAIN_USDB` — Orchestra source in USDB base units
//!   (6-decimals); defaults to 1_000_000 (= 1.00 USDB, ~1.00 USDC delivered).
//! - `MAINNET_TEST_CROSS_CHAIN_SATS` — Boltz source in **sats** (BTC source);
//!   defaults to 1_600. The recipient lands the fiat-equivalent in USDT.
//!
//! # Run locally
//! ```bash
//! MAINNET_TEST_MNEMONIC="..." BREEZ_API_KEY="..." \
//!   cargo test -p breez-sdk-itest --test mainnet_cross_chain -- --test-threads=1 --nocapture
//! ```

use std::time::{Duration, Instant};

use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use tracing::{debug, info, warn};

/// Destination chain for both tests: Arbitrum One. Matched on `chain_id` (the
/// decimal EVM chainId), since providers spell the chain *name* differently
/// (Orchestra reports `"arbitrum"`, Boltz reports `"Arbitrum One"`); `chain_id`
/// is the stable cross-provider key.
const TARGET_CHAIN_ID: &str = "42161";

/// Whether a route's destination is Arbitrum One. Prefers `chain_id`; falls back
/// to the normalized chain name for the (documented) case where a provider
/// doesn't surface a chainId. The name set is exact (not a prefix) so sibling
/// chains like "Arbitrum Nova" (chainId 42170) don't match.
fn is_target_chain(route: &CrossChainRoutePair) -> bool {
    if route.chain_id.as_deref() == Some(TARGET_CHAIN_ID) {
        return true;
    }
    matches!(
        route.chain.to_lowercase().replace(['-', '_'], " ").trim(),
        "arbitrum" | "arbitrum one"
    )
}

// IMPORTANT: the cross-chain `amount` is in the **source** asset's units, not the
// destination's. For the Orchestra (USDB token) case that's USDB base units; for
// the Boltz (BTC) case that's **sats**. They are sized and asserted separately.

/// Orchestra source budget in USDB base units (6-decimals), so 1_000_000 = 1.00
/// USDB. USDB↔USDC are ~1:1, so this also approximates the USDC the recipient
/// lands. Override via `MAINNET_TEST_CROSS_CHAIN_USDB`. A too-small value (below
/// the route minimum) makes `prepare` error (loudly).
const DEFAULT_ORCHESTRA_SOURCE_USDB: u128 = 1_000_000;

/// Boltz source budget in **sats** (BTC source). Override via
/// `MAINNET_TEST_CROSS_CHAIN_SATS`. Note this is sats, NOT a USD amount: the
/// recipient lands the fiat-equivalent in USDT. A value below the route minimum
/// makes `prepare` error (loudly).
const DEFAULT_BOLTZ_SOURCE_SATS: u128 = 1_600;

/// Tolerance (bps) allowed between the SDK-reported `delivered_amount` and the
/// balance actually observed on-chain, to absorb rounding / settlement timing.
const ONCHAIN_TOLERANCE_BPS: u128 = 100;

/// How far below the *requested* amount a successful delivery may legitimately
/// land, in bps. The SDK tolerates up to its default slippage (~100 bps) between
/// quote and execution, while the fees-excluded overpay pad is only ~15 bps, so
/// a fully successful send can land somewhat under target. Sized above the SDK
/// slippage default with headroom for bridge/rounding dust, so a real success
/// isn't flagged as a shortfall.
const SLIPPAGE_TOLERANCE_BPS: u128 = 150;

/// Settlement timeout for both the SDK status poll and the on-chain balance
/// poll. Cross-chain bridging (CCTP / LayerZero) adds minutes of latency.
const SETTLE_TIMEOUT_SECS: u64 = 600;

/// Poll interval while waiting on the SDK's conversion status to go terminal.
const STATUS_POLL_INTERVAL: Duration = Duration::from_secs(10);

fn env_amount_or(var: &str, default: u128) -> u128 {
    std::env::var(var)
        .ok()
        .and_then(|s| s.trim().parse::<u128>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(default)
}

/// `amount` reduced by `bps` basis points (floored), e.g. a lower bound that
/// allows for slippage. Saturating, so it never underflows.
fn reduce_by_bps(amount: u128, bps: u128) -> u128 {
    amount.saturating_sub(amount.saturating_mul(bps) / 10_000)
}

/// Alice's `(sats, token)` balances, for the per-test cost breadcrumb. Syncs
/// first so the read reflects the latest state.
async fn alice_balances(alice: &SdkInstance, token_id: &str) -> Result<(u64, u128)> {
    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let info = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?;
    let tokens = info
        .token_balances
        .get(token_id)
        .map(|b| b.balance)
        .unwrap_or(0);
    Ok((info.balance_sats, tokens))
}

/// Log Alice's net sats/token movement over a test — the cross-chain flow is
/// Alice → external chain, so her drain (plus any setup conversion) is the cost.
fn log_cost(label: &str, token_id: &str, pre: (u64, u128), post: (u64, u128)) {
    let sats_delta = i128::from(post.0) - i128::from(pre.0);
    let token_delta =
        i128::try_from(post.1).unwrap_or(i128::MAX) - i128::try_from(pre.1).unwrap_or(i128::MAX);
    info!("[cross-chain-cost] {label}: alice sats Δ {sats_delta}, {token_id} Δ {token_delta}");
}

/// Orchestra: USD-stable token (USDB) → USDC on Arbitrum, fees-excluded. This is
/// the case that exercises target-overpay (USD-stable source→destination).
#[test_log::test(tokio::test)]
async fn test_cross_chain_orchestra_fees_excluded_evm() -> Result<()> {
    let Some((mut alice, token_id, mnemonic)) = mainnet_cross_chain_setup().await? else {
        return Ok(());
    };
    info!("=== Starting test_cross_chain_orchestra_fees_excluded_evm ===");
    let pre = alice_balances(&alice, &token_id).await?;

    // Source budget in USDB base units (USD-stable, ~1:1 with the USDC delivered).
    let usdb_amount = env_amount_or(
        "MAINNET_TEST_CROSS_CHAIN_USDB",
        DEFAULT_ORCHESTRA_SOURCE_USDB,
    );

    // Alice needs USDB to spend; ensure she holds ~1.3x the source amount (to
    // cover the send plus fees with a little headroom), topping up from her own
    // sats if short.
    let required_usdb = usdb_amount.saturating_mul(13) / 10;
    if !ensure_wallet_has_tokens(
        &alice,
        &alice,
        "Alice",
        &token_id,
        required_usdb,
        required_usdb,
    )
    .await?
    {
        warn!("Could not top up Alice with {token_id}; skipping");
        return Ok(());
    }

    let (recipient, _signer) = mainnet_evm_recipient(&mnemonic)?;
    // No conversion_options: Orchestra accepts USDB directly as a source, so the
    // dispatcher uses the direct USDB-source path (no AMM hop) and applies the
    // fees-excluded target-overpay on the stable-token source.
    run_cross_chain_evm_send(
        &mut alice,
        &recipient,
        CrossChainProvider::Orchestra,
        "USDC",
        Some(token_id.clone()),
        None,
        usdb_amount,
        // USDB↔USDC parity: the recipient should land at least the requested USDB
        // amount in USDC base units (the fees-excluded / overpay guarantee).
        Some(usdb_amount),
    )
    .await?;

    log_cost(
        "orchestra",
        &token_id,
        pre,
        alice_balances(&alice, &token_id).await?,
    );
    Ok(())
}

/// Boltz: BTC (Alice's sats) → USDT on Arbitrum, fees-excluded. Exercises a
/// different asset + provider than the Orchestra case, on the same chain.
#[test_log::test(tokio::test)]
async fn test_cross_chain_boltz_fees_excluded_evm() -> Result<()> {
    let Some((mut alice, token_id, mnemonic)) = mainnet_cross_chain_setup().await? else {
        return Ok(());
    };
    info!("=== Starting test_cross_chain_boltz_fees_excluded_evm ===");
    let pre = alice_balances(&alice, &token_id).await?;

    // Source budget in SATS (BTC source). The recipient lands the fiat-equivalent
    // in USDT, so there's no parity unit to assert against the sats amount.
    let sats_amount = env_amount_or("MAINNET_TEST_CROSS_CHAIN_SATS", DEFAULT_BOLTZ_SOURCE_SATS);
    let (recipient, _signer) = mainnet_evm_recipient(&mnemonic)?;
    run_cross_chain_evm_send(
        &mut alice,
        &recipient,
        CrossChainProvider::Boltz,
        "USDT",
        None, // BTC source
        None, // no conversion: amount is sats
        sats_amount,
        None, // sats source has no parity with the USDT delivered
    )
    .await?;

    log_cost(
        "boltz",
        &token_id,
        pre,
        alice_balances(&alice, &token_id).await?,
    );
    Ok(())
}

/// Shared flow: discover the route, baseline the recipient's on-chain balance,
/// prepare+send fees-excluded, wait for the SDK to report `Completed`, then
/// verify the on-chain receipt. Skips (warn + `Ok`) when no matching route
/// exists or Alice is underfunded.
///
/// - `amount` is in the route's **source** units (USDB base units for a token
///   source, sats for a BTC source).
/// - `parity_min_out` is the minimum delivery to assert in **destination** base
///   units, set only when the source is parity-equivalent to the destination
///   (USDB→USDC). `None` for non-parity sources (BTC→USDT), where we instead
///   verify the on-chain balance against the SDK's reported `delivered_amount`.
#[allow(clippy::too_many_arguments)]
async fn run_cross_chain_evm_send(
    alice: &mut SdkInstance,
    recipient: &str,
    provider: CrossChainProvider,
    asset: &str,
    token_identifier: Option<String>,
    conversion_options: Option<ConversionOptions>,
    amount: u128,
    parity_min_out: Option<u128>,
) -> Result<()> {
    // Log the destination up front (derived at m/44'/60'/0'/0/0 of the test
    // mnemonic) so it's captured before any send — this is the address holding
    // any funds that need manual recovery.
    info!("Cross-chain {provider:?} {asset} send → EVM recipient {recipient} (m/44'/60'/0'/0/0)");

    // 1. Discover the route for this EVM recipient, then pick provider+chain+asset.
    let address_details = CrossChainAddressDetails {
        address: recipient.to_string(),
        address_family: CrossChainAddressFamily::Evm,
        contract_address: None,
        chain_id: None,
        amount: None,
    };
    let routes = alice
        .sdk
        .get_cross_chain_routes(&CrossChainRouteFilter::Send { address_details })
        .await?;
    let Some(route) = routes.into_iter().find(|r| {
        r.provider == provider && is_target_chain(r) && r.asset.eq_ignore_ascii_case(asset)
    }) else {
        warn!("No {provider:?} {asset} route on Arbitrum One; skipping");
        return Ok(());
    };
    let contract = route
        .contract_address
        .clone()
        .ok_or_else(|| anyhow::anyhow!("{provider:?} {asset} route has no contract address"))?;
    let Some(rpc_url) = evm_rpc_url(&route.chain) else {
        warn!("No JSON-RPC endpoint for chain {}; skipping", route.chain);
        return Ok(());
    };
    info!(
        "Selected route: {asset} on {} ({contract}) [{provider:?}], recipient {recipient}",
        route.chain
    );

    // 2. Baseline the recipient's on-chain balance before sending.
    let baseline = evm_erc20_balance(&rpc_url, &contract, recipient).await?;
    info!("Recipient baseline {asset} balance: {baseline}");

    // 3. Prepare fees-excluded with default target-overpay.
    let prepared = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::CrossChain {
                address: recipient.to_string(),
                route,
                max_slippage_bps: None,
                target_overpay_bps: None,
            },
            amount: Some(amount),
            token_identifier: token_identifier.clone(),
            conversion_options,
            fee_policy: Some(FeePolicy::FeesExcluded),
        })
        .await?;

    // 4. Assert the fees-excluded contract.
    let (fee_mode, estimated_out, amount_in, source_transfer_fee_sats) = match &prepared
        .payment_method
    {
        SendPaymentMethod::CrossChainAddress {
            fee_mode,
            estimated_out,
            amount_in,
            asset_amount_in,
            fee_amount,
            service_fee_amount,
            service_fee_asset,
            source_transfer_fee_sats,
            expires_at,
            ..
        } => {
            info!(
                "Cross-chain quote [{provider:?} {asset}]: amount_in={amount_in} \
                     asset_amount_in={asset_amount_in} estimated_out={estimated_out} \
                     fee_amount={fee_amount} service_fee={service_fee_amount} {service_fee_asset:?} \
                     source_transfer_fee_sats={source_transfer_fee_sats} fee_mode={fee_mode:?} \
                     expires_at={expires_at}"
            );
            (
                *fee_mode,
                *estimated_out,
                *amount_in,
                *source_transfer_fee_sats,
            )
        }
        other => anyhow::bail!("expected CrossChainAddress payment method, got {other:?}"),
    };
    assert_eq!(
        fee_mode,
        CrossChainFeeMode::FeesExcluded,
        "cross-chain prepare should run under FeesExcluded"
    );
    assert!(
        estimated_out > 0,
        "prepare should estimate a positive delivery"
    );
    // For parity sources, the prepare-time estimate (in destination units)
    // should be ~the requested amount: the fees-excluded / overpay guarantee.
    // Allow a slippage band below it — the SDK tolerates quote-vs-execution
    // slippage larger than the overpay pad, so the estimate can sit slightly
    // under target without the send being wrong.
    if let Some(min_out) = parity_min_out {
        let floor = reduce_by_bps(min_out, SLIPPAGE_TOLERANCE_BPS);
        assert!(
            estimated_out >= floor,
            "fees-excluded estimated_out {estimated_out} should be >= {floor} \
             (requested {min_out} less {SLIPPAGE_TOLERANCE_BPS} bps)"
        );
    }

    // 5. Funding check (skip if Alice can't cover the source leg + transfer fee).
    if !alice_can_cover(
        alice,
        token_identifier.as_deref(),
        amount_in,
        source_transfer_fee_sats,
    )
    .await?
    {
        warn!("Alice underfunded for the cross-chain send; skipping");
        return Ok(());
    }

    // 6. Send.
    let resp = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepared,
            options: None,
            idempotency_key: None,
        })
        .await?;
    let payment_id = resp.payment.id.clone();
    info!("Cross-chain send dispatched: payment {payment_id}");

    // 7. Wait for the SDK's background monitor to report a terminal status.
    let delivered =
        wait_for_cross_chain_completion(&alice.sdk, &payment_id, SETTLE_TIMEOUT_SECS).await?;
    info!("SDK reports conversion Completed, delivered_amount = {delivered:?}");

    // 8. Verify delivery (all in destination base units).
    //
    // `delivered_amount` (SDK-reported, per-payment) is the source of truth for
    // "how much this send delivered"; the on-chain read is independent
    // corroboration that those funds actually landed. Anchoring the pass
    // criterion on `delivered_amount` (not the raw balance delta) is what keeps
    // standing funds from prior runs at this reused address from masking a
    // genuine shortfall — a short delivery shows up in `delivered_amount`, and
    // stale funds can only ever help the on-chain floor be met sooner.
    // (A fresh recipient per run would fully isolate runs, but conflicts with
    // the deterministic-recovery design; the residual is a narrow window where
    // the SDK over-reports AND stale funds happen to cover it.)
    //
    // For a parity (USD-stable) source, also assert the fees-excluded guarantee:
    // the recipient lands ~the requested amount, within the slippage band.
    if let (Some(target), Some(d)) = (parity_min_out, delivered) {
        let floor = reduce_by_bps(target, SLIPPAGE_TOLERANCE_BPS);
        assert!(
            d >= floor,
            "fees-excluded: delivered {d} should be >= {floor} \
             (requested {target} less {SLIPPAGE_TOLERANCE_BPS} bps) — recipient landed short"
        );
    }

    // Minimum increase we require to observe on-chain.
    let onchain_floor = match (delivered, parity_min_out) {
        // Trust the SDK's per-payment delivered amount; confirm it landed.
        (Some(d), _) => reduce_by_bps(d, ONCHAIN_TOLERANCE_BPS),
        // delivered unknown (terminal reached before the amount was recorded)
        // but we have a parity target: require ~that amount, slippage-adjusted.
        (None, Some(target)) => reduce_by_bps(target, SLIPPAGE_TOLERANCE_BPS),
        // No SDK amount and no parity target: only confirm some funds arrived.
        (None, None) => {
            warn!(
                "Conversion Completed without a delivered_amount and no parity target; \
                 verifying only that some funds arrived on-chain"
            );
            1
        }
    };
    let final_balance = wait_for_evm_balance_increase(
        &rpc_url,
        &contract,
        recipient,
        baseline,
        onchain_floor,
        SETTLE_TIMEOUT_SECS,
    )
    .await?;
    let on_chain_delta = final_balance.saturating_sub(baseline);
    info!(
        "On-chain {asset} delta = {on_chain_delta} (required floor {onchain_floor}, \
         SDK delivered {delivered:?})"
    );

    log_evm_recovery_balance(&rpc_url, &contract, recipient, asset).await;
    Ok(())
}

/// Whether Alice can cover the source leg of a cross-chain send. For a token
/// source she needs `amount_in` token base units plus the sats transfer fee; for
/// a BTC source she needs `amount_in + source_transfer_fee_sats` sats.
async fn alice_can_cover(
    alice: &SdkInstance,
    token_identifier: Option<&str>,
    amount_in: u128,
    source_transfer_fee_sats: u64,
) -> Result<bool> {
    let info = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?;
    match token_identifier {
        Some(tid) => {
            let token_balance = info.token_balances.get(tid).map(|b| b.balance).unwrap_or(0);
            let ok = token_balance >= amount_in
                && u128::from(info.balance_sats) >= u128::from(source_transfer_fee_sats);
            if !ok {
                warn!(
                    "Funding check: have {token_balance} {tid} + {} sats; \
                     need {amount_in} {tid} + {source_transfer_fee_sats} sats",
                    info.balance_sats
                );
            }
            Ok(ok)
        }
        None => {
            let need = amount_in.saturating_add(u128::from(source_transfer_fee_sats));
            let ok = u128::from(info.balance_sats) >= need;
            if !ok {
                warn!(
                    "Funding check: have {} sats; need {need} sats \
                     (amount_in {amount_in} + transfer fee {source_transfer_fee_sats})",
                    info.balance_sats
                );
            }
            Ok(ok)
        }
    }
}

/// Short description of a conversion's provider handles, for correlating a run
/// with the provider dashboard / chain explorer when debugging.
fn describe_conversion(info: &ConversionInfo) -> String {
    match info {
        ConversionInfo::Orchestra {
            order_id,
            quote_id,
            chain,
            asset,
            recipient_address,
            ..
        } => format!(
            "Orchestra order_id={order_id} quote_id={quote_id} chain={chain} \
             asset={asset} recipient={recipient_address}"
        ),
        ConversionInfo::Boltz {
            swap_id,
            chain,
            asset,
            bridge_ref,
            recipient_address,
            ..
        } => format!(
            "Boltz swap_id={swap_id} chain={chain} asset={asset} \
             bridge_ref={bridge_ref:?} recipient={recipient_address}"
        ),
        ConversionInfo::Amm { conversion_id, .. } => {
            format!("AMM conversion_id={conversion_id} (unexpected for a cross-chain send)")
        }
    }
}

/// Poll the payment until its cross-chain `ConversionInfo` reaches a terminal
/// state. Returns `delivered_amount` on `Completed`; errors on a failed/refunded
/// outcome or timeout. Logs the provider handles once and every status change,
/// so a debugging run has a trail to correlate against the provider/explorer.
async fn wait_for_cross_chain_completion(
    sdk: &BreezSdk,
    payment_id: &str,
    timeout_secs: u64,
) -> Result<Option<u128>> {
    let start = Instant::now();
    let mut handles_logged = false;
    let mut last_status: Option<String> = None;
    loop {
        let payment = sdk
            .get_payment(GetPaymentRequest {
                payment_id: payment_id.to_string(),
            })
            .await?
            .payment;

        let conversion_info = payment.details.as_ref().and_then(|d| match d {
            PaymentDetails::Spark {
                conversion_info, ..
            }
            | PaymentDetails::Token {
                conversion_info, ..
            }
            | PaymentDetails::Lightning {
                conversion_info, ..
            } => conversion_info.as_ref(),
            _ => None,
        });

        if let Some(info) = conversion_info {
            if !handles_logged {
                info!("Cross-chain conversion: {}", describe_conversion(info));
                handles_logged = true;
            }
            let (status, delivered) = match info {
                ConversionInfo::Orchestra {
                    status,
                    delivered_amount,
                    ..
                }
                | ConversionInfo::Boltz {
                    status,
                    delivered_amount,
                    ..
                } => (status, *delivered_amount),
                // A cross-chain send should carry an Orchestra/Boltz conversion,
                // never a plain AMM one. Fail fast rather than spin to timeout.
                ConversionInfo::Amm { status, .. } => anyhow::bail!(
                    "cross-chain payment {payment_id} has an unexpected AMM conversion_info \
                     (status {status:?})"
                ),
            };
            let status_str = format!("{status:?}");
            if last_status.as_deref() != Some(status_str.as_str()) {
                info!(
                    "Cross-chain status: {status_str} (delivered_amount={delivered:?}, \
                     elapsed {}s)",
                    start.elapsed().as_secs()
                );
                last_status = Some(status_str);
            }
            match status {
                ConversionStatus::Completed => return Ok(delivered),
                ConversionStatus::Failed
                | ConversionStatus::Refunded
                | ConversionStatus::RefundNeeded => {
                    anyhow::bail!("cross-chain conversion for {payment_id} ended in {status:?}")
                }
                ConversionStatus::Pending => {}
            }
        } else {
            debug!(
                "payment {payment_id} has no conversion_info yet (status {:?})",
                payment.status
            );
        }

        if start.elapsed() >= Duration::from_secs(timeout_secs) {
            anyhow::bail!(
                "timeout after {timeout_secs}s waiting for cross-chain completion on \
                 {payment_id} (last status {last_status:?})"
            );
        }
        // The status update arrives via the background monitor; nudge a sync so
        // the locally-read payment reflects it promptly.
        let _ = sdk.sync_wallet(SyncWalletRequest {}).await;
        tokio::time::sleep(STATUS_POLL_INTERVAL).await;
    }
}
