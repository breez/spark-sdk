//! Mainnet-only test helpers: env-gated credentials, mainnet builders (client
//! and server mode), the deterministic Alice/Bob wallet pair, and the pool
//! quote helpers tests use to size conversion amounts.

use anyhow::Result;
use breez_sdk_spark::*;
use tempfile::TempDir;
use tokio::sync::mpsc;
use tracing::{info, warn};

use super::{ChannelEventListener, wait_for_token_balance_increase};
use crate::SdkInstance;

// ============================================================================
// Mainnet conversion tests
//
// The token-conversion / stable-balance tests need real Flashnet pool liquidity,
// which regtest lacks. They run against mainnet, gated on env credentials, and
// the dedicated `mainnet_teardown` test drains funds back to the test account.
// See the module docs on `tests/mainnet_token_conversion.rs`,
// `tests/mainnet_stable_balance.rs`, and `tests/mainnet_teardown.rs`.
// ============================================================================

/// Fixed BIP-39 passphrase used to derive the deterministic "Bob" wallet from
/// the mainnet test mnemonic. Deterministic derivation means funds left behind by
/// a crashed run are recovered by the next `mainnet_teardown` run.
const MAINNET_BOB_PASSPHRASE: &str = "breez-itest-bob";

/// Mainnet test credentials read from the environment.
///
/// Returns `None` (so the caller skips the test) unless BOTH are set:
/// - `MAINNET_TEST_MNEMONIC` — the pre-funded test account ("Alice").
/// - `BREEZ_API_KEY` — required for the mainnet SDK to function.
///
/// Returns `Some((mnemonic, api_key))` otherwise.
pub fn mainnet_test_creds() -> Option<(String, String)> {
    let mnemonic = std::env::var("MAINNET_TEST_MNEMONIC")
        .ok()
        .filter(|s| !s.trim().is_empty())?;
    let api_key = std::env::var("BREEZ_API_KEY")
        .ok()
        .filter(|s| !s.trim().is_empty())?;
    Some((mnemonic, api_key))
}

/// Token identifier used by the mainnet conversion tests. Defaults to USDB
/// ([`crate::fixtures::USDB_MAINNET_TOKEN_ID`]); override with `MAINNET_TEST_TOKEN_ID`.
pub fn mainnet_test_token_id() -> String {
    std::env::var("MAINNET_TEST_TOKEN_ID")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| crate::fixtures::USDB_MAINNET_TOKEN_ID.to_string())
}

/// Build a mainnet SDK instance from a mnemonic (+ optional passphrase) for the
/// env-gated mainnet conversion tests. Uses SQLite storage in a temp dir.
///
/// Unlike the regtest builders this sets `Network::Mainnet` and a real
/// `api_key`, and keeps the mainnet `lnurl_domain` / real-time sync URL since
/// the conversions are backed by live services.
pub async fn build_mainnet_sdk_from_mnemonic(
    storage_dir: String,
    mnemonic: String,
    passphrase: Option<String>,
    api_key: String,
    temp_dir: Option<TempDir>,
    stable_balance_config: Option<StableBalanceConfig>,
) -> Result<SdkInstance> {
    let mut config = default_config(Network::Mainnet);
    config.api_key = Some(api_key);
    config.prefer_spark_over_lightning = true;
    config.sync_interval_secs = 5;
    config.stable_balance_config = stable_balance_config;

    let seed = Seed::Mnemonic {
        mnemonic,
        passphrase,
    };
    let sdk = SdkBuilder::new(config, seed)
        .with_default_storage(storage_dir)
        .build()
        .await?;

    let (tx, rx) = mpsc::channel(100);
    let event_listener = Box::new(ChannelEventListener { tx });
    let _listener_id = sdk.add_event_listener(event_listener).await;

    // Ensure initial sync completes.
    let _ = sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(true),
        })
        .await?;

    Ok(SdkInstance {
        sdk,
        events: rx,
        span: tracing::Span::current(),
        temp_dir,
        data_sync_fixture: None,
        lnurl_fixture: None,
        turnkey_guard: None,
    })
}

