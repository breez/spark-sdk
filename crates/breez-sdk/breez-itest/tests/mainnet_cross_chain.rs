//! Mainnet cross-chain itests (env-gated).
//!
//! Exercises both directions against the funded test account ("Alice") on Spark
//! and a **deterministic EVM wallet derived from the same test mnemonic** at
//! `m/44'/60'/0'/0/0`. All tests target **Arbitrum One**, the only chain
//! offering USD-stable assets on both providers.
//!
//! ## Send tests (Alice → EVM)
//!
//! - `test_cross_chain_01_boltz_send_fees_excluded_evm`: BTC (sats) → USDT via Boltz.
//! - `test_cross_chain_02_orchestra_send_fees_excluded_evm`: USDB → USDC via Orchestra.
//!
//! Verifies receipt independently by reading the EVM recipient's ERC-20 balance
//! over JSON-RPC. Both exercise the fees-excluded / target-overpay guarantee.
//!
//! ## Receive tests (EVM → Alice)
//!
//! Each receive test is paired with a send test so the pool an itest pass
//! drains gets refilled by its counterpart, keeping Alice's Spark-side
//! balances roughly conservative across runs:
//!
//! - `test_cross_chain_03_orchestra_receive_fees_excluded_evm`: **USDT → BTC
//!   (sats)**. Alice targets a small sats amount; SDK computes the required
//!   USDT deposit. Counters test 01 so the sats Alice spent to build the USDT
//!   stash come back here. Also exercises the receive branch where the
//!   external-fee buffer is NOT applied (BTC destination).
//! - `test_cross_chain_04_orchestra_receive_fees_included_evm`: **USDC →
//!   USDB**. Sweep the EVM wallet's entire USDC balance back into Alice's
//!   Spark wallet as USDB. Counters test 02 so the USDB Alice spent to build
//!   the USDC stash comes back here.
//!
//! Receive tests need Arbitrum ETH at the deterministic EVM address to pay gas
//! for the ERC-20 transfer; the tests read the balance and skip loudly (never
//! hard-fail) if it's below the threshold. Roadmap: when a second cross-chain
//! receive provider ships, one of these should switch to that provider so both
//! providers get exercised in a single run.
//!
//! ## Test ordering
//!
//! Tests are name-prefixed `01_`…`04_` and run in that order under
//! `--test-threads=1` (libtest sorts tests alphabetically). Ordering matters:
//! each send test deposits its destination asset at the EVM recipient, and
//! the two receive tests then consume those balances (test 03 uses the USDT
//! from Boltz's send; test 04 sweeps the USDC from Orchestra's send).
//!
//! NOTE: the cross-chain `amount` semantics depend on both the fee mode and
//! the test direction. See [`ReceivePaymentMethod::CrossChain::amount`] and
//! [`PaymentRequest::CrossChain::amount`] for the full contract.
//!
//! # Required environment variables
//! - `MAINNET_TEST_MNEMONIC`: mnemonic of the funded test account. Primary gate.
//! - `BREEZ_API_KEY`: API key the mainnet SDK requires.
//!
//! # Optional
//! - `MAINNET_TEST_TOKEN_ID`: USD-stable source token; defaults to USDB.
//! - `MAINNET_TEST_CROSS_CHAIN_SEND_USDB`: Orchestra send source in USDB base units
//!   (6-decimals); defaults to 1_500_000 (1.50 USDB, ~$1.50 USDC delivered).
//!   Sized to leave enough USDC above the sweep minimum for test 04.
//! - `MAINNET_TEST_CROSS_CHAIN_SEND_SATS`: Boltz send source in **sats** (BTC
//!   source); defaults to 1_500 (~$1.50 USDT delivered). Sized to leave a bit
//!   above what the fees-excluded receive test consumes each pass.
//! - `MAINNET_TEST_CROSS_CHAIN_RECEIVE_USDB_TARGET`: fees-excluded receive
//!   target in USDB base units (6dp). Applies to both receive tests.
//!   Defaults to `1_000_000` ($1). For test 03 (BTC destination) the SDK
//!   converts USD to sats at prepare time via the live BTC/USD rate.
//! - `MAINNET_TEST_CROSS_CHAIN_RECEIVE_USDC_MAX`: cap for the fees-included
//!   USDC sweep, in USDC base units. Unset by default (sweep the full
//!   balance).
//!
//! # Precondition for the receive tests
//! The EVM address at `m/44'/60'/0'/0/0` needs a small Arbitrum ETH balance for
//! gas (~0.00005 ETH is plenty for a single ERC-20 transfer). Top up manually
//! when needed; tests skip when it's short.
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

