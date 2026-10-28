//! The operators do not transfer a leaf once it is exiting, so the SSP can only
//! exit such a pool leaf unilaterally. When the leaf is worth more than the fees
//! of its unilateral exit, the SSP broadcasts each transaction still to confirm
//! for it, down to the leaf's refund transaction, once that transaction is mature.
//!
//! These transactions have no fee, so the SSP pays for each of them with a CPFP
//! child.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use bitcoin::{FeeRate, OutPoint, Sequence, Transaction, Txid, Weight, relative};
use spark::core::{initial_timelock_sequence, next_sequence};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::chain::{BroadcastError, ChainClient, ChainRepository as _, ChainTip};
use crate::fees;
use crate::pool::tx_builder::{
    self, ReplacedChild, bump_due, bump_fee, cpfp_child_weight, max_cpfp_child_inputs,
};
use crate::postgresql::{
    ChainRepository, MatureTransaction, PostgresLeafStore, PostgresUnilateralExitStore,
    StoredCpfpChild,
};
use crate::wakeup::Wakeup;
use crate::wallet::OnchainWallet;
use crate::wallet::onchain::{CONFLICT_CONFIRMATIONS, HeldCoins, Utxo};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Between blocks, the loop runs a pass this often to bring the fee rates up to
/// date.
const UNILATERAL_EXIT_INTERVAL: Duration = Duration::from_secs(60);

const NO_DEADLINE_CONF_TARGET: u32 = 144;

pub struct UnilateralExitDeps {
    pub leaf_store: Arc<PostgresLeafStore>,
    pub unilateral_exit_store: Arc<PostgresUnilateralExitStore>,
    pub chain_repository: Arc<ChainRepository>,
    pub chain_tip: ChainTip,
    pub chain_client: Arc<dyn ChainClient + Send + Sync>,
    pub onchain: Arc<OnchainWallet<ChainRepository>>,
    pub blocks: Wakeup,
}

pub async fn run_unilateral_exit_loop(deps: UnilateralExitDeps, token: CancellationToken) {
    loop {
        if let Err(e) = broadcast_mature(&deps).await {
            error!("could not broadcast the exit transactions of pool leaves: {e}");
        }
        tokio::select! {
            () = token.cancelled() => return,
            () = deps.blocks.waited() => {}
            () = tokio::time::sleep(UNILATERAL_EXIT_INTERVAL) => {}
        }
    }
}

fn blocks(sequence: Sequence) -> u32 {
    match sequence.to_relative_lock_time() {
        Some(relative::LockTime::Blocks(height)) => u32::from(height.value()),
        _ => 0,
    }
}

pub(crate) fn initial_timelock() -> u32 {
    blocks(initial_timelock_sequence().0)
}

/// The timelock of a new refund or node transaction is this many blocks below that
/// of the earlier transaction with the same input.
fn time_lock_interval() -> u32 {
    let initial = initial_timelock_sequence().0;
    next_sequence(initial).map_or(0, |(next, _)| blocks(initial).saturating_sub(blocks(next)))
}

/// The blocks in which a mature transaction can confirm while no other
/// transaction with the same input can.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Deadline {
    blocks_left: u64,
    window: u64,
}

/// The timelock of the next transaction to mature with the same input as
/// `mature`: the direct transaction, which the operators' watchtower broadcasts,
/// or the earlier node or refund transaction. If the timelock of `mature` is
/// between zero and the initial timelock, the earlier transaction has a timelock
/// one time lock interval higher. Whoever has that transaction can exit the leaf
/// unilaterally with it.
fn next_timelock(mature: &MatureTransaction) -> Option<u32> {
    let replaced = (mature.timelock > 0 && mature.timelock < initial_timelock())
        .then(|| mature.timelock.saturating_add(time_lock_interval()));
    [mature.direct_timelock, replaced]
        .into_iter()
        .flatten()
        .filter(|timelock| *timelock > mature.timelock)
        .min()
}

fn deadline(mature: &MatureTransaction, tip_height: u64) -> Option<Deadline> {
    let next = next_timelock(mature)?;
    Some(Deadline {
        blocks_left: mature
            .input_confirmation_height
            .saturating_add(u64::from(next))
            .saturating_sub(tip_height.saturating_add(1)),
        window: u64::from(next.saturating_sub(mature.timelock)),
    })
}