/// Build the two mainnet test wallets.
///
/// - **Alice** = the env mnemonic (the pre-funded test account, the funder).
/// - **Bob** = deterministically derived from Alice's mnemonic via a fixed
///   passphrase (recoverable, swept back to Alice on teardown).
///
/// When `stable_balance` is true, **Bob** (the receiver) is configured with a
/// USDB stable-balance config; `threshold_sats`/`max_slippage_bps` are left
/// `None` so the SDK applies the pool-derived minimum and default slippage.
pub async fn mainnet_alice_bob(
    mnemonic: &str,
    api_key: &str,
    stable_balance: bool,
) -> Result<(SdkInstance, SdkInstance)> {
    let bob_stable = stable_balance.then(|| {
        let token_id = mainnet_test_token_id();
        StableBalanceConfig {
            tokens: vec![StableBalanceToken {
                label: "USDB".to_string(),
                token_identifier: token_id,
            }],
            default_active_label: Some("USDB".to_string()),
            threshold_sats: None,
            max_slippage_bps: None,
        }
    });

    let alice_dir = tempfile::Builder::new()
        .prefix("breez-sdk-mainnet-alice")
        .tempdir()?;
    let alice_path = alice_dir.path().to_string_lossy().to_string();
    info!("Initializing mainnet Alice (test account) at: {alice_path}");
    let alice = build_mainnet_sdk_from_mnemonic(
        alice_path,
        mnemonic.to_string(),
        None,
        api_key.to_string(),
        Some(alice_dir),
        None,
    )
    .await?;

    let bob_dir = tempfile::Builder::new()
        .prefix("breez-sdk-mainnet-bob")
        .tempdir()?;
    let bob_path = bob_dir.path().to_string_lossy().to_string();
    info!("Initializing mainnet Bob (deterministic) at: {bob_path}");
    let bob = build_mainnet_sdk_from_mnemonic(
        bob_path,
        mnemonic.to_string(),
        Some(MAINNET_BOB_PASSPHRASE.to_string()),
        api_key.to_string(),
        Some(bob_dir),
        bob_stable,
    )
    .await?;

    Ok((alice, bob))
}

/// Build a mainnet SDK instance in **server mode** from a mnemonic. Same as
/// [`build_mainnet_sdk_from_mnemonic`] but starts from `default_server_config`:
/// no background sync, no real-time sync URL, no auto-optimization workers.
/// Callers must drive `sync_wallet` explicitly.
///
/// Server mode does not support stable balance (the worker is a background
/// service), so there's no `stable_balance_config` parameter.
pub async fn build_mainnet_sdk_server_mode_from_mnemonic(
    storage_dir: String,
    mnemonic: String,
    passphrase: Option<String>,
    api_key: String,
    temp_dir: Option<TempDir>,
) -> Result<SdkInstance> {
    let mut config = default_server_config(Network::Mainnet);
    config.api_key = Some(api_key);
    config.prefer_spark_over_lightning = true;

    let seed = Seed::Mnemonic {
        mnemonic,
        passphrase,
    };
    let sdk = SdkBuilder::new(config, seed)
        .with_default_storage(storage_dir)
        .build()
        .await?;

    let (tx, rx) = mpsc::channel(100);
    let event_listener = Box::new(ChannelEventListener { tx });
    let _listener_id = sdk.add_event_listener(event_listener).await;

    // Server mode rejects `ensure_synced=true` (no background sync to await);
    // drive the initial sync explicitly so the tree-store hydrate completes
    // before the caller proceeds.
    sdk.sync_wallet(SyncWalletRequest {}).await?;

    Ok(SdkInstance {
        sdk,
        events: rx,
        span: tracing::Span::current(),
        temp_dir,
        data_sync_fixture: None,
        lnurl_fixture: None,
        turnkey_guard: None,
    })
}