// SEND-side note: `amount` on `PaymentRequest::CrossChain` is denominated in
// the **source** asset's units. Orchestra send uses USDB base units; Boltz
// send (BTC source) uses **sats**. Sized and asserted separately.

/// Orchestra source budget in USDB base units (6-decimals), so 1_500_000 = 1.50
/// USDB. USDB↔USDC are ~1:1, so this also approximates the USDC the recipient
/// lands. Override via `MAINNET_TEST_CROSS_CHAIN_SEND_USDB`. A too-small value (below
/// the route minimum) makes `prepare` error (loudly).
///
/// Sized to leave a small amount of USDC headroom at the deterministic
/// recipient above Orchestra's $1 sweep minimum, so the fees-included USDC
/// sweep test (test 04) can consistently pick up something to sweep.
const DEFAULT_ORCHESTRA_SEND_USDB: u128 = 1_500_000;

/// Boltz source budget in **sats** (BTC source). Override via
/// `MAINNET_TEST_CROSS_CHAIN_SEND_SATS`. Note this is sats, NOT a USD amount: the
/// recipient lands the fiat-equivalent in USDT. A value below the route minimum
/// makes `prepare` error (loudly).
///
/// Sized to leave a small amount of USDT headroom at the deterministic
/// recipient beyond what the fees-excluded receive test (test 03) consumes
/// per pass (~$1.10 deposit for a $1 target).
const DEFAULT_BOLTZ_SEND_SATS: u128 = 1_500;

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

/// Minimum Arbitrum ETH balance (in wei) the deterministic recipient needs to
/// broadcast an ERC-20 transfer. Set at 0.00005 ETH: an ERC-20 transfer on
/// Arbitrum costs a fraction of that at typical fees, so this is a generous
/// skip gate (not a tuning knob).
const MIN_ARBITRUM_ETH_WEI: u128 = 50_000_000_000_000;

/// Default receive target in USDB base units (6-decimal, `1_000_000 = $1`).
/// The SDK converts to sats internally at prepare time when the destination
/// is BTC, so callers never touch destination decimals.
const DEFAULT_RECEIVE_USDB_TARGET: u128 = 1_000_000;

/// Minimum stable-source balance (base units, 6-decimal) needed at the
/// deterministic recipient to run the fees-excluded receive test. Sized to
/// cover the $1 USDB target plus Orchestra fees + external overhead.
const MIN_STABLE_FEES_EXCLUDED: u128 = 1_100_000;

/// Minimum stable-source balance (base units, 6-decimal) needed to run the
/// fees-included sweep test. Set at Orchestra's route minimum of $1.
const MIN_STABLE_SWEEP: u128 = 1_000_000;

