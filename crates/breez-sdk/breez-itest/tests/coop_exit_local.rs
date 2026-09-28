#![cfg(feature = "local-itest")]

use std::collections::HashSet;
use std::str::FromStr;

use anyhow::Result;
use bitcoin::{Address, Txid};
use breez_sdk_itest::{Environment, env, wait_for_balance};
use breez_sdk_spark::{
    GetInfoRequest, GetPaymentRequest, PaymentMethod, PaymentRequest, PaymentStatus, PaymentType,
    PrepareSendPaymentRequest, ReceivePaymentMethod, ReceivePaymentRequest, SendPaymentRequest,
    SyncWalletRequest,
};
use rstest::rstest;
use sspd_lib::coop_exit::coop_exit_fees;
use sspd_lib::fees::MIN_FEE_RATE_SAT_PER_KW;
use sspd_lib::static_deposit::claim_fee_sats;
use tokio::time::{Duration, Instant, sleep};
use tracing::info;

/// The SSP's coop-exit fee for one amount leaf, at the floor rate it quotes on
/// regtest, where bitcoind has no fee history for `estimatesmartfee`.
fn single_leaf_fee_sats() -> u64 {
    let fees = coop_exit_fees(1, MIN_FEE_RATE_SAT_PER_KW);
    fees.l1_broadcast_fee_sats
        .saturating_add(fees.user_fee_sats)
}

/// 0xFFF8, credited as one leaf of each power of two from 8 to 32768: a withdraw
/// takes one leaf and the rest compose [`single_leaf_fee_sats`].
const CREDIT_SATS: u64 = 65_528;

