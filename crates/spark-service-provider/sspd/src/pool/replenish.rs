use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use bitcoin::secp256k1::{PublicKey, SecretKey};
use bitcoin::{Address, OutPoint, Transaction};
use futures::{StreamExt, TryStreamExt};
use spark::tree::TreeNodeId;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::tree::builder::{TreeBlueprint, build_tree_blueprint};
use crate::tree::deposit::CreatedTree;
use crate::wakeup::Wakeup;
use crate::{
    chain::{BroadcastError, ChainClient, ChainRepository},
    fees::{self, FeeRateSource},
    pool::{
        config::{BRANCH_FACTOR, PoolConfig},
        deficit::{TreeSpec, compute_trees_needed},
        repository::{DepositTree, DepositTx, FundingBump, TreeNodes},
        restock::{PoolEvent, RestockService},
        tx_builder::{self, DepositOutput, ReplacedChild},
    },
    postgresql::PoolRepository,
    wallet::{
        SspWallet,
        onchain::{CONFLICT_CONFIRMATIONS, Utxo},
    },
};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

const BUMP_AFTER_BLOCKS: u64 = 6;

/// Bounds the funding transaction, and the operator calls one cycle makes.
const MAX_TREES_PER_FUNDING: usize = 32;

const CONCURRENT_TREE_CREATIONS: usize = 4;

/// The least wait between cycles, since each reads the whole pool.
const MIN_REPLENISH_SPACING: Duration = Duration::from_secs(10);

/// The longest a failing cycle waits before it is tried again. A failed cycle can leave
/// the operators holding the deposit addresses it prepared, of which they allow each
/// wallet a limited number.
const MAX_REPLENISH_BACKOFF: Duration = Duration::from_secs(60 * 60);