/// Server-mode variant of [`mainnet_alice_bob`]. Both wallets are built with
/// `default_server_config(Network::Mainnet)` — no background sync / event
/// delivery / auto-conversion workers. Stable balance is not supported in
/// server mode. Uses the same env mnemonic and deterministic Bob passphrase
/// as the client-mode tests, so the on-chain identity is shared and the
/// teardown drains the same wallet.
pub async fn mainnet_alice_bob_server_mode(
    mnemonic: &str,
    api_key: &str,
) -> Result<(SdkInstance, SdkInstance)> {
    let alice_dir = tempfile::Builder::new()
        .prefix("breez-sdk-mainnet-server-alice")
        .tempdir()?;
    let alice_path = alice_dir.path().to_string_lossy().to_string();
    info!("Initializing mainnet server-mode Alice (test account) at: {alice_path}");
    let alice = build_mainnet_sdk_server_mode_from_mnemonic(
        alice_path,
        mnemonic.to_string(),
        None,
        api_key.to_string(),
        Some(alice_dir),
    )
    .await?;

    let bob_dir = tempfile::Builder::new()
        .prefix("breez-sdk-mainnet-server-bob")
        .tempdir()?;
    let bob_path = bob_dir.path().to_string_lossy().to_string();
    info!("Initializing mainnet server-mode Bob (deterministic) at: {bob_path}");
    let bob = build_mainnet_sdk_server_mode_from_mnemonic(
        bob_path,
        mnemonic.to_string(),
        Some(MAINNET_BOB_PASSPHRASE.to_string()),
        api_key.to_string(),
        Some(bob_dir),
    )
    .await?;

    Ok((alice, bob))
}

/// Minimum token input for a Token→Bitcoin conversion of `token_id` (token base
/// units).
///
/// The pool only reliably exposes input minimums (`min_from_amount`); output
/// minimums (`min_to_amount`) are not returned, so callers size token amounts off
/// this and derive sats amounts via [`estimate_tobtc_sats_out`].
pub async fn tobtc_min_token_input(sdk: &BreezSdk, token_id: &str) -> Result<u128> {
    sdk.fetch_conversion_limits(FetchConversionLimitsRequest {
        conversion_type: ConversionType::ToBitcoin {
            from_token_identifier: token_id.to_string(),
        },
        token_identifier: None,
    })
    .await?
    .min_from_amount
    .ok_or_else(|| anyhow::anyhow!("ToBitcoin min_from_amount missing"))
}

/// Estimate the sats produced by converting `token_amount` of `token_id` to
/// Bitcoin, by preparing (without sending) a Token→BTC payment to `dest`. Used to
/// size sats amounts, since the pool exposes no output minimum.
pub async fn estimate_tobtc_sats_out(
    sdk: &BreezSdk,
    dest: &str,
    token_id: &str,
    token_amount: u128,
) -> Result<u128> {
    let prepared = sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input {
                input: dest.to_string(),
            },
            amount: Some(token_amount),
            token_identifier: Some(token_id.to_string()),
            conversion_options: Some(ConversionOptions {
                conversion_type: ConversionType::ToBitcoin {
                    from_token_identifier: token_id.to_string(),
                },
                max_slippage_bps: None,
                completion_timeout_secs: None,
            }),
            fee_policy: None,
        })
        .await?;
    Ok(prepared
        .conversion_estimate
        .ok_or_else(|| anyhow::anyhow!("conversion estimate missing"))?
        .amount_out)
}

/// Pre-test snapshot of Alice + Bob balances + a one-shot pool rate + token
/// display metadata. Captured by [`mainnet_test_setup`] and consumed by
/// [`log_test_diff`] to produce a single end-of-test cost line with a
/// sat-normalized total drain.
#[derive(Debug, Clone)]
pub struct MainnetTestSnapshot {
    pub alice_sats: u64,
    pub alice_tokens: u128,
    pub bob_sats: u64,
    pub bob_tokens: u128,
    /// Pool rate at snapshot time: how many token base units convert to 1 sat
    /// via the ToBitcoin pool. `None` if the rate query failed (display will
    /// fall back to raw deltas without a sat-normalized total).
    pub tokens_per_sat: Option<u128>,
    /// Display ticker for the test token (e.g. "USDB"), or "?" if metadata is
    /// unavailable (token not in either wallet's `token_balances`).
    pub token_ticker: String,
    /// Display decimals for the test token (e.g. 6 for USDB), or 0 fallback.
    pub token_decimals: u32,
}

/// One-shot pool quote: how many token base units of `token_id` are needed to
/// produce 1 sat via the ToBitcoin pool. Used by [`snapshot_test_pair`] to
/// seed the rate field on the snapshot.
async fn quote_tokens_per_sat(
    alice: &SdkInstance,
    bob: &SdkInstance,
    token_id: &str,
) -> Result<u128> {
    // Probe well above the pool minimum so the quote is robust and precise.
    let probe = tobtc_min_token_input(&alice.sdk, token_id)
        .await?
        .saturating_mul(2);
    let bob_spark = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;
    let sats_out = estimate_tobtc_sats_out(&alice.sdk, &bob_spark, token_id, probe).await?;
    if sats_out == 0 {
        anyhow::bail!("ToBitcoin quote returned 0 sats");
    }
    Ok(probe / sats_out)
}