fn confirmation_target(deadline: Option<Deadline>) -> u32 {
    deadline.map_or(NO_DEADLINE_CONF_TARGET, |deadline| {
        u32::try_from(deadline.blocks_left)
            .unwrap_or(NO_DEADLINE_CONF_TARGET)
            .clamp(1, NO_DEADLINE_CONF_TARGET)
    })
}

/// A fee estimate is no guarantee, and the fewer blocks are left, the more certain
/// the confirmation has to be. So the SSP raises the estimate as the blocks run
/// out, up to double the estimate when none are left.
fn projected(estimate: FeeRate, deadline: Option<Deadline>) -> FeeRate {
    let Some(deadline) = deadline else {
        return estimate;
    };
    let passed = deadline.window.saturating_sub(deadline.blocks_left);
    let estimate = estimate.to_sat_per_kwu();
    FeeRate::from_sat_per_kwu(
        estimate.saturating_add(
            estimate
                .saturating_mul(passed)
                .checked_div(deadline.window)
                .unwrap_or(0),
        ),
    )
}

async fn broadcast_mature(deps: &UnilateralExitDeps) -> Result<(), BoxError> {
    let Some(tip) = deps.chain_tip.stored() else {
        return Ok(());
    };
    // The leaf store uses the chain record to decide which transactions are
    // mature. While the record is at another block than bitcoind, a transaction
    // that the store treats as unconfirmed may have confirmed already.
    if deps.chain_client.get_best_block_hash().await? != tip.hash {
        return Ok(());
    }
    let unspent = deps.chain_repository.confirmed_unspent_outputs().await?;
    let mut mature: Vec<(MatureTransaction, Option<Deadline>)> = deps
        .leaf_store
        .mature_transactions(&unspent, tip.height)
        .await?
        .into_iter()
        .map(|mature| {
            let deadline = deadline(&mature, tip.height);
            (mature, deadline)
        })
        .collect();
    // The SSP funds the CPFP children of the transactions with the fewest blocks
    // left first, in case the wallet does not have outputs for all of them.
    mature.sort_by_key(|(mature, deadline)| {
        let blocks_left = deadline.map_or(u64::MAX, |deadline| deadline.blocks_left);
        (blocks_left, mature.transaction.compute_txid())
    });

    let parents: HashSet<Txid> = mature
        .iter()
        .map(|(mature, _)| mature.transaction.compute_txid())
        .collect();
    let children = settle_cpfp_children(
        &deps.unilateral_exit_store,
        &deps.chain_repository,
        &parents,
        tip.height,
    )
    .await?;
    let mut estimates = HashMap::new();
    for (mature, deadline) in &mature {
        let stored = children.get(&mature.transaction.compute_txid());
        // Not every error is a fault: bitcoind can refuse the package when it
        // already has the transaction with another CPFP child.
        if let Err(e) = broadcast(deps, mature, *deadline, stored, &mut estimates).await {
            warn!(leaves = ?mature.leaves, "could not broadcast an exit transaction of a pool leaf: {e}");
        }
    }
    Ok(())
}

/// The stored CPFP children to broadcast again, by the transaction each is the
/// CPFP child of. The function deletes every other unconfirmed CPFP child, so
/// that the wallet no longer keeps its inputs out of coin selection. It also
/// deletes every CPFP child with [`CONFLICT_CONFIRMATIONS`] confirmations.
async fn settle_cpfp_children(
    unilateral_exit_store: &PostgresUnilateralExitStore,
    chain_repository: &ChainRepository,
    mature: &HashSet<Txid>,
    tip_height: u64,
) -> Result<HashMap<Txid, Transaction>, BoxError> {
    let stored = unilateral_exit_store.cpfp_children().await?;
    if stored.is_empty() {
        return Ok(HashMap::new());
    }
    let inputs: Vec<OutPoint> = stored
        .iter()
        .flat_map(StoredCpfpChild::wallet_inputs)
        .collect();
    let txids: Vec<Txid> = stored
        .iter()
        .map(|child| child.child.compute_txid())
        .chain(inputs.iter().map(|input| input.txid))
        .collect();
    let heights = chain_repository.tx_heights(&txids).await?;
    let spenders = chain_repository.get_spenders(&inputs).await?;

    let mut to_broadcast = HashMap::new();
    let mut deleted = Vec::new();
    for child in stored {
        let txid = child.child.compute_txid();
        if let Some(height) = heights.get(&txid) {
            let confirmations = tip_height.saturating_add(1).saturating_sub(*height);
            if confirmations >= CONFLICT_CONFIRMATIONS.cast_unsigned() {
                deleted.push(txid);
            }
            continue;
        }
        let inputs = child.wallet_inputs();
        let input_left_chain = inputs
            .iter()
            .any(|input| !heights.contains_key(&input.txid));
        let conflicted = spenders
            .iter()
            .any(|spender| spender.txid != txid && inputs.contains(&spender.outpoint));
        if input_left_chain || conflicted || !mature.contains(&child.parent) {
            info!(txid = %child.parent, child_txid = %txid, "the CPFP child of an exit transaction can no longer confirm, so the wallet no longer keeps its inputs out of coin selection");
            deleted.push(txid);
        } else {
            to_broadcast.insert(child.parent, child.child);
        }
    }
    unilateral_exit_store.delete_cpfp_children(&deleted).await?;
    Ok(to_broadcast)
}