/// Timeout for `eth_getTransactionReceipt` polling. An Arbitrum transaction
/// lands in a few seconds under normal conditions; give it 5 minutes so a
/// public-endpoint latency spike doesn't fail the test.
const TX_CONFIRM_TIMEOUT_SECS: u64 = 300;

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
async fn test_cross_chain_02_orchestra_send_fees_excluded_evm() -> Result<()> {
    let Some((mut alice, token_id, mnemonic)) = mainnet_cross_chain_setup().await? else {
        return Ok(());
    };
    info!("=== Starting test_cross_chain_02_orchestra_send_fees_excluded_evm ===");
    let pre = alice_balances(&alice, &token_id).await?;

    // Source budget in USDB base units (USD-stable, ~1:1 with the USDC delivered).
    let usdb_amount = env_amount_or(
        "MAINNET_TEST_CROSS_CHAIN_SEND_USDB",
        DEFAULT_ORCHESTRA_SEND_USDB,
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
async fn test_cross_chain_01_boltz_send_fees_excluded_evm() -> Result<()> {
    let Some((mut alice, token_id, mnemonic)) = mainnet_cross_chain_setup().await? else {
        return Ok(());
    };
    info!("=== Starting test_cross_chain_01_boltz_send_fees_excluded_evm ===");
    let pre = alice_balances(&alice, &token_id).await?;

    // Source budget in SATS (BTC source). The recipient lands the fiat-equivalent
    // in USDT, so there's no parity unit to assert against the sats amount.
    let sats_amount = env_amount_or(
        "MAINNET_TEST_CROSS_CHAIN_SEND_SATS",
        DEFAULT_BOLTZ_SEND_SATS,
    );
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

/// Orchestra: USDT on Arbitrum → BTC (sats) on Spark, fees-excluded. Alice
/// specifies a target in USDB base units (6dp); the SDK converts to sats
/// internally via the live BTC/USD rate at prepare time, then sizes the USDT
/// deposit. The deterministic EVM wallet signs & broadcasts it.
///
/// Landing as BTC (not USDB) is the counterpart to test 01 (BTC → USDT via
/// Boltz): the send drains Alice's sats to fund the EVM wallet's USDT; this
/// receive returns those sats. Also exercises the receive branch where the
/// external-fee buffer is intentionally NOT applied (`apply_stable_buffers`
/// is `false` for non-stable destinations).
#[test_log::test(tokio::test)]
async fn test_cross_chain_03_orchestra_receive_fees_excluded_evm() -> Result<()> {
    let Some((mut alice, usdb_token_id, mnemonic)) = mainnet_cross_chain_setup().await? else {
        return Ok(());
    };
    info!("=== Starting test_cross_chain_03_orchestra_receive_fees_excluded_evm ===");
    let pre = alice_balances(&alice, &usdb_token_id).await?;

    let target_usdb = env_amount_or(
        "MAINNET_TEST_CROSS_CHAIN_RECEIVE_USDB_TARGET",
        DEFAULT_RECEIVE_USDB_TARGET,
    );

    run_cross_chain_evm_receive(
        &mut alice,
        &mnemonic,
        &usdb_token_id,
        "USDT",
        SparkAsset::Bitcoin,
        ReceiveTestPlan::FeesExcluded {
            target: target_usdb,
            min_source: MIN_STABLE_FEES_EXCLUDED,
        },
    )
    .await?;

    log_cost(
        "orchestra-receive-excluded",
        &usdb_token_id,
        pre,
        alice_balances(&alice, &usdb_token_id).await?,
    );
    Ok(())
}

/// Orchestra: sweep the deterministic EVM wallet's entire USDC (Arbitrum)
/// balance back into Alice's Spark wallet as USDB, fees-included. USDC is
/// the counterpart to the Orchestra USDB → USDC send test, so sweeping it
/// back closes the loop and drains the dust each pass. Alice lands whatever's
/// left after Orchestra's fees + external overhead.
#[test_log::test(tokio::test)]
async fn test_cross_chain_04_orchestra_receive_fees_included_evm() -> Result<()> {
    let Some((mut alice, usdb_token_id, mnemonic)) = mainnet_cross_chain_setup().await? else {
        return Ok(());
    };
    info!("=== Starting test_cross_chain_04_orchestra_receive_fees_included_evm ===");
    let pre = alice_balances(&alice, &usdb_token_id).await?;

    let cap = std::env::var("MAINNET_TEST_CROSS_CHAIN_RECEIVE_USDC_MAX")
        .ok()
        .and_then(|s| s.trim().parse::<u128>().ok())
        .filter(|v| *v > 0);

    run_cross_chain_evm_receive(
        &mut alice,
        &mnemonic,
        &usdb_token_id,
        "USDC",
        SparkAsset::Token {
            token_identifier: usdb_token_id.clone(),
        },
        ReceiveTestPlan::FeesIncludedSweep {
            min_source: MIN_STABLE_SWEEP,
            cap,
        },
    )
    .await?;

    log_cost(
        "orchestra-receive-included",
        &usdb_token_id,
        pre,
        alice_balances(&alice, &usdb_token_id).await?,
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

/// How a receive test decides on the source amount and the min-out assertion.
enum ReceiveTestPlan {
    /// Alice targets a specific delivery amount, always in USDB base units
    /// (6dp, `1_000_000 = $1`) regardless of the destination asset. The SDK
    /// translates internally to destination-native units (sats for BTC
    /// destinations via the live BTC/USD rate). EVM wallet needs at least
    /// `min_source` in the source token + gas ETH.
    FeesExcluded { target: u128, min_source: u128 },
    /// Sweep the EVM wallet's source-token balance (optionally capped by
    /// `cap`). Amount passed to the SDK is the source balance numerically:
    /// safe because Arbitrum USDC is 6dp, same as USDB.
    FeesIncludedSweep { min_source: u128, cap: Option<u128> },
}

/// Shared flow for both receive tests: discover the Arbitrum source-asset
/// receive route (`source_asset` is the external-chain ticker, e.g. `"USDT"`
/// or `"USDC"`), gate on ETH-for-gas and source-token balance, call
/// `receive_payment`, sign + broadcast the source-token deposit from the EVM
/// wallet, wait for both the tx to confirm and Alice's Spark-side balance
/// (BTC sats or USDB, per `destination`) to increase.
///
/// Skips (warn + `Ok`) when no matching route, the EVM wallet lacks ETH for
/// gas, or its source-token balance is below the plan's minimum.
async fn run_cross_chain_evm_receive(
    alice: &mut SdkInstance,
    mnemonic: &str,
    usdb_token_id: &str,
    source_asset: &str,
    destination: SparkAsset,
    plan: ReceiveTestPlan,
) -> Result<()> {
    let (recipient, signer) = mainnet_evm_recipient(mnemonic)?;
    let dest_label = spark_asset_label(&destination, usdb_token_id);
    info!(
        "Cross-chain receive: Arbitrum {source_asset} → {dest_label}. Sender EVM wallet \
         {recipient} (m/44'/60'/0'/0/0)"
    );

    // 1. Discover the Orchestra {source_asset}-on-Arbitrum receive route.
    let routes = alice
        .sdk
        .get_cross_chain_routes(&CrossChainRouteFilter::Receive {
            contract_address: None,
        })
        .await?;
    let Some(route) = routes.into_iter().find(|r| {
        r.provider == CrossChainProvider::Orchestra
            && is_target_chain(r)
            && r.asset.eq_ignore_ascii_case(source_asset)
    }) else {
        warn!("No Orchestra {source_asset} receive route on Arbitrum One; skipping");
        return Ok(());
    };
    let source_contract = route
        .contract_address
        .clone()
        .ok_or_else(|| anyhow::anyhow!("{source_asset} receive route has no contract address"))?;
    let Some(rpc_url) = evm_rpc_url(&route.chain) else {
        warn!("No JSON-RPC endpoint for chain {}; skipping", route.chain);
        return Ok(());
    };
    info!(
        "Selected receive route: {source_asset} on {} ({source_contract})",
        route.chain
    );

    // Confirm the route lands the requested destination.
    if !route.spark_assets.contains(&destination) {
        warn!(
            "Route {source_asset}→Arbitrum does not offer {dest_label} destination \
             (spark_assets={:?}); skipping",
            route.spark_assets
        );
        return Ok(());
    }

    // 2. Gas-for-broadcast check.
    let eth_balance = evm_native_balance(&rpc_url, &recipient).await?;
    if eth_balance < MIN_ARBITRUM_ETH_WEI {
        warn!(
            "Recipient {recipient} has {eth_balance} wei on Arbitrum \
             (< {MIN_ARBITRUM_ETH_WEI} needed to pay gas); skipping"
        );
        return Ok(());
    }
    info!("Recipient ETH balance: {eth_balance} wei");

    // 3. Source-balance check + plan resolution.
    let source_balance = evm_erc20_balance(&rpc_url, &source_contract, &recipient).await?;
    info!("Recipient {source_asset} balance: {source_balance}");
    let (fee_mode, amount, parity_min_out): (CrossChainFeeMode, u128, Option<u128>) = match &plan {
        ReceiveTestPlan::FeesExcluded { target, min_source } => {
            if source_balance < *min_source {
                warn!(
                    "Recipient {source_asset} balance {source_balance} below fees-excluded \
                     minimum {min_source}; skipping"
                );
                return Ok(());
            }
            // Parity assertion needs target and delivered in matching units.
            // USDB destinations do; BTC destinations land sats and would need
            // a BTC/USD conversion (skipped for the signal it adds).
            let parity = match &destination {
                SparkAsset::Token { .. } => Some(*target),
                SparkAsset::Bitcoin => None,
            };
            (CrossChainFeeMode::FeesExcluded, *target, parity)
        }
        ReceiveTestPlan::FeesIncludedSweep { min_source, cap } => {
            if source_balance < *min_source {
                warn!(
                    "Recipient {source_asset} balance {source_balance} below sweep minimum \
                     {min_source}; skipping"
                );
                return Ok(());
            }
            let sweep = cap.map(|c| c.min(source_balance)).unwrap_or(source_balance);
            info!("Sweep amount: {sweep} (from balance {source_balance}, cap {cap:?})");
            (CrossChainFeeMode::FeesIncluded, sweep, None)
        }
    };

    // 4. Baseline Alice's destination-side balance (sats for BTC, token base
    //    units for USDB).
    let (baseline_sats, baseline_usdb) = alice_balances(alice, usdb_token_id).await?;
    let baseline: u128 = match &destination {
        SparkAsset::Bitcoin => u128::from(baseline_sats),
        SparkAsset::Token { .. } => baseline_usdb,
    };
    info!("Alice baseline {dest_label}: {baseline}");

    // 5. Ask the SDK to prepare the cross-chain receive.
    let response = alice
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::CrossChain {
                route: route.clone(),
                amount,
                destination: Some(destination.clone()),
                fee_mode: Some(fee_mode),
                max_slippage_bps: None,
                target_overpay_bps: None,
            },
        })
        .await?;
    let info = response
        .cross_chain_info
        .ok_or_else(|| anyhow::anyhow!("receive_payment returned no cross_chain_info"))?;
    info!(
        "Receive prepared: deposit_address={} deposit_amount={} expected_received={} \
         expires_at={}",
        info.deposit_address, info.deposit_amount, info.expected_received_amount, info.expires_at
    );

    // 6. Confirm the EVM wallet holds enough source-token for the SDK-sized
    //    deposit. Only meaningful for fees-excluded (sweep case has
    //    `deposit_amount == source_balance` by construction).
    if source_balance < info.deposit_amount {
        warn!(
            "Recipient {source_asset} balance {source_balance} < SDK-computed deposit {} \
             (fees + overpay pushed it over the balance); skipping",
            info.deposit_amount
        );
        return Ok(());
    }

    // 7. Broadcast the ERC-20 transfer.
    let tx_hash = evm_send_erc20(
        &rpc_url,
        &signer,
        &source_contract,
        &info.deposit_address,
        info.deposit_amount,
    )
    .await?;
    wait_for_evm_tx_confirmation(&rpc_url, &tx_hash, TX_CONFIRM_TIMEOUT_SECS).await?;
    info!("{source_asset} deposit confirmed on Arbitrum: {tx_hash}");

    // 8. Wait for the delivered amount to land on Alice's Spark side.
    //    Orchestra's background monitor reconciles once bridging completes.
    let final_balance: u128 = match &destination {
        SparkAsset::Bitcoin => u128::from(
            wait_for_balance(
                &alice.sdk,
                Some(
                    u64::try_from(baseline)
                        .unwrap_or(u64::MAX)
                        .saturating_add(1),
                ),
                None,
                SETTLE_TIMEOUT_SECS,
            )
            .await?,
        ),
        SparkAsset::Token { .. } => {
            wait_for_token_balance_increase(
                &alice.sdk,
                usdb_token_id,
                baseline,
                SETTLE_TIMEOUT_SECS,
            )
            .await?
        }
    };
    let delivered = final_balance.saturating_sub(baseline);
    info!(
        "Alice {dest_label} delivered: {delivered} (baseline {baseline} → final {final_balance})"
    );

    // Cost decomposition:
    //   `expected_received - delivered`: how much reality fell BELOW the SDK's
    //     shaved prepare-time estimate. Meaningful across both destination
    //     kinds.
    //   `total_spread = deposit - delivered`: single-number cost, only
    //     meaningful for stable-to-stable routes where deposit and delivery
    //     share units. Omitted for BTC destinations (source is 6dp USD-stable,
    //     delivery is sats).
    let expected_vs_delivered = i128::try_from(delivered).unwrap_or(i128::MAX)
        - i128::try_from(info.expected_received_amount).unwrap_or(i128::MAX);
    match &destination {
        SparkAsset::Token { .. } => {
            let total_spread = info.deposit_amount.saturating_sub(delivered);
            info!(
                "[cross-chain-breakdown] {source_asset} → USDB: deposit={} expected={} \
                 delivered={} total_spread={total_spread} \
                 expected_vs_delivered={expected_vs_delivered}",
                info.deposit_amount, info.expected_received_amount, delivered
            );
        }
        SparkAsset::Bitcoin => {
            info!(
                "[cross-chain-breakdown] {source_asset} → BTC: deposit={} (source units) \
                 expected={} sats delivered={} sats expected_vs_delivered={expected_vs_delivered}",
                info.deposit_amount, info.expected_received_amount, delivered
            );
        }
    }

    // 10. Assert the fees-excluded contract when a parity target is set.
    if let Some(target) = parity_min_out {
        let floor = reduce_by_bps(target, SLIPPAGE_TOLERANCE_BPS);
        assert!(
            delivered >= floor,
            "fees-excluded receive: delivered {delivered} should be >= {floor} \
             (target {target} less {SLIPPAGE_TOLERANCE_BPS} bps)"
        );
    } else {
        assert!(
            delivered > 0,
            "sweep receive: expected some {dest_label} delivered, got {delivered}"
        );
        let deposit_pct = delivered.saturating_mul(10_000) / amount.max(1);
        info!(
            "Sweep receive: {delivered} {dest_label} delivered for {amount} {source_asset} \
             deposit ({deposit_pct} bps of source, i.e. {}%)",
            deposit_pct / 100
        );
    }

    log_evm_recovery_balance(&rpc_url, &source_contract, &recipient, source_asset).await;
    Ok(())
}

/// Short label for a receive destination, used in log lines. Distinguishes
/// BTC (sats) from USDB by matching the token identifier against Alice's
/// active USDB token id; any other token identifier renders as
/// `Token(...prefix)` for debuggability.
fn spark_asset_label(destination: &SparkAsset, usdb_token_id: &str) -> String {
    match destination {
        SparkAsset::Bitcoin => "BTC".to_string(),
        SparkAsset::Token { token_identifier } if token_identifier == usdb_token_id => {
            "USDB".to_string()
        }
        SparkAsset::Token { token_identifier } => {
            format!(
                "Token({}...)",
                &token_identifier[..token_identifier.len().min(12)]
            )
        }
    }
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