/// Mirrors the wallet's `find_exact_multiple_match`, whose greedy descending pick
/// is exact for power-of-two leaves.
fn greedy_composes(mut values: Vec<u64>, target: u64) -> bool {
    values.sort_unstable_by(|a, b| b.cmp(a));
    let mut remaining = target;
    for v in values {
        if v <= remaining {
            remaining -= v;
            if remaining == 0 {
                return true;
            }
        }
    }
    remaining == 0
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_coop_exit_withdraw_local(#[future] env: Result<Environment>) -> Result<()> {
    let env = env.await?;
    let local = env.local().create_wallet_with_side_channel().await?;

    // Private mode hides the user's leaves from non-owners; the SSP reads their
    // values over the operators' internal query, which ignores privacy, so the exit
    // needs no read access from the wallet.
    local
        .spark_wallet
        .update_wallet_settings(Some(true), None)
        .await?;

    let faucet = env.faucet()?;
    let deposit_addr = local
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::BitcoinAddress { new_address: None },
        })
        .await?
        .payment_request;
    // The claim fee comes off the deposit, so funding it on top credits exactly
    // `CREDIT_SATS` at the floor fee rate.
    faucet
        .fund_address(
            &deposit_addr,
            CREDIT_SATS + claim_fee_sats(MIN_FEE_RATE_SAT_PER_KW),
        )
        .await?;
    env.local().fixtures().bitcoind.generate_blocks(1).await?;
    let funded = wait_for_balance(&local.sdk, Some(1), None, 180).await?;
    info!("Alice funded: {funded} sats");

    // One amount leaf fixes the fee at `single_leaf_fee_sats`, which the remaining
    // leaves must compose exactly.
    local.spark_wallet.sync().await?;
    let mut values: Vec<u64> = local
        .spark_wallet
        .list_leaves()
        .await?
        .available
        .iter()
        .map(|leaf| leaf.value)
        .collect();
    values.sort_unstable_by(|a, b| b.cmp(a));
    info!("Alice leaf denominations ({}): {values:?}", values.len());

    let fee_sats = single_leaf_fee_sats();
    let mut seen = HashSet::new();
    let amount = values
        .iter()
        .copied()
        .find(|&value| {
            if !seen.insert(value) {
                return false;
            }
            let mut remainder = values.clone();
            if let Some(pos) = remainder.iter().position(|&x| x == value) {
                remainder.remove(pos);
            }
            greedy_composes(remainder, fee_sats)
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no single leaf leaves a remainder composing the {fee_sats}-sat coop-exit fee; leaves: {values:?}"
            )
        })?;
    info!("Withdraw amount (single leaf): {amount} sats; coop-exit fee: {fee_sats} sats");

    let dest: String = env
        .local()
        .fixtures()
        .bitcoind
        .rpc(
            "getnewaddress",
            &[serde_json::json!(""), serde_json::json!("bech32m")],
        )
        .await?;
    let dest_addr = Address::from_str(&dest)?.assume_checked();
    info!("Withdraw destination L1 address: {dest}");

    let balance_before = local
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;

    let prepare = local
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input {
                input: dest.clone(),
            },
            amount: Some(u128::from(amount)),
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;
    let send = local
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: None,
            idempotency_key: None,
        })
        .await?;
    assert!(
        matches!(send.payment.method, PaymentMethod::Withdraw),
        "payment method: {:?}",
        send.payment.method
    );
    assert!(
        matches!(send.payment.payment_type, PaymentType::Send),
        "payment type: {:?}",
        send.payment.payment_type
    );
    // A connector-layout rejection fails `send_payment`, so a Pending payment proves
    // the operator accepted the SSP's connector tx.
    let stored = local
        .sdk
        .get_payment(GetPaymentRequest {
            payment_id: send.payment.id.clone(),
        })
        .await?;
    assert!(
        matches!(stored.payment.status, PaymentStatus::Pending),
        "stored status: {:?}",
        stored.payment.status
    );
    info!("Withdraw submitted; operator accepted the connector, payment Pending");

    // Nothing else mines here, so mine until the SSP has broadcast the exit and
    // claimed the leaves.
    let deadline = Instant::now() + Duration::from_secs(300);
    let mut broadcast_logged = false;
    let coop_exit_txid = loop {
        env.local().fixtures().bitcoind.generate_blocks(1).await?;
        if let Some(record) = local.coop_exit_records().await?.into_iter().next() {
            if let Some(txid) = &record.broadcast_txid
                && !broadcast_logged
            {
                info!("coop-exit tx broadcast: {txid}");
                broadcast_logged = true;
            }
            if record.leaves_claimed {
                break record.coop_exit_txid;
            }
        }
        if Instant::now() >= deadline {
            anyhow::bail!("coop exit did not broadcast + claim within 300s");
        }
        sleep(Duration::from_secs(3)).await;
    };
    info!("coop exit complete: tx {coop_exit_txid}, SSP claimed the user's leaves");

    let tx = env
        .local()
        .fixtures()
        .bitcoind
        .get_transaction(&Txid::from_str(&coop_exit_txid)?)
        .await?;
    let paid = tx
        .output
        .iter()
        .any(|out| out.script_pubkey == dest_addr.script_pubkey() && out.value.to_sat() == amount);
    assert!(
        paid,
        "coop-exit tx {coop_exit_txid} must pay {amount} sats to {dest}"
    );
    info!("Destination {dest} received {amount} sats on-chain via {coop_exit_txid}");

    let record = local
        .coop_exit_records()
        .await?
        .into_iter()
        .next()
        .expect("exactly one coop exit");
    assert!(
        record.leaves_claimed,
        "the SSP must have claimed the user's committed leaves"
    );

    local.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let balance_after = local
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;
    let dropped = balance_before - balance_after;
    assert_eq!(
        dropped,
        amount + fee_sats,
        "balance should drop by amount + fee ({amount} + {fee_sats}); before {balance_before}, after {balance_after}"
    );
    info!(
        "COOP EXIT PROVEN: balance {balance_before} -> {balance_after} (-{dropped}); {amount} sats withdrawn to {dest} on-chain (tx {coop_exit_txid}); SSP claimed the leaves"
    );

    Ok(())
}