async fn fee_rate(
    chain_client: &(dyn ChainClient + Send + Sync),
    estimates: &mut HashMap<u32, FeeRate>,
    deadline: Option<Deadline>,
) -> Result<FeeRate, BoxError> {
    let target = confirmation_target(deadline);
    let estimate = if let Some(estimate) = estimates.get(&target) {
        *estimate
    } else {
        let estimate = chain_client
            .estimate_fee_rate(target)
            .await?
            .max(fees::MIN_FEE_RATE);
        estimates.insert(target, estimate);
        estimate
    };
    Ok(projected(estimate, deadline))
}

/// Broadcasts `mature` with the CPFP child stored for it, so that the SSP reuses
/// the wallet inputs from one pass to the next.
async fn broadcast(
    deps: &UnilateralExitDeps,
    mature: &MatureTransaction,
    deadline: Option<Deadline>,
    stored: Option<&Transaction>,
    estimates: &mut HashMap<u32, FeeRate>,
) -> Result<(), BoxError> {
    let txid = mature.transaction.compute_txid();
    let rate = fee_rate(deps.chain_client.as_ref(), estimates, deadline).await?;
    let child = if let Some(stored) = stored {
        match replacement(deps, mature, deadline, stored, rate).await {
            Ok(Some(child)) => child,
            Ok(None) => stored.clone(),
            Err(e) => {
                warn!(%txid, "could not raise the fee of an exit transaction of a pool leaf: {e}");
                stored.clone()
            }
        }
    } else {
        if !worth_unilateral_exit(mature.value, &mature.remaining, rate) {
            debug!(leaves = ?mature.leaves, value = mature.value, %rate, "pool leaves are not worth exiting unilaterally at the current fee rate");
            return Ok(());
        }
        let (child, held) = fund_cpfp_child(deps, &mature.transaction, rate).await?;
        deps.unilateral_exit_store
            .set_cpfp_child(&txid, &child)
            .await?;
        drop(held);
        info!(leaves = ?mature.leaves, %txid, child_txid = %child.compute_txid(), %rate, ?deadline, "broadcasting an exit transaction of a pool leaf");
        child
    };
    let child_txid = child.compute_txid();
    match deps
        .chain_client
        .broadcast_package(&[mature.transaction.clone(), child])
        .await
    {
        Ok(()) | Err(BroadcastError::AlreadyKnown) => Ok(()),
        Err(BroadcastError::UnknownError(reason)) if !refused_for_its_fee(&reason) => {
            // bitcoind will not accept this CPFP child at any fee, so the SSP
            // deletes it and builds another in the next pass.
            deps.unilateral_exit_store
                .delete_cpfp_children(&[child_txid])
                .await?;
            Err(format!("bitcoind refused the CPFP child {child_txid}: {reason}").into())
        }
        Err(e) => Err(e.into()),
    }
}

/// Whether bitcoind refused a transaction for its fee. bitcoind can accept the
/// transaction later: with a CPFP child that has a higher fee, or once its mempool
/// has room.
fn refused_for_its_fee(reason: &str) -> bool {
    [
        "insufficient fee",
        "min relay fee not met",
        "mempool min fee not met",
        "package-fee-too-low",
        "mempool full",
    ]
    .iter()
    .any(|fee_reason| reason.contains(fee_reason))
}

