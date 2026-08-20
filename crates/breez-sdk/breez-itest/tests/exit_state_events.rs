//! How many `UnilateralExitStateChanged` events an outgoing payment produces,
//! measured from a wallet that is otherwise idle.

use std::collections::HashSet;

use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rstest::*;
use tracing::info;

/// Silence required before the send, so the events counted after it cannot be
/// leftovers from funding.
const QUIET_WINDOW_SECS: u64 = 20;

/// How long to keep counting after the send. Long enough to cover several
/// 5-second background syncs, each of which wakes the exit chain downloader.
const COUNT_WINDOW_SECS: u64 = 60;

const FUNDING_SATS: u64 = 10_000;

/// A stored leaf, identified so a leaf the send created can be told apart from
/// one it merely left alone. A leaf keeps its id across a timelock renewal and
/// only a swap mints new ones.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Leaf {
    id: String,
    value: u64,
}

/// The wallet's leaves, smallest value first. The SDK exposes no leaf listing,
/// but the exit state export carries every stored leaf.
async fn leaves(sdk: &BreezSdk) -> Result<Vec<Leaf>> {
    let export = sdk.export_unilateral_exit_state().await?;
    let envelope: serde_json::Value = serde_json::from_str(&export.exit_state)?;
    let pedigrees = envelope["pedigrees"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("exit state carries no pedigrees"))?;
    let mut leaves = pedigrees
        .iter()
        .map(|pedigree| {
            let leaf = &pedigree["leaf"];
            let id = leaf["id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("exported leaf has no id"))?;
            let value = leaf["value"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("exported leaf has no value"))?;
            Ok(Leaf {
                id: id.to_string(),
                value,
            })
        })
        .collect::<Result<Vec<Leaf>>>()?;
    leaves.sort_by(|a, b| (a.value, &a.id).cmp(&(b.value, &b.id)));
    Ok(leaves)
}

fn values(leaves: &[Leaf]) -> Vec<u64> {
    leaves.iter().map(|leaf| leaf.value).collect()
}

/// The leaves in `after` that the wallet did not hold before.
fn minted(before: &[Leaf], after: &[Leaf]) -> Vec<Leaf> {
    let held: HashSet<&str> = before.iter().map(|leaf| leaf.id.as_str()).collect();
    after
        .iter()
        .filter(|leaf| !held.contains(leaf.id.as_str()))
        .cloned()
        .collect()
}

fn subset_sums(values: &[u64]) -> HashSet<u64> {
    let mut sums = HashSet::from([0]);
    for value in values {
        for reachable in sums.clone() {
            sums.insert(reachable + value);
        }
    }
    sums
}

/// An amount no combination of the wallet's leaves adds up to, which is what
/// forces the send to swap.
fn amount_forcing_a_swap(values: &[u64]) -> Result<u64> {
    let sums = subset_sums(values);
    let total: u64 = values.iter().sum();
    (1..total)
        .find(|amount| !sums.contains(amount))
        .ok_or_else(|| anyhow::anyhow!("every amount is payable from {values:?} without a swap"))
}

/// An amount that is exactly one leaf, which is what lets the send spend a whole
/// leaf and create none.
fn amount_matching_one_leaf(values: &[u64]) -> Result<u64> {
    values
        .last()
        .copied()
        .ok_or_else(|| anyhow::anyhow!("wallet holds no leaves"))
}

/// Settles the wallet and proves it settled: one full sync, which also collects
/// every outstanding exit chain, then a window that must stay silent.
async fn quiesce(instance: &mut SdkInstance) -> Result<()> {
    instance.sdk.sync_wallet(SyncWalletRequest {}).await?;
    clear_event_receiver(&mut instance.events).await;

    let stragglers =
        count_unilateral_exit_state_changed_events(&mut instance.events, QUIET_WINDOW_SECS).await;
    anyhow::ensure!(
        stragglers == 0,
        "wallet is not quiescent: {stragglers} UnilateralExitStateChanged events \
         arrived in the {QUIET_WINDOW_SECS}s before the send"
    );
    Ok(())
}

async fn send_sats(alice: &SdkInstance, destination: &str, amount_sats: u64) -> Result<Payment> {
    let prepare = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input {
                input: destination.to_string(),
            },
            amount: Some(u128::from(amount_sats)),
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;
    let response = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: None,
            idempotency_key: None,
        })
        .await?;
    Ok(response.payment)
}

async fn spark_address(bob: &SdkInstance) -> Result<String> {
    Ok(bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request)
}

/// A send whose amount no leaf combination covers, so leaves are swapped and
/// change leaves come back. The change leaves have no exit chain yet, so the
/// resolver fetches one and reports the state change.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_exit_state_events_for_a_send_that_swaps(
    #[future] alice_sdk_no_auto_opt: Result<SdkInstance>,
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    let mut alice = alice_sdk_no_auto_opt.await?;
    let bob = bob_sdk.await?;

    ensure_funded(&mut alice, FUNDING_SATS).await?;
    quiesce(&mut alice).await?;

    let before = leaves(&alice.sdk).await?;
    let amount = amount_forcing_a_swap(&values(&before))?;
    info!(
        "Alice holds leaves {:?}, sending {amount} sats to force a swap",
        values(&before)
    );

    let payment = send_sats(&alice, &spark_address(&bob).await?, amount).await?;
    info!("Alice send status: {:?}", payment.status);

    let events =
        count_unilateral_exit_state_changed_events(&mut alice.events, COUNT_WINDOW_SECS).await;

    let after = leaves(&alice.sdk).await?;
    let minted = minted(&before, &after);
    assert!(
        !minted.is_empty(),
        "the send was meant to swap but minted no leaf: before {:?}, after {:?}",
        values(&before),
        values(&after)
    );
    assert_eq!(
        events,
        1,
        "a swapping send should report the exit state changed once, for the {} minted leaves",
        minted.len()
    );

    Ok(())
}

/// A send whose amount is exactly one existing leaf, so a whole leaf is spent
/// and none is created. Nothing is left needing an exit chain, and dropping a
/// leaf is deliberately silent, so the wallet stays quiet.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_exit_state_events_for_a_send_without_a_swap(
    #[future] alice_sdk_no_auto_opt: Result<SdkInstance>,
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    let mut alice = alice_sdk_no_auto_opt.await?;
    let bob = bob_sdk.await?;

    ensure_funded(&mut alice, FUNDING_SATS).await?;
    quiesce(&mut alice).await?;

    let before = leaves(&alice.sdk).await?;
    let amount = amount_matching_one_leaf(&values(&before))?;
    info!(
        "Alice holds leaves {:?}, sending {amount} sats to avoid a swap",
        values(&before)
    );

    let payment = send_sats(&alice, &spark_address(&bob).await?, amount).await?;
    info!("Alice send status: {:?}", payment.status);

    let events =
        count_unilateral_exit_state_changed_events(&mut alice.events, COUNT_WINDOW_SECS).await;

    let after = leaves(&alice.sdk).await?;
    let minted = minted(&before, &after);
    assert!(
        minted.is_empty(),
        "the send was meant to spend a whole leaf but minted {:?}",
        values(&minted)
    );
    assert_eq!(
        events, 0,
        "a send that mints no leaf should not report the exit state changed"
    );

    Ok(())
}