/// Sync + read Alice and Bob, capturing their sat balances and balance of
/// `token_id`, plus a one-shot pool rate and token display metadata. Used by
/// [`mainnet_test_setup`] (and any test that builds its wallets through a
/// different builder, e.g. server mode) to seed [`log_test_diff`].
pub async fn snapshot_test_pair(
    alice: &SdkInstance,
    bob: &SdkInstance,
    token_id: &str,
) -> Result<MainnetTestSnapshot> {
    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let alice_info = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?;
    let bob_info = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?;
    let token_balance = |info: &GetInfoResponse| -> u128 {
        info.token_balances
            .get(token_id)
            .map(|b| b.balance)
            .unwrap_or(0)
    };
    let metadata = alice_info
        .token_balances
        .get(token_id)
        .map(|tb| tb.token_metadata.clone())
        .or_else(|| {
            bob_info
                .token_balances
                .get(token_id)
                .map(|tb| tb.token_metadata.clone())
        });
    let (token_ticker, token_decimals) = metadata
        .map(|m| (m.ticker, m.decimals))
        .unwrap_or_else(|| ("?".to_string(), 0));
    let tokens_per_sat = quote_tokens_per_sat(alice, bob, token_id).await.ok();
    Ok(MainnetTestSnapshot {
        alice_sats: alice_info.balance_sats,
        alice_tokens: token_balance(&alice_info),
        bob_sats: bob_info.balance_sats,
        bob_tokens: token_balance(&bob_info),
        tokens_per_sat,
        token_ticker,
        token_decimals,
    })
}

/// Standard mainnet-test preamble: gate on env credentials, build Alice and
/// Bob, optionally skip if Alice has no sats, and capture a pre-test balance
/// snapshot for cost tracking. Returns `None` if the test should skip (logged
/// with `warn!`); `Some((alice, bob, token_id, snapshot))` otherwise.
///
/// Pair with [`log_test_diff`] at the end of the test body to emit a single
/// per-test cost line covering both wallets + combined totals.
///
/// - `stable_balance_on_bob` — third arg to [`mainnet_alice_bob`].
/// - `require_alice_funded` — when true, also syncs Alice and returns `None`
///   if her sat balance is zero.
pub async fn mainnet_test_setup(
    stable_balance_on_bob: bool,
    require_alice_funded: bool,
) -> Result<Option<(SdkInstance, SdkInstance, String, MainnetTestSnapshot)>> {
    let Some((mnemonic, api_key)) = mainnet_test_creds() else {
        warn!("Skipping mainnet test: set MAINNET_TEST_MNEMONIC and BREEZ_API_KEY to run it");
        return Ok(None);
    };
    let (alice, bob) = mainnet_alice_bob(&mnemonic, &api_key, stable_balance_on_bob).await?;

    let token_id = mainnet_test_token_id();
    let snapshot = snapshot_test_pair(&alice, &bob, &token_id).await?;

    if require_alice_funded && snapshot.alice_sats == 0 {
        warn!("Skipping mainnet test: test account (Alice) has 0 sats");
        return Ok(None);
    }

    Ok(Some((alice, bob, token_id, snapshot)))
}

/// Format a signed token base-unit amount with the token's `decimals` and
/// `ticker`, e.g. `+0.999342 USDB` or `-2.890307 USDB`. Falls back to raw base
/// units when `decimals == 0`.
fn format_token_amount(amount: i128, decimals: u32, ticker: &str) -> String {
    if decimals == 0 {
        return format!("{amount:+} {ticker}");
    }
    let sign = if amount < 0 { '-' } else { '+' };
    let abs = amount.unsigned_abs();
    let divisor = 10_u128.pow(decimals);
    let int = abs / divisor;
    let frac = abs % divisor;
    let width = decimals as usize;
    format!("{sign}{int}.{frac:0width$} {ticker}")
}