/// Keeps unconfirmed funding transactions broadcast, bumps them when due, and
/// funds new trees for what the pool is short of.
#[allow(clippy::too_many_arguments)]
pub async fn run_replenish_loop<C, R>(
    config: PoolConfig,
    wallet: Arc<SspWallet<R>>,
    chain_client: Arc<C>,
    chain_repository: Arc<R>,
    pool_repo: Arc<PoolRepository>,
    fee_rates: Arc<dyn FeeRateSource>,
    restock: Arc<RestockService>,
    chain_advanced: Wakeup,
    token: CancellationToken,
) where
    C: ChainClient + Send + Sync + 'static,
    R: ChainRepository + Send + Sync + 'static,
{
    info!(
        "Starting pool replenish loop (interval: {:?})",
        config.replenish_interval
    );
    let mut pool_changes = wallet.spark.tree_store.subscribe_balance_changes();
    let mut spacing = MIN_REPLENISH_SPACING;

    loop {
        tokio::select! {
            () = token.cancelled() => {
                info!("Replenish loop cancelled");
                return;
            }
            () = chain_advanced.waited() => {}
            _ = pool_changes.changed() => {}
            () = tokio::time::sleep(config.replenish_interval) => {}
        }

        match replenish_once(
            &config,
            &wallet,
            &chain_client,
            &chain_repository,
            &pool_repo,
            &fee_rates,
            &restock,
        )
        .await
        {
            Ok(()) => spacing = MIN_REPLENISH_SPACING,
            Err(e) => {
                spacing = spacing.saturating_mul(2).min(MAX_REPLENISH_BACKOFF);
                error!("Replenish cycle failed, trying again in {spacing:?}: {e}");
            }
        }
        tokio::select! {
            () = token.cancelled() => return,
            () = tokio::time::sleep(spacing) => {}
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Funding {
    Confirmed,
    /// A coin it spends was spent by another confirmed transaction.
    Replaced,
    Unconfirmed,
}

/// `build_funding_tx` puts the change last.
fn change_output(deposit: &DepositTx) -> Result<(OutPoint, &bitcoin::TxOut), BoxError> {
    let txid = deposit.tx.compute_txid();
    let output = deposit
        .tx
        .output
        .last()
        .ok_or_else(|| format!("funding tx {txid} has no outputs"))?;
    let vout = u32::try_from(deposit.tx.output.len())?.saturating_sub(1);
    Ok((OutPoint { txid, vout }, output))
}

async fn funding_state<R: ChainRepository>(
    chain_repository: &R,
    network: bitcoin::Network,
    deposit: &DepositTx,
) -> Result<Funding, BoxError> {
    let txid = deposit.tx.compute_txid();
    let (change, output) = change_output(deposit)?;
    let address = Address::from_script(&output.script_pubkey, network)?;
    let txos = chain_repository.get_txos_for_address(&address).await?;
    if txos.iter().any(|txo| txo.outpoint == change) {
        return Ok(Funding::Confirmed);
    }
    let inputs: Vec<OutPoint> = deposit.tx.input.iter().map(|i| i.previous_output).collect();
    let spenders = chain_repository.get_spenders(&inputs).await?;
    if spenders.iter().any(|spender| spender.txid != txid) {
        return Ok(Funding::Replaced);
    }
    Ok(Funding::Unconfirmed)
}

/// Returns whether the deposit's trees can still reach the pool.
async fn attend_deposit<C, R>(
    wallet: &SspWallet<R>,
    chain_client: &C,
    chain_repository: &R,
    pool_repo: &PoolRepository,
    deposit: &DepositTx,
    height: u64,
    fee_rate: u64,
) -> Result<bool, BoxError>
where
    C: ChainClient + Send + Sync + 'static,
    R: ChainRepository + Send + Sync + 'static,
{
    let txid = deposit.tx.compute_txid();
    match funding_state(chain_repository, wallet.onchain.network(), deposit).await? {
        Funding::Replaced => {
            pool_repo.retire_deposit(deposit).await?;
            warn!("funding tx {txid} can no longer confirm: one of its coins was spent elsewhere");
            return Ok(false);
        }
        Funding::Unconfirmed => {
            if let Err(e) =
                keep_confirming(wallet, chain_client, pool_repo, deposit, height, fee_rate).await
            {
                warn!("funding tx {txid} is unconfirmed and could not be broadcast or bumped: {e}");
            }
        }
        Funding::Confirmed => {
            if let Err(e) =
                settle_bumps(chain_client, chain_repository, pool_repo, deposit, height).await
            {
                warn!("could not settle the fee bumps of funding tx {txid}: {e}");
            }
        }
    }
    Ok(true)
}

#[allow(clippy::too_many_arguments)]
async fn replenish_once<C, R>(
    config: &PoolConfig,
    wallet: &SspWallet<R>,
    chain_client: &Arc<C>,
    chain_repository: &Arc<R>,
    pool_repo: &PoolRepository,
    fee_rates: &Arc<dyn FeeRateSource>,
    restock: &RestockService,
) -> Result<(), BoxError>
where
    C: ChainClient + Send + Sync + 'static,
    R: ChainRepository + Send + Sync + 'static,
{
    let height = chain_repository
        .get_tip()
        .await?
        .map_or(0, |tip| tip.height);
    let fee_rate = fee_rates.sat_per_kw().await?;

    let mut pending: HashMap<u64, u32> = HashMap::new();
    for deposit in pool_repo.open_deposits().await? {
        if attend_deposit(
            wallet,
            chain_client.as_ref(),
            chain_repository.as_ref(),
            pool_repo,
            &deposit,
            height,
            fee_rate,
        )
        .await?
        {
            for tree in &deposit.trees {
                let count = pending.entry(tree.denomination).or_default();
                *count = count.saturating_add(tree.leaf_count);
            }
        }
    }

    let available = wallet.onchain.list_utxos().await?;
    let available_sats: u64 = available.iter().map(|u| u.value).sum();
    // Charged as if every coin were spent, with change, since the funding
    // transaction's inputs are not selected yet.
    let base_fee = fees::fee_sats(
        fee_rate,
        tx_builder::funding_tx_weight_wu(available.len(), 1),
    )
    .saturating_add(fees::P2TR_DUST_SATS);
    let output_fee = fees::fee_sats(fee_rate, fees::P2TR_OUTPUT_WU);

    let leaves = wallet.spark.tree_store.get_leaves().await?;
    let requested = restock.pending();
    let plan = compute_trees_needed(
        &leaves,
        config,
        available_sats.saturating_sub(base_fee),
        output_fee,
        &pending,
        &requested,
        MAX_TREES_PER_FUNDING,
    );
    if plan.trees.is_empty() {
        return Ok(());
    }

    fund_trees(
        wallet,
        chain_client.as_ref(),
        pool_repo,
        fee_rate,
        height,
        &plan.trees,
        restock,
    )
    .await?;
    let funded: HashMap<u64, u32> = requested
        .into_iter()
        .map(|(denomination, count)| {
            let unplanned = plan.unplanned.get(&denomination).copied().unwrap_or(0);
            (denomination, count.saturating_sub(unplanned))
        })
        .collect();
    restock.fulfil(&funded);
    Ok(())
}

async fn rebroadcast<C: ChainClient>(chain_client: &C, tx: &Transaction) {
    match chain_client.broadcast_tx(tx.clone()).await {
        Ok(()) | Err(BroadcastError::AlreadyKnown) => {}
        Err(e) => warn!("failed to rebroadcast tx {}: {e}", tx.compute_txid()),
    }
}

async fn keep_confirming<C, R>(
    wallet: &SspWallet<R>,
    chain_client: &C,
    pool_repo: &PoolRepository,
    deposit: &DepositTx,
    height: u64,
    fee_rate: u64,
) -> Result<(), BoxError>
where
    C: ChainClient + Send + Sync + 'static,
    R: ChainRepository + Send + Sync + 'static,
{
    let bump = if bump_due(deposit, height, fee_rate) {
        match bump_funding(wallet, pool_repo, deposit, height, fee_rate).await {
            Ok(bump) => Some(bump),
            Err(e) => {
                warn!(
                    "could not bump funding tx {}: {e}",
                    deposit.tx.compute_txid()
                );
                deposit.latest_bump.clone()
            }
        }
    } else {
        deposit.latest_bump.clone()
    };
    let broadcast = match &bump {
        Some(bump) => {
            chain_client
                .broadcast_package(&[deposit.tx.clone(), bump.tx.clone()])
                .await
        }
        None => chain_client.broadcast_tx(deposit.tx.clone()).await,
    };
    match broadcast {
        Ok(()) | Err(BroadcastError::AlreadyKnown) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Keeps a confirmed funding transaction's latest child broadcast until one of its
/// children confirms, and drops the others once that child is deep enough that they
/// can no longer confirm.
async fn settle_bumps<C, R>(
    chain_client: &C,
    chain_repository: &R,
    pool_repo: &PoolRepository,
    deposit: &DepositTx,
    height: u64,
) -> Result<(), BoxError>
where
    C: ChainClient + Send + Sync + 'static,
    R: ChainRepository + Send + Sync + 'static,
{
    let Some(latest) = &deposit.latest_bump else {
        return Ok(());
    };
    let (change, _) = change_output(deposit)?;
    match chain_repository.get_spenders(&[change]).await?.first() {
        None => rebroadcast(chain_client, &latest.tx).await,
        Some(spender) => {
            let confirmations = height
                .saturating_add(1)
                .saturating_sub(spender.block_height);
            if confirmations >= CONFLICT_CONFIRMATIONS.cast_unsigned() {
                pool_repo
                    .settle_bumps(&deposit.tx.compute_txid(), &spender.txid)
                    .await?;
            }
        }
    }
    Ok(())
}

fn bump_due(deposit: &DepositTx, height: u64, fee_rate: u64) -> bool {
    let weight = deposit.tx.weight().to_wu();
    let (waited_from, paid_sats, paid_weight) = match &deposit.latest_bump {
        Some(bump) => (
            bump.height,
            deposit.fee_sats.saturating_add(bump.fee_sats),
            weight.saturating_add(bump.tx.weight().to_wu()),
        ),
        None => (deposit.stored_height, deposit.fee_sats, weight),
    };
    height >= waited_from.saturating_add(BUMP_AFTER_BLOCKS)
        && paid_sats < fees::fee_sats(fee_rate, paid_weight)
}

/// Stores a child raising the funding transaction's fee rate, before anything
/// broadcasts it. A child replacing an earlier one spends that child's coins too,
/// so the two conflict on all of them.
async fn bump_funding<R>(
    wallet: &SspWallet<R>,
    pool_repo: &PoolRepository,
    deposit: &DepositTx,
    height: u64,
    fee_rate: u64,
) -> Result<FundingBump, BoxError>
where
    R: ChainRepository + Send + Sync + 'static,
{
    let txid = deposit.tx.compute_txid();
    let (change_outpoint, output) = change_output(deposit)?;
    let change = Utxo {
        outpoint: change_outpoint,
        value: output.value.to_sat(),
        block_height: 0,
        address: Address::from_script(&output.script_pubkey, wallet.onchain.network())?,
    };
    let earlier_coins: Vec<OutPoint> = deposit
        .latest_bump
        .iter()
        .flat_map(|bump| bump.tx.input.iter().map(|input| input.previous_output))
        .filter(|outpoint| *outpoint != change_outpoint)
        .collect();
    let reused = wallet.onchain.outputs_at(&earlier_coins).await?;
    if reused.len() != earlier_coins.len() {
        return Err(format!(
            "the earlier child of funding tx {txid} spends coins the wallet does not know"
        )
        .into());
    }
    let reused_sats: u64 = reused.iter().map(|u| u.value).sum();

    let weight = deposit.tx.weight().to_wu();
    let replaces = deposit.latest_bump.as_ref().map(|bump| ReplacedChild {
        fee_sats: bump.fee_sats,
        weight_wu: bump.tx.weight().to_wu(),
    });
    let base_inputs = reused.len().saturating_add(1);
    let fee =
        |inputs: usize| tx_builder::bump_fee(fee_rate, weight, deposit.fee_sats, inputs, replaces);
    let short = |extra: usize| {
        fee(base_inputs.saturating_add(extra))
            .saturating_add(fees::P2TR_DUST_SATS)
            .saturating_sub(change.value.saturating_add(reused_sats))
    };
    let (extra, held) = if short(0) == 0 {
        (Vec::new(), None)
    } else {
        let (coins, held) = wallet.onchain.select_coins(&short).await?;
        (coins, Some(held))
    };
    let fee_sats = fee(base_inputs.saturating_add(extra.len()));

    let others: Vec<Utxo> = reused.into_iter().chain(extra).collect();
    let coins: Vec<&Utxo> = std::iter::once(&change).chain(&others).collect();
    let keys = keys_for(wallet, &coins).await?;
    // Paid to the change address, so a child the node refuses uses up no address.
    let tx = tx_builder::build_bump_tx(&change, &others, &change.address, fee_sats, &|a| {
        key_for(&keys, a)
    })?;
    let bump = FundingBump {
        tx,
        fee_sats,
        height,
    };
    pool_repo.add_bump(&txid, &bump).await?;
    drop(held);
    info!(
        "Bumped funding tx {txid} to {fee_rate} sat/kw with child {}",
        bump.tx.compute_txid()
    );
    Ok(bump)
}

async fn keys_for<R>(
    wallet: &SspWallet<R>,
    coins: &[&Utxo],
) -> Result<HashMap<Address, SecretKey>, BoxError>
where
    R: ChainRepository + Send + Sync + 'static,
{
    let mut keys = HashMap::new();
    for coin in coins {
        if !keys.contains_key(&coin.address) {
            let key = wallet
                .onchain
                .derive_keypair_for_address(&coin.address)
                .await?;
            keys.insert(coin.address.clone(), key);
        }
    }
    Ok(keys)
}

fn key_for(
    keys: &HashMap<Address, SecretKey>,
    address: &Address,
) -> Result<SecretKey, tx_builder::TxBuildError> {
    keys.get(address)
        .copied()
        .ok_or_else(|| tx_builder::TxBuildError::Signing(format!("no key for {address}")))
}

struct PlannedTree {
    blueprint: TreeBlueprint,
    leaf_ids: Vec<TreeNodeId>,
    address: Address,
    verifying_public_key: PublicKey,
}

/// Funds the trees `specs` describe with one transaction. It is broadcast only once
/// every tree is finalized and stored, so no tree output reaches the chain without
/// signed exit transactions.
async fn fund_trees<C, R>(
    wallet: &SspWallet<R>,
    chain_client: &C,
    pool_repo: &PoolRepository,
    fee_rate: u64,
    height: u64,
    specs: &[TreeSpec],
    restock: &RestockService,
) -> Result<(), BoxError>
where
    C: ChainClient + Send + Sync + 'static,
    R: ChainRepository + Send + Sync + 'static,
{
    let mut planned = Vec::with_capacity(specs.len());
    for spec in specs {
        planned.push(plan_tree(wallet, spec).await?);
    }

    let destinations: Vec<DepositOutput> = planned
        .iter()
        .map(|p| DepositOutput {
            address: p.address.clone(),
            amount: p.blueprint.value(),
        })
        .collect();
    let need = |inputs: usize| tx_builder::funding_need(&destinations, inputs, fee_rate);
    let (inputs, held) = wallet.onchain.select_coins(&need).await?;
    let (change_address, _) = wallet.onchain.next_address().await?;
    let keys = keys_for(wallet, &inputs.iter().collect::<Vec<_>>()).await?;
    let tx =
        tx_builder::build_funding_tx(&inputs, &destinations, &change_address, fee_rate, &|a| {
            key_for(&keys, a)
        })?;
    let txid = tx.compute_txid();
    let total_sats: u64 = destinations.iter().map(|d| d.amount).sum();
    info!(
        "Creating {} trees ({total_sats} sats) on funding tx {txid}",
        planned.len()
    );

    // The chain monitor has to see the outputs to tell when the trees confirm.
    for tree in &planned {
        wallet.onchain.register_watch_address(&tree.address).await?;
    }

    let created = create_trees(wallet, &planned, &tx).await?;

    let mut trees = Vec::with_capacity(created.len());
    let mut nodes = Vec::with_capacity(created.len());
    let mut signing_leaf_ids = Vec::new();
    for (((vout, created), plan), spec) in created.into_iter().zip(&planned).zip(specs) {
        signing_leaf_ids.extend(
            created
                .pairs
                .iter()
                .map(|(node, leaf_id)| (node.id.clone(), leaf_id.clone())),
        );
        trees.push(DepositTree {
            outpoint: OutPoint { txid, vout },
            deposit_address: plan.address.to_string(),
            denomination: spec.denomination,
            leaf_count: u32::try_from(spec.leaf_count)?,
        });
        nodes.push(TreeNodes {
            leaves: created.nodes.leaves,
            branches: created.nodes.branches,
        });
    }
    let input_sats: u64 = inputs.iter().map(|u| u.value).sum();
    let output_sats: u64 = tx.output.iter().map(|o| o.value.to_sat()).sum();
    let deposit = DepositTx {
        tx,
        fee_sats: input_sats.saturating_sub(output_sats),
        stored_height: height,
        latest_bump: None,
        trees,
    };
    pool_repo
        .insert_deposit(&deposit, &nodes, &signing_leaf_ids)
        .await?;
    drop(held);

    match chain_client.broadcast_tx(deposit.tx.clone()).await {
        Ok(()) | Err(BroadcastError::AlreadyKnown) => {
            info!("Broadcast funding tx {txid}");
            restock.publish(PoolEvent::FundingBroadcast {
                txid: txid.to_string(),
                total_sats,
                denominations: specs.iter().map(|spec| spec.denomination).collect(),
            });
        }
        // Stored, so the next cycle broadcasts it again.
        Err(e) => warn!("failed to broadcast funding tx {txid}: {e}"),
    }
    Ok(())
}

async fn create_trees<R>(
    wallet: &SspWallet<R>,
    planned: &[PlannedTree],
    tx: &Transaction,
) -> Result<Vec<(u32, CreatedTree)>, BoxError>
where
    R: ChainRepository + Send + Sync + 'static,
{
    let creations: Vec<_> = planned
        .iter()
        .enumerate()
        .map(|(vout, tree)| async move {
            let vout = u32::try_from(vout)?;
            let created = wallet
                .spark
                .tree_deposit_service
                .execute_deposit_tree(
                    &tree.blueprint,
                    &tree.leaf_ids,
                    &tree.verifying_public_key,
                    tx.clone(),
                    vout,
                )
                .await?;
            Ok::<_, BoxError>((vout, created))
        })
        .collect();
    futures::stream::iter(creations)
        .buffered(CONCURRENT_TREE_CREATIONS)
        .try_collect()
        .await
}

async fn plan_tree<R>(wallet: &SspWallet<R>, spec: &TreeSpec) -> Result<PlannedTree, BoxError>
where
    R: ChainRepository + Send + Sync + 'static,
{
    let blueprint = build_tree_blueprint(vec![spec.denomination; spec.leaf_count], BRANCH_FACTOR)?;
    let plan = wallet
        .spark
        .tree_deposit_service
        .plan_deposit_tree(&blueprint)
        .await?;
    let deposit_address = wallet
        .spark
        .deposit_service
        .generate_deposit_address(plan.root_public_key, &TreeNodeId::generate())
        .await?;
    Ok(PlannedTree {
        blueprint,
        leaf_ids: plan.leaf_ids,
        address: deposit_address.address,
        verifying_public_key: deposit_address.verifying_public_key,
    })
}
