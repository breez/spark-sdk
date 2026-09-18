#![cfg(feature = "local-itest")]

use std::str::FromStr;
use std::sync::Arc;

use anyhow::Result;
use bitcoin::{Address, Amount};
use breez_sdk_itest::{SignerBackend, build_local_sdk_with_config, wait_for_balance};
use breez_sdk_spark::{
    ClaimDepositRequest, GetInfoRequest, InstantClaimStatus, ListUnclaimedDepositsRequest, MaxFee,
    ReceivePaymentMethod, ReceivePaymentRequest,
};
use rstest::*;
use spark_itest::fixtures::setup::TestFixtures;
use tokio::time::{Duration, Instant, sleep};
use tracing::{info, warn};

/// Errors from the SSP or operators not having seen the mempool tx yet. Any other
/// error fails the test rather than being retried into a timeout.
fn is_transient_claim_error(message: &str) -> bool {
    let message = message.to_lowercase();
    [
        "transaction not found",
        "not indexed",
        "confirmation",
        "enough confirmations",
        "operators have not seen",
        "utxo not found",
        "not yet",
        "unknown",
    ]
    .iter()
    .any(|marker| message.contains(marker))
}

/// `gettxout`, mempool included, answers null for a spent output.
async fn deposit_outpoint_unspent(fixtures: &TestFixtures, txid: bitcoin::Txid, vout: u32) -> bool {
    let out: serde_json::Value = fixtures
        .bitcoind
        .rpc(
            "gettxout",
            &[
                serde_json::json!(txid.to_string()),
                serde_json::json!(vout),
                serde_json::json!(true),
            ],
        )
        .await
        .unwrap_or(serde_json::Value::Null);
    !out.is_null()
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_instant_static_deposit_claim_local() -> Result<()> {
    let fixtures = Arc::new(TestFixtures::new().await?);

    // A zero fee ceiling stops background claims, so only the explicit claim below,
    // with its own ceiling, claims the deposit.
    let local =
        build_local_sdk_with_config(Arc::clone(&fixtures), SignerBackend::Seed, None, |cfg| {
            cfg.max_deposit_claim_fee = Some(MaxFee::Fixed { amount: 0 });
        })
        .await?;

    let start_balance = local
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;
    info!("Start balance: {start_balance} sats");

    let fund_amount = 50_000u64;

    let addr = local
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::BitcoinAddress { new_address: None },
        })
        .await?
        .payment_request;
    info!("Static deposit address: {addr}");
    let parsed_addr = Address::from_str(&addr)?.assume_checked();

    // `sendtoaddress` does not mine, and an immature deposit is what makes
    // `claim_deposit` take the instant path. Its tx also has a change output, so the
    // vout is looked up.
    let txid = fixtures
        .bitcoind
        .fund_address(&parsed_addr, Amount::from_sat(fund_amount))
        .await?;
    info!("Funded static deposit at 0-conf, txid: {txid}");
    let tx = fixtures.bitcoind.get_transaction(&txid).await?;
    let vout = tx
        .output
        .iter()
        .enumerate()
        .find(|(_, o)| {
            bitcoin::Address::from_script(&o.script_pubkey, bitcoin::Network::Regtest)
                .is_ok_and(|a| a == parsed_addr)
        })
        .map(|(i, _)| i as u32)
        .expect("funding tx has no output paying the deposit address");
    info!("Deposit outpoint: {txid}:{vout}");

    let deadline = Instant::now() + Duration::from_secs(90);
    let claim_resp = loop {
        match local
            .sdk
            .claim_deposit(ClaimDepositRequest {
                txid: txid.to_string(),
                vout,
                // Headroom over the SSP's instant claim fee.
                max_fee: Some(MaxFee::Fixed { amount: 3_000 }),
            })
            .await
        {
            Ok(resp) => break resp,
            Err(e) if is_transient_claim_error(&e.to_string()) => {
                if Instant::now() >= deadline {
                    anyhow::bail!("instant claim never became claimable within 90s: {e}");
                }
                info!("instant claim not ready yet, retrying: {e}");
                sleep(Duration::from_secs(2)).await;
            }
            Err(e) => return Err(anyhow::anyhow!("instant claim failed (terminal): {e}")),
        }
    };

    // Only a matured deposit's claim returns a payment; an instant claim credits on
    // a later sync.
    assert!(
        claim_resp.payment.is_none(),
        "instant claim must not settle synchronously (got a payment: {:?})",
        claim_resp.payment
    );

    // `claim_deposit` inserts and marks the row itself, so it is listed as Submitted
    // right away, before the credit settles.
    let deposits = local
        .sdk
        .list_unclaimed_deposits(ListUnclaimedDepositsRequest {})
        .await?
        .deposits;
    let dep = deposits
        .iter()
        .find(|d| d.txid == txid.to_string())
        .expect("the instant-claimed deposit must be listed (created and marked)");
    assert!(
        matches!(
            dep.instant_claim_status,
            Some(InstantClaimStatus::Submitted { .. })
        ),
        "instant-claimed deposit must be marked Submitted: {:?}",
        dep.instant_claim_status
    );
    info!("Deposit marked Submitted; waiting for the async credit to settle");

    // No block has been mined since funding, so a rise is a 0-conf credit.
    let balance = wait_for_balance(&local.sdk, Some(start_balance + 1), None, 180).await?;
    info!("Instant credit settled at 0-conf: {balance} sats (was {start_balance})");
    assert!(balance > start_balance, "balance must rise at 0-conf");
    assert!(
        balance <= start_balance + fund_amount,
        "credit ({}) exceeds the funded amount ({fund_amount})",
        balance - start_balance
    );
    warn!(
        "INSTANT 0-conf claim proven: balance {start_balance} -> {balance} (+{}), deposit still at 0 confirmations",
        balance - start_balance
    );

    // The SSP collects the deposit UTXO only once it confirms, and nothing else
    // mines here.
    assert!(
        deposit_outpoint_unspent(&fixtures, txid, vout).await,
        "deposit UTXO must be unspent before the deferred CLAIM (instant credit came from the SSP pool, not the UTXO)"
    );

    fixtures.bitcoind.generate_blocks(1).await?;
    info!("Mined 1 block to confirm the deposit; waiting on the deferred CLAIM worker");

    let deadline = Instant::now() + Duration::from_secs(120);
    let spend_txid = loop {
        fixtures.bitcoind.generate_blocks(1).await?;
        if let Some(spend_txid) = local
            .static_deposit_spend_txid(&txid.to_string(), vout)
            .await?
        {
            break spend_txid;
        }
        if Instant::now() >= deadline {
            anyhow::bail!(
                "deferred CLAIM did not co-sign + broadcast the deposit-spend within 120s (operator may not have indexed the confirmation yet)"
            );
        }
        info!("deferred CLAIM not complete yet, retrying");
        sleep(Duration::from_secs(3)).await;
    };
    info!("Deferred CLAIM co-signed + broadcast the deposit-spend: {spend_txid}");

    assert!(
        !deposit_outpoint_unspent(&fixtures, txid, vout).await,
        "the deposit UTXO must be spent by the broadcast deposit-spend {spend_txid}"
    );
    warn!(
        "Deferred worker CLAIM proven: deposit-spend {spend_txid} broadcast, deposit UTXO collected by the SSP"
    );

    Ok(())
}