/// Log the per-wallet + combined sat/token deltas a test caused, normalized to
/// a single `net Δ ≈ N sats` headline using the pool rate captured by
/// [`mainnet_test_setup`]. The combined total captures actual drain on the
/// funder (fees + anything stranded in pools); Alice and Bob per-wallet deltas
/// show what was shifted between them.
pub async fn log_test_diff(
    test_name: &str,
    alice: &SdkInstance,
    bob: &SdkInstance,
    token_id: &str,
    pre: &MainnetTestSnapshot,
) -> Result<()> {
    let post = snapshot_test_pair(alice, bob, token_id).await?;
    let alice_sats = i128::from(post.alice_sats) - i128::from(pre.alice_sats);
    #[allow(clippy::cast_possible_wrap)]
    let alice_tokens = post.alice_tokens as i128 - pre.alice_tokens as i128;
    let bob_sats = i128::from(post.bob_sats) - i128::from(pre.bob_sats);
    #[allow(clippy::cast_possible_wrap)]
    let bob_tokens = post.bob_tokens as i128 - pre.bob_tokens as i128;
    let total_sats = alice_sats + bob_sats;
    let total_tokens = alice_tokens + bob_tokens;

    let ticker = pre.token_ticker.as_str();
    let decimals = pre.token_decimals;
    let alice_tok = format_token_amount(alice_tokens, decimals, ticker);
    let bob_tok = format_token_amount(bob_tokens, decimals, ticker);
    let total_tok = format_token_amount(total_tokens, decimals, ticker);

    let (headline, combined_extra, rate_str) = match pre.tokens_per_sat {
        Some(rate) if rate > 0 => {
            #[allow(clippy::cast_possible_wrap)]
            let rate_i = rate as i128;
            let token_sats = total_tokens / rate_i;
            let net = total_sats + token_sats;
            (
                format!("net Δ ≈ {net:+} sats"),
                format!(" (≈ {token_sats:+} sats)"),
                format!("rate ≈ {rate} {ticker}-units/sat"),
            )
        }
        _ => (
            "net Δ N/A".to_string(),
            String::new(),
            "rate unavailable".to_string(),
        ),
    };

    info!(
        "[mainnet-cost] {test_name}: {headline} | \
         alice {alice_sats:+} sats {alice_tok} | \
         bob {bob_sats:+} sats {bob_tok} | \
         combined {total_sats:+} sats {total_tok}{combined_extra} | {rate_str}"
    );
    Ok(())
}

/// Ensures Bob holds at least `min_required` units of `token_id`. If he
/// doesn't, runs a Bitcoin→Token conversion from Alice for `seed_amount` to
/// top him up.
///
/// Returns `Ok(true)` when Bob has enough tokens (already, or freshly seeded).
/// Returns `Ok(false)` when seeding was required but Alice's sat balance can't
/// cover the conversion — callers should skip the test.
pub async fn ensure_bob_has_tokens(
    alice: &SdkInstance,
    bob: &SdkInstance,
    token_id: &str,
    min_required: u128,
    seed_amount: u128,
) -> Result<bool> {
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let bob_tokens_before = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .token_balances
        .get(token_id)
        .map(|b| b.balance)
        .unwrap_or(0);
    info!("Bob token balance: {bob_tokens_before}");

    if bob_tokens_before >= min_required {
        return Ok(true);
    }

    info!("Seeding Bob with {seed_amount} units of {token_id} via Bitcoin→Token");
    let bob_spark = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;
    let seed_prepare = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input { input: bob_spark },
            amount: Some(seed_amount),
            token_identifier: Some(token_id.to_string()),
            conversion_options: Some(ConversionOptions {
                conversion_type: ConversionType::FromBitcoin,
                max_slippage_bps: None,
                completion_timeout_secs: None,
            }),
            fee_policy: None,
        })
        .await?;
    let estimate = seed_prepare
        .conversion_estimate
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("seed conversion estimate missing"))?;
    let seed_cost = estimate.amount_in.saturating_add(estimate.fee);

    let alice_balance = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;
    if u128::from(alice_balance) < seed_cost {
        warn!("Cannot seed Bob: Alice balance {alice_balance} sats < seed cost ~{seed_cost} sats");
        return Ok(false);
    }

    alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: seed_prepare,
            options: None,
            idempotency_key: None,
        })
        .await?;
    wait_for_token_balance_increase(&bob.sdk, token_id, bob_tokens_before, 120).await?;
    Ok(true)
}