/// The replacement for `stored` has every wallet input of `stored` among its
/// inputs, so the two CPFP children conflict on all of them.
async fn replacement(
    deps: &UnilateralExitDeps,
    mature: &MatureTransaction,
    deadline: Option<Deadline>,
    stored: &Transaction,
    rate: FeeRate,
) -> Result<Option<Transaction>, BoxError> {
    let parent = &mature.transaction;
    let parent_txid = parent.compute_txid();
    let outpoints: Vec<OutPoint> = stored
        .input
        .iter()
        .map(|input| input.previous_output)
        .filter(|outpoint| outpoint.txid != parent_txid)
        .collect();
    let mut coins = deps.onchain.outputs_at(&outpoints).await?;
    if coins.len() != outpoints.len() {
        return Err("the wallet does not have some of the stored CPFP child's inputs".into());
    }
    coins.sort_by_key(|coin| outpoints.iter().position(|o| *o == coin.outpoint));
    let coin_sats: u64 = coins.iter().map(|coin| coin.value).sum();
    let paid = coin_sats.saturating_sub(stored.output.iter().map(|o| o.value.to_sat()).sum());
    let paid_rate = rate_paid(paid, package_weight(parent, coins.len()));
    if !bump_due(paid_rate, rate) || !worth_unilateral_exit(mature.value, &mature.remaining, rate) {
        return Ok(None);
    }

    let earlier = ReplacedChild {
        fee_sats: paid,
        weight: stored.weight(),
    };
    let reused = coins.len();
    let fee = |inputs: usize| {
        let weight = cpfp_child_weight(inputs);
        bump_fee(rate, parent.weight(), 0, weight, Some(earlier))
    };
    let short = |extra: usize| {
        fee(reused.saturating_add(extra))
            .saturating_add(fees::P2TR_DUST_SATS)
            .saturating_sub(coin_sats)
    };
    let held = if short(0) == 0 {
        None
    } else {
        let most = max_cpfp_child_inputs().saturating_sub(reused);
        let (extra, held) = deps.onchain.select_at_most(most, &short).await?;
        coins.extend(extra);
        Some(held)
    };
    let child = signed_cpfp_child(deps, parent, &coins, fee(coins.len())).await?;
    deps.unilateral_exit_store
        .replace_cpfp_child(&parent_txid, &stored.compute_txid(), &child)
        .await?;
    drop(held);
    info!(leaves = ?mature.leaves, txid = %parent_txid, child_txid = %child.compute_txid(), %paid_rate, %rate, ?deadline, "raised the fee of an exit transaction of a pool leaf");
    Ok(Some(child))
}

fn worth_unilateral_exit(value: u64, txs: &[Transaction], rate: FeeRate) -> bool {
    let cost = txs
        .iter()
        .map(|tx| cpfp_child_fee(tx, 1, rate))
        .fold(0, u64::saturating_add);
    value >= fees::P2TR_DUST_SATS && value > cost
}

fn cpfp_child_fee(parent: &Transaction, inputs: usize, rate: FeeRate) -> u64 {
    let weight = cpfp_child_weight(inputs);
    bump_fee(rate, parent.weight(), 0, weight, None)
}

fn package_weight(parent: &Transaction, inputs: usize) -> Weight {
    parent
        .weight()
        .checked_add(cpfp_child_weight(inputs))
        .unwrap_or(Weight::MAX)
}

fn rate_paid(fee: u64, weight: Weight) -> FeeRate {
    FeeRate::from_sat_per_kwu(
        fee.saturating_mul(1000)
            .checked_div(weight.to_wu())
            .unwrap_or(u64::MAX),
    )
}

async fn fund_cpfp_child(
    deps: &UnilateralExitDeps,
    parent: &Transaction,
    rate: FeeRate,
) -> Result<(Transaction, HeldCoins), BoxError> {
    let need =
        |inputs: usize| cpfp_child_fee(parent, inputs, rate).saturating_add(fees::P2TR_DUST_SATS);
    let (coins, held) = deps
        .onchain
        .select_at_most(max_cpfp_child_inputs(), &need)
        .await?;
    let fee = cpfp_child_fee(parent, coins.len(), rate);
    Ok((signed_cpfp_child(deps, parent, &coins, fee).await?, held))
}

