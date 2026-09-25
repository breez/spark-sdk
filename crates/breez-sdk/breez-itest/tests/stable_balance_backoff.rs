//! A stable balance conversion that cannot succeed must back off rather than
//! retry on every sync.
//!
//! The failure is forced with a token no pool trades, funded past the
//! conversion threshold so the attempt is made. The sats stay in the wallet.

use anyhow::Result;
use breez_sdk_itest::helpers::count_events_during;
use breez_sdk_itest::helpers::regtest::{build_sdk_with_custom_config, ensure_funded};
use std::sync::Mutex;

use breez_sdk_spark::{
    Config, Network, SdkEvent, StableBalanceConfig, StableBalanceConversionKind,
    StableBalanceToken, default_config,
};
use rand::RngCore;
use tracing::info;

/// Balance the wallet needs before auto-convert will attempt anything.
const THRESHOLD_SATS: u64 = 1000;

/// The first delay after a failure.
const BASE_DELAY_SECS: u64 = 30;

/// How long to watch. A failed per-receive holds back the batch sweep until it
/// expires at 120s, so this covers the sweep's retry and the one after it.
const WINDOW_SECS: u64 = 200;

/// The most attempts the schedule allows in `WINDOW_SECS`: the per-receive,
/// the sweep once it expires, and one retry after a 60s delay, plus one spare.
/// A loop would produce dozens.
const MAX_ATTEMPTS: usize = 4;

/// Well-formed but traded by no pool, so selecting a pool for it fails.
const UNTRADED_TOKEN_ID: &str = "btknrt14w46h2at4w46h2at4w46h2at4w46h2at4w46h2at4w46h2at4w4sh59f4v";

fn config_with_untradeable_stable_balance() -> Config {
    let mut cfg = default_config(Network::Regtest);
    cfg.stable_balance_config = Some(StableBalanceConfig {
        tokens: vec![StableBalanceToken {
            label: "UNTRADED".to_string(),
            token_identifier: UNTRADED_TOKEN_ID.to_string(),
        }],
        default_active_label: Some("UNTRADED".to_string()),
        threshold_sats: Some(THRESHOLD_SATS),
        max_slippage_bps: Some(500),
    });
    cfg
}

#[test_log::test(tokio::test)]
async fn a_conversion_that_cannot_succeed_backs_off_instead_of_retrying() -> Result<()> {
    let dir = tempfile::Builder::new()
        .prefix("breez-sdk-stable-balance-backoff")
        .tempdir()?;
    let path = dir.path().to_string_lossy().to_string();
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);

    let mut alice = build_sdk_with_custom_config(
        path,
        seed,
        config_with_untradeable_stable_balance(),
        Some(dir),
        true,
    )
    .await?;

    // Auto-convert only attempts a conversion once the balance clears the
    // threshold; below it the task is skipped, not failed. Funding also drains
    // the event channel, so attempts are counted only after it returns.
    ensure_funded(&mut alice, THRESHOLD_SATS.saturating_mul(4)).await?;

    // Any conversion kind counts: the funding receive can fail per-receive
    // before the batch sweep gets its turn, and both are gated by one delay.
    let failures = Mutex::new(Vec::new());
    count_events_during(&mut alice.events, WINDOW_SECS, |event| {
        let SdkEvent::StableBalanceConversionFailed {
            conversion,
            retry_in_secs,
            ..
        } = event
        else {
            return false;
        };
        failures.lock().unwrap().push((*conversion, *retry_in_secs));
        true
    })
    .await;
    let failures = failures.into_inner().unwrap();
    info!("conversion failures in {WINDOW_SECS}s: {failures:?}");

    assert!(
        failures.len() <= MAX_ATTEMPTS,
        "expected at most {MAX_ATTEMPTS} attempts in {WINDOW_SECS}s, got {failures:?}"
    );
    // A received payment's own conversion is not retried, so it reports no delay.
    assert!(
        failures
            .iter()
            .filter(|(kind, _)| *kind == StableBalanceConversionKind::PerReceive)
            .all(|(_, delay)| delay.is_none()),
        "a per-receive failure must not promise a retry, got {failures:?}"
    );
    // The sweep was retried once a delay ran out, and reported a longer one:
    // the failures accumulate rather than each starting from the base.
    let last_delay = failures
        .iter()
        .rfind(|(kind, _)| *kind == StableBalanceConversionKind::AutoConvert)
        .and_then(|(_, delay)| *delay)
        .expect("the sweep was never retried, so the delay was not exercised");
    assert!(
        last_delay > BASE_DELAY_SECS,
        "expected the delay to grow past {BASE_DELAY_SECS}s, got {failures:?}"
    );

    alice.sdk.disconnect().await?;
    Ok(())
}