async fn signed_cpfp_child(
    deps: &UnilateralExitDeps,
    parent: &Transaction,
    coins: &[Utxo],
    fee_sats: u64,
) -> Result<Transaction, BoxError> {
    let keys = deps.onchain.keys_for(coins).await?;
    Ok(tx_builder::build_cpfp_child(
        parent,
        coins,
        fee_sats,
        &|address| tx_builder::key_for(&keys, address),
    )?)
}

#[cfg(test)]
mod tests {
    use bitcoin::hashes::Hash;
    use bitcoin::key::Secp256k1;
    use bitcoin::secp256k1::{Message, SecretKey};
    use bitcoin::sighash::{Prevouts, SighashCache, TapSighashType};
    use bitcoin::{
        Address, Amount, ScriptBuf, TxIn, TxOut, XOnlyPublicKey, absolute::LockTime,
        transaction::Version,
    };

    use super::*;

    fn anchor() -> TxOut {
        TxOut {
            value: Amount::ZERO,
            script_pubkey: ScriptBuf::from(vec![0x51, 0x02, 0x4e, 0x73]),
        }
    }

    fn p2tr(key: &SecretKey) -> Address {
        let secp = Secp256k1::new();
        let (xonly, _) = key.public_key(&secp).x_only_public_key();
        Address::p2tr(&secp, xonly, None, bitcoin::Network::Regtest)
    }

    fn exit_tx(value: u64) -> Transaction {
        let script = p2tr(&SecretKey::from_slice(&[9; 32]).unwrap()).script_pubkey();
        Transaction {
            version: Version::non_standard(3),
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::null(),
                sequence: Sequence::from_height(2_000),
                ..Default::default()
            }],
            output: vec![
                TxOut {
                    value: Amount::from_sat(value),
                    script_pubkey: script,
                },
                anchor(),
            ],
        }
    }

    fn mature(
        timelock: u32,
        direct_timelock: Option<u32>,
        input_confirmation_height: u64,
    ) -> MatureTransaction {
        MatureTransaction {
            transaction: exit_tx(1_000),
            timelock,
            input_confirmation_height,
            direct_timelock,
            leaves: Vec::new(),
            value: 1_000,
            remaining: Vec::new(),
        }
    }

    fn coin(key: &SecretKey) -> Utxo {
        Utxo {
            outpoint: OutPoint {
                txid: Txid::from_byte_array([1; 32]),
                vout: 0,
            },
            value: 50_000,
            block_height: 1,
            address: p2tr(key),
        }
    }

    fn cpfp_child(parent: &Transaction, coin: &Utxo, key: SecretKey, fee: u64) -> Transaction {
        tx_builder::build_cpfp_child(parent, std::slice::from_ref(coin), fee, &|_| Ok(key)).unwrap()
    }

    #[test]
    fn the_blocks_left_end_where_the_watchtower_broadcasts() {
        // The refund can confirm from block 2001, and the direct refund from block 2051.
        let refund = mature(1_900, Some(1_950), 101);
        let deadline = |tip: u64| deadline(&refund, tip).unwrap();

        assert_eq!(
            deadline(2_000),
            Deadline {
                blocks_left: 50,
                window: 50
            }
        );
        assert_eq!(deadline(2_049).blocks_left, 1);
        assert_eq!(deadline(2_050).blocks_left, 0);
        assert_eq!(deadline(2_500).blocks_left, 0);
    }

    #[test]
    fn the_blocks_left_end_where_the_previous_refund_matures() {
        // The refund before this one has a timelock of 2000, so it can confirm
        // from block 2101.
        assert_eq!(
            deadline(&mature(1_900, None, 101), 2_000),
            Some(Deadline {
                blocks_left: 100,
                window: 100
            })
        );
    }

    #[test]
    fn a_transaction_without_a_timelock_has_lost_a_block_of_its_window() {
        // The transaction could have confirmed in the same block as the
        // transaction above it.
        assert_eq!(
            deadline(&mature(0, Some(50), 100), 100),
            Some(Deadline {
                blocks_left: 49,
                window: 50
            })
        );
    }

    #[test]
    fn a_transaction_nothing_else_can_replace_has_no_deadline() {
        // A transaction at the initial timelock, or without a timelock, has no
        // earlier transaction with the same input.
        assert_eq!(next_timelock(&mature(2_000, None, 100)), None);
        assert_eq!(next_timelock(&mature(0, None, 100)), None);
        assert_eq!(next_timelock(&mature(0, Some(50), 100)), Some(50));
    }

    #[test]
    fn the_fee_rate_rises_as_the_blocks_left_run_out() {
        let deadline = |blocks_left: u64| {
            Some(Deadline {
                blocks_left,
                window: 50,
            })
        };
        assert_eq!(confirmation_target(None), 144);
        assert_eq!(confirmation_target(deadline(50)), 50);
        assert_eq!(confirmation_target(deadline(3)), 3);
        assert_eq!(confirmation_target(deadline(0)), 1);
        assert_eq!(
            confirmation_target(Some(Deadline {
                blocks_left: 10_000,
                window: 10_000
            })),
            144
        );

        let rate = FeeRate::from_sat_per_kwu;
        assert_eq!(projected(rate(1_000), None), rate(1_000));
        assert_eq!(projected(rate(1_000), deadline(50)), rate(1_000));
        assert_eq!(projected(rate(1_000), deadline(40)), rate(1_200));
        assert_eq!(projected(rate(1_000), deadline(1)), rate(1_980));
        assert_eq!(projected(rate(1_000), deadline(0)), rate(2_000));
    }

    #[test]
    fn a_leaf_is_exited_unilaterally_only_when_it_covers_the_fees() {
        let (node, refund) = (exit_tx(65_536), exit_tx(65_536));
        let rate = fees::MIN_FEE_RATE;

        assert!(worth_unilateral_exit(
            65_536,
            &[node.clone(), refund.clone()],
            rate
        ));
        assert!(!worth_unilateral_exit(256, &[node, refund], rate));
        assert!(!worth_unilateral_exit(64, &[], rate));
    }

    #[test]
    fn bitcoind_refuses_a_cpfp_child_for_its_fee_or_for_another_reason() {
        assert!(refused_for_its_fee(
            "insufficient fee, rejecting replacement"
        ));
        assert!(refused_for_its_fee("min relay fee not met, 100 < 141"));
        assert!(!refused_for_its_fee("bad-txns-inputs-missingorspent"));
        assert!(!refused_for_its_fee("transaction failed"));
    }

    #[test]
    fn a_cpfp_child_spends_the_anchor_and_signs_its_coins() {
        let parent = exit_tx(10_000);
        let key = SecretKey::from_slice(&[3; 32]).unwrap();
        let coin = coin(&key);
        let fee = cpfp_child_fee(&parent, 1, fees::MIN_FEE_RATE);

        let child = cpfp_child(&parent, &coin, key, fee);

        let anchor_input = child.input.last().unwrap();
        assert_eq!(
            anchor_input.previous_output,
            OutPoint::new(parent.compute_txid(), 1)
        );
        assert!(anchor_input.witness.is_empty());
        let prevouts = [
            TxOut {
                value: Amount::from_sat(coin.value),
                script_pubkey: coin.address.script_pubkey(),
            },
            anchor(),
        ];
        let sighash = SighashCache::new(&child)
            .taproot_key_spend_signature_hash(0, &Prevouts::All(&prevouts), TapSighashType::Default)
            .unwrap();
        let output_key =
            XOnlyPublicKey::from_slice(&prevouts[0].script_pubkey.as_bytes()[2..]).unwrap();
        let signature =
            bitcoin::secp256k1::schnorr::Signature::from_slice(&child.input[0].witness[0]).unwrap();
        Secp256k1::verification_only()
            .verify_schnorr(
                &signature,
                &Message::from_digest(*sighash.as_byte_array()),
                &output_key,
            )
            .unwrap();
        let paid = coin.value - child.output[0].value.to_sat();
        assert!(paid >= fee);
        assert!(paid <= fee + 2);
        assert!(package_weight(&parent, 1) >= parent.weight() + child.weight());
    }

    #[test]
    fn a_replacement_pays_what_bitcoin_core_requires_of_it() {
        let parent = exit_tx(10_000);
        let key = SecretKey::from_slice(&[3; 32]).unwrap();
        let coin = coin(&key);
        let rate = fees::MIN_FEE_RATE;
        let stored = cpfp_child(&parent, &coin, key, cpfp_child_fee(&parent, 1, rate));
        let earlier = ReplacedChild {
            fee_sats: coin.value - stored.output[0].value.to_sat(),
            weight: stored.weight(),
        };
        let weight = cpfp_child_weight(1);
        let relay = fees::fee_sats(rate, weight);
        let replacement = |rate: FeeRate| bump_fee(rate, parent.weight(), 0, weight, Some(earlier));

        // At a rate barely above that of the stored CPFP child, Bitcoin Core
        // requires more than the fee at that rate.
        let barely_above = FeeRate::from_sat_per_kwu(rate.to_sat_per_kwu() + 1);
        let triple = rate.checked_mul(3).unwrap();
        assert_eq!(replacement(barely_above), earlier.fee_sats + relay);
        assert_eq!(replacement(triple), cpfp_child_fee(&parent, 1, triple));
    }

    struct Stored {
        unilateral_exit_store: PostgresUnilateralExitStore,
        chain: ChainRepository,
        pool: Arc<sqlx::PgPool>,
        _database: crate::postgresql::testing::TestDatabase,
    }

    impl Stored {
        async fn new() -> Self {
            let database = crate::postgresql::testing::TestDatabase::new().await;
            let pool = Arc::clone(&database.pool);
            Self {
                unilateral_exit_store: PostgresUnilateralExitStore::new(Arc::clone(&pool)),
                chain: ChainRepository::new(Arc::clone(&pool), bitcoin::Network::Regtest),
                pool,
                _database: database,
            }
        }

        async fn block(&self, height: u8, outputs: &[OutPoint], spends: &[(OutPoint, Txid)]) {
            use crate::chain::{AddressUtxo, BlockHeader, SpentTxo, Txo};
            let key = SecretKey::from_slice(&[2; 32]).unwrap();
            let outputs: Vec<AddressUtxo> = outputs
                .iter()
                .map(|outpoint| AddressUtxo {
                    address: p2tr(&key),
                    utxo: Txo {
                        block_height: u64::from(height),
                        outpoint: *outpoint,
                        tx_out: TxOut {
                            value: Amount::from_sat(100_000),
                            script_pubkey: p2tr(&key).script_pubkey(),
                        },
                    },
                })
                .collect();
            let spends: Vec<SpentTxo> = spends
                .iter()
                .map(|(outpoint, spender)| SpentTxo {
                    outpoint: *outpoint,
                    spending_tx: *spender,
                    spending_input_index: 0,
                })
                .collect();
            let header = BlockHeader {
                hash: bitcoin::BlockHash::from_byte_array([height; 32]),
                height: u64::from(height),
            };
            self.chain
                .add_block(&header, &outputs, &spends)
                .await
                .unwrap();
        }

        async fn cpfp_child(&self, parent: &Transaction) -> (OutPoint, Transaction) {
            let wallet_output = OutPoint::new(Txid::from_byte_array([7; 32]), 0);
            self.block(1, &[wallet_output], &[]).await;
            let mut child = exit_tx(500);
            child.input = vec![
                TxIn {
                    previous_output: wallet_output,
                    ..Default::default()
                },
                TxIn {
                    previous_output: OutPoint::new(parent.compute_txid(), 1),
                    ..Default::default()
                },
            ];
            self.unilateral_exit_store
                .set_cpfp_child(&parent.compute_txid(), &child)
                .await
                .unwrap();
            (wallet_output, child)
        }

        async fn settle(&self, mature: &[Txid], tip_height: u64) -> HashMap<Txid, Transaction> {
            let mature = mature.iter().copied().collect();
            settle_cpfp_children(
                &self.unilateral_exit_store,
                &self.chain,
                &mature,
                tip_height,
            )
            .await
            .unwrap()
        }

        async fn recorded_spends(&self) -> i64 {
            sqlx::query_scalar("SELECT COUNT(*) FROM brz_ssp_onchain_wallet_spends")
                .fetch_one(&*self.pool)
                .await
                .unwrap()
        }
    }

    #[tokio::test]
    async fn a_transaction_keeps_the_cpfp_child_stored_for_it() {
        let stored = Stored::new().await;
        let parent = exit_tx(1_000);
        let txid = parent.compute_txid();
        let (wallet_output, child) = stored.cpfp_child(&parent).await;

        assert!(
            stored
                .unilateral_exit_store
                .set_cpfp_child(&txid, &child)
                .await
                .is_err()
        );
        let to_broadcast = stored.settle(&[txid], 1).await;
        assert_eq!(to_broadcast, [(txid, child.clone())].into_iter().collect());
        assert_eq!(stored.recorded_spends().await, 1);

        let mut replacement = child.clone();
        replacement.output[0].value = Amount::from_sat(400);
        let unknown = Txid::from_byte_array([0xee; 32]);
        let unilateral_exit_store = &stored.unilateral_exit_store;
        assert!(
            unilateral_exit_store
                .replace_cpfp_child(&txid, &unknown, &replacement)
                .await
                .is_err()
        );
        unilateral_exit_store
            .replace_cpfp_child(&txid, &child.compute_txid(), &replacement)
            .await
            .unwrap();
        let to_broadcast = stored.settle(&[txid], 1).await;
        assert_eq!(
            to_broadcast,
            [(txid, replacement.clone())].into_iter().collect()
        );
        let spending_txids: Vec<String> =
            sqlx::query_scalar("SELECT spending_txid FROM brz_ssp_onchain_wallet_spends")
                .fetch_all(&*stored.pool)
                .await
                .unwrap();
        assert_eq!(spending_txids, [replacement.compute_txid().to_string()]);
        assert_eq!(replacement.input[0].previous_output, wallet_output);
    }

    #[tokio::test]
    async fn a_cpfp_child_is_deleted_once_its_transaction_is_not_among_the_mature_transactions() {
        let stored = Stored::new().await;
        let parent = exit_tx(1_000);
        stored.cpfp_child(&parent).await;

        // The transaction's input is spent: the transaction confirmed with
        // another CPFP child, or another transaction with the same input
        // confirmed.
        assert!(stored.settle(&[], 1).await.is_empty());

        assert!(
            stored
                .unilateral_exit_store
                .cpfp_children()
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(stored.recorded_spends().await, 0);
    }

    #[tokio::test]
    async fn a_cpfp_child_is_deleted_once_another_transaction_spent_its_wallet_output() {
        let stored = Stored::new().await;
        let parent = exit_tx(1_000);
        let (wallet_output, _) = stored.cpfp_child(&parent).await;

        let another = Txid::from_byte_array([0xee; 32]);
        stored.block(2, &[], &[(wallet_output, another)]).await;

        assert!(stored.settle(&[parent.compute_txid()], 2).await.is_empty());
        assert_eq!(stored.recorded_spends().await, 0);
    }

    #[tokio::test]
    async fn a_cpfp_child_is_deleted_once_its_wallet_output_left_the_chain() {
        let stored = Stored::new().await;
        // The chain record's first block, which the chain monitor never undoes.
        stored.block(0, &[], &[]).await;
        let parent = exit_tx(1_000);
        stored.cpfp_child(&parent).await;

        let block = bitcoin::BlockHash::from_byte_array([1; 32]);
        stored.chain.undo_block(block).await.unwrap();

        assert!(stored.settle(&[parent.compute_txid()], 0).await.is_empty());
        assert_eq!(stored.recorded_spends().await, 0);
    }

    #[tokio::test]
    async fn a_cpfp_child_that_confirmed_is_kept_until_no_reorg_undoes_it() {
        let stored = Stored::new().await;
        let parent = exit_tx(1_000);
        let (wallet_output, child) = stored.cpfp_child(&parent).await;
        stored
            .block(2, &[], &[(wallet_output, child.compute_txid())])
            .await;

        // The transaction confirmed with this CPFP child, so the transaction is
        // not among `mature`.
        assert!(stored.settle(&[], 2).await.is_empty());
        assert_eq!(
            stored
                .unilateral_exit_store
                .cpfp_children()
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(stored.recorded_spends().await, 1);

        let deep = 2 + CONFLICT_CONFIRMATIONS.cast_unsigned() - 1;
        assert!(stored.settle(&[], deep).await.is_empty());
        assert!(
            stored
                .unilateral_exit_store
                .cpfp_children()
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_cpfp_child_stays_within_the_weight_bitcoin_core_relays() {
        let most = max_cpfp_child_inputs();
        assert!(most >= 1);
        assert!(cpfp_child_weight(most) <= Weight::from_vb_unchecked(1000));
        assert!(cpfp_child_weight(most + 1) > Weight::from_vb_unchecked(1000));
    }
}
