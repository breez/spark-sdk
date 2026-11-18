#![allow(clippy::needless_raw_string_hashes)]

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use bitcoin::{OutPoint, Txid};
use chrono::{DateTime, Utc};
use deadpool_postgres::Pool;
use spark::tree::{LeafPedigree, TreeNode, TreeNodeId};
use tokio_postgres::Row;
use tokio_postgres::types::Json;

use super::{LeafStoreError, PostgresLeafStore, execute, query, query_opt};

use crate::leaves::LeafSigningKeys;
use crate::pool::repository::{DepositTree, DepositTx, FundingBump, TreeNodes};
use crate::wallet::onchain::record_spends;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

struct DepositTreeRow {
    txid: String,
    raw_tx: Vec<u8>,
    fee_sats: i64,
    stored_height: i64,
    bump_tx: Option<Vec<u8>>,
    bump_fee_sats: Option<i64>,
    bump_height: Option<i64>,
    vout: Option<i32>,
    deposit_address: Option<String>,
    denomination: Option<i64>,
    leaf_count: Option<i32>,
}

impl TryFrom<&Row> for DepositTreeRow {
    type Error = tokio_postgres::Error;

    fn try_from(row: &Row) -> Result<Self, Self::Error> {
        Ok(Self {
            txid: row.try_get("txid")?,
            raw_tx: row.try_get("raw_tx")?,
            fee_sats: row.try_get("fee_sats")?,
            stored_height: row.try_get("stored_height")?,
            bump_tx: row.try_get("bump_tx")?,
            bump_fee_sats: row.try_get("bump_fee_sats")?,
            bump_height: row.try_get("bump_height")?,
            vout: row.try_get("vout")?,
            deposit_address: row.try_get("deposit_address")?,
            denomination: row.try_get("denomination")?,
            leaf_count: row.try_get("leaf_count")?,
        })
    }
}

pub struct PoolRepository {
    pool: Arc<Pool>,
}

impl PoolRepository {
    pub fn new(pool: Arc<Pool>) -> Self {
        Self { pool }
    }

    /// Stores a funding transaction, its trees, the wallet coins it spends and the id
    /// each leaf's signing key derives from, in one database transaction.
    pub async fn insert_deposit(
        &self,
        deposit: &DepositTx,
        nodes: &[TreeNodes],
        signing_leaf_ids: &[(TreeNodeId, TreeNodeId)],
    ) -> Result<(), BoxError> {
        let txid = deposit.tx.compute_txid();
        let mut client = self.pool.get().await?;
        let db = client.transaction().await?;
        execute(
            &db,
            r#"INSERT INTO brz_ssp_spark_pool_deposit_txs (txid, raw_tx, fee_sats, stored_height)
               VALUES ($1, $2, $3, $4)"#,
            &[
                &txid.to_string(),
                &bitcoin::consensus::serialize(&deposit.tx),
                &deposit.fee_sats.cast_signed(),
                &deposit.stored_height.cast_signed(),
            ],
        )
        .await?;
        let trees: Vec<_> = deposit.trees.iter().zip(nodes).collect();
        let vouts: Vec<i32> = trees
            .iter()
            .map(|(tree, _)| tree.outpoint.vout.cast_signed())
            .collect();
        let addresses: Vec<&str> = trees
            .iter()
            .map(|(tree, _)| tree.deposit_address.as_str())
            .collect();
        let denominations: Vec<i64> = trees
            .iter()
            .map(|(tree, _)| tree.denomination.cast_signed())
            .collect();
        let leaf_counts: Vec<i32> = trees
            .iter()
            .map(|(tree, _)| tree.leaf_count.cast_signed())
            .collect();
        let leaves: Vec<_> = trees.iter().map(|(_, n)| Json(&n.leaves)).collect();
        let branches: Vec<_> = trees.iter().map(|(_, n)| Json(&n.branches)).collect();
        execute(
            &db,
            r#"INSERT INTO brz_ssp_spark_pool_deposit_trees
               (txid, vout, deposit_address, denomination, leaf_count, leaves, branches)
               SELECT $1, tree.*
               FROM UNNEST($2::int[], $3::text[], $4::bigint[], $5::int[], $6::jsonb[],
                           $7::jsonb[])
                    AS tree (vout, deposit_address, denomination, leaf_count, leaves, branches)"#,
            &[
                &txid.to_string(),
                &vouts,
                &addresses,
                &denominations,
                &leaf_counts,
                &leaves,
                &branches,
            ],
        )
        .await?;
        let (node_ids, deposit_leaf_ids): (Vec<String>, Vec<String>) = signing_leaf_ids
            .iter()
            .map(|(node_id, leaf_id)| (node_id.to_string(), leaf_id.to_string()))
            .unzip();
        execute(
            &db,
            r#"INSERT INTO brz_ssp_spark_leaf_id_map (node_id, deposit_leaf_id)
               SELECT * FROM UNNEST($1::text[], $2::text[])"#,
            &[&node_ids, &deposit_leaf_ids],
        )
        .await?;
        let inputs: Vec<OutPoint> = deposit.tx.input.iter().map(|i| i.previous_output).collect();
        record_spends(&db, &txid, &inputs).await?;
        db.commit().await?;
        Ok(())
    }

    /// Every funding transaction with a tree not yet in the pool or fee bump children
    /// not yet settled, carrying only those trees and its latest child.
    pub async fn open_deposits(&self) -> Result<Vec<DepositTx>, BoxError> {
        let rows = query(
            &self.pool.get().await?,
            r#"SELECT t.txid, t.raw_tx, t.fee_sats, t.stored_height,
                      b.raw_tx AS bump_tx, b.fee_sats AS bump_fee_sats, b.height AS bump_height,
                      d.vout, d.deposit_address, d.denomination, d.leaf_count
                   FROM brz_ssp_spark_pool_deposit_txs t
                   LEFT JOIN brz_ssp_spark_pool_deposit_trees d ON d.txid = t.txid AND NOT d.pooled
                   LEFT JOIN LATERAL (
                       SELECT raw_tx, fee_sats, height
                       FROM brz_ssp_spark_pool_funding_bumps
                       WHERE funding_txid = t.txid
                       ORDER BY created_at DESC
                       LIMIT 1
                   ) b ON TRUE
                   WHERE d.txid IS NOT NULL OR t.bump_pending
                   ORDER BY t.created_at, d.vout"#,
            &[],
        )
        .await?;

        let mut deposits: Vec<DepositTx> = Vec::new();
        let mut index: HashMap<String, usize> = HashMap::new();
        for row in &rows {
            let row = DepositTreeRow::try_from(row)?;
            let position = if let Some(position) = index.get(&row.txid) {
                *position
            } else {
                let position = deposits.len();
                let latest_bump = match (row.bump_tx, row.bump_fee_sats, row.bump_height) {
                    (Some(tx), Some(fee_sats), Some(height)) => Some(FundingBump {
                        tx: bitcoin::consensus::deserialize(&tx)?,
                        fee_sats: fee_sats.cast_unsigned(),
                        height: height.cast_unsigned(),
                    }),
                    _ => None,
                };
                index.insert(row.txid.clone(), position);
                deposits.push(DepositTx {
                    tx: bitcoin::consensus::deserialize(&row.raw_tx)?,
                    fee_sats: row.fee_sats.cast_unsigned(),
                    stored_height: row.stored_height.cast_unsigned(),
                    latest_bump,
                    trees: Vec::new(),
                });
                position
            };
            if let (Some(vout), Some(deposit_address), Some(denomination), Some(leaf_count)) = (
                row.vout,
                row.deposit_address,
                row.denomination,
                row.leaf_count,
            ) && let Some(deposit) = deposits.get_mut(position)
            {
                deposit.trees.push(DepositTree {
                    outpoint: OutPoint {
                        txid: Txid::from_str(&row.txid)?,
                        vout: vout.cast_unsigned(),
                    },
                    deposit_address,
                    denomination: denomination.cast_unsigned(),
                    leaf_count: leaf_count.cast_unsigned(),
                });
            }
        }
        Ok(deposits)
    }

    /// Stores a fee bump child of funding transaction `funding_txid` with the wallet
    /// coins it spends. Earlier children keep theirs: any of them can still confirm.
    pub async fn add_bump(&self, funding_txid: &Txid, bump: &FundingBump) -> Result<(), BoxError> {
        let txid = bump.tx.compute_txid();
        let mut client = self.pool.get().await?;
        let db = client.transaction().await?;
        execute(
            &db,
            r#"INSERT INTO brz_ssp_spark_pool_funding_bumps (txid, funding_txid, raw_tx, fee_sats, height)
               VALUES ($1, $2, $3, $4, $5)
               ON CONFLICT DO NOTHING"#,
            &[
                &txid.to_string(),
                &funding_txid.to_string(),
                &bitcoin::consensus::serialize(&bump.tx),
                &bump.fee_sats.cast_signed(),
                &bump.height.cast_signed(),
            ],
        )
        .await?;
        execute(
            &db,
            r#"UPDATE brz_ssp_spark_pool_deposit_txs SET bump_pending = TRUE WHERE txid = $1"#,
            &[&funding_txid.to_string()],
        )
        .await?;
        let inputs: Vec<OutPoint> = bump.tx.input.iter().map(|i| i.previous_output).collect();
        record_spends(&db, &txid, &inputs).await?;
        db.commit().await?;
        Ok(())
    }

    /// Drops every fee bump child of `funding_txid` but `confirmed`, and frees the
    /// wallet coins only they spend.
    pub async fn settle_bumps(
        &self,
        funding_txid: &Txid,
        confirmed: &Txid,
    ) -> Result<(), BoxError> {
        let mut client = self.pool.get().await?;
        let db = client.transaction().await?;
        let lost = query(
            &db,
            r#"DELETE FROM brz_ssp_spark_pool_funding_bumps
               WHERE funding_txid = $1 AND txid <> $2
               RETURNING txid"#,
            &[&funding_txid.to_string(), &confirmed.to_string()],
        )
        .await?
        .iter()
        .map(|row| row.try_get(0))
        .collect::<Result<Vec<String>, _>>()?;
        execute(
            &db,
            r#"DELETE FROM brz_ssp_onchain_wallet_spends WHERE spending_txid = ANY($1)"#,
            &[&lost],
        )
        .await?;
        execute(
            &db,
            r#"UPDATE brz_ssp_spark_pool_deposit_txs SET bump_pending = FALSE WHERE txid = $1"#,
            &[&funding_txid.to_string()],
        )
        .await?;
        db.commit().await?;
        Ok(())
    }

    /// Deletes a funding transaction, its trees and its fee bump children, and frees
    /// the wallet coins they spend.
    pub async fn retire_deposit(&self, deposit: &DepositTx) -> Result<(), BoxError> {
        let txid = deposit.tx.compute_txid().to_string();
        let mut client = self.pool.get().await?;
        let db = client.transaction().await?;
        execute(
            &db,
            r#"DELETE FROM brz_ssp_onchain_wallet_spends
               WHERE spending_txid = $1
               OR spending_txid IN (SELECT txid FROM brz_ssp_spark_pool_funding_bumps WHERE funding_txid = $1)"#,
            &[&txid],
        )
        .await?;
        execute(
            &db,
            r#"DELETE FROM brz_ssp_spark_pool_deposit_trees WHERE txid = $1"#,
            &[&txid],
        )
        .await?;
        execute(
            &db,
            r#"DELETE FROM brz_ssp_spark_pool_deposit_txs WHERE txid = $1"#,
            &[&txid],
        )
        .await?;
        db.commit().await?;
        Ok(())
    }

    /// Returns `None` when the store has no tree on `outpoint`.
    pub async fn tree_nodes(&self, outpoint: &OutPoint) -> Result<Option<TreeNodes>, BoxError> {
        let row = query_opt(
            &self.pool.get().await?,
            r#"SELECT leaves, branches FROM brz_ssp_spark_pool_deposit_trees WHERE txid = $1 AND vout = $2"#,
            &[&outpoint.txid.to_string(), &outpoint.vout.cast_signed()],
        )
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let Json(leaves) = row.try_get(0)?;
        let Json(branches) = row.try_get(1)?;
        Ok(Some(TreeNodes { leaves, branches }))
    }

    /// Returns `false`, and changes nothing, for a tree that is pooled already.
    pub async fn pool_tree(
        &self,
        outpoint: &OutPoint,
        leaf_store: &PostgresLeafStore,
        leaves: &[LeafPedigree],
        queried_at: DateTime<Utc>,
    ) -> Result<bool, LeafStoreError> {
        let mut client = self.pool.get().await?;
        let db = client.transaction().await?;
        let marked = execute(
            &db,
            r#"UPDATE brz_ssp_spark_pool_deposit_trees SET pooled = TRUE
               WHERE txid = $1 AND vout = $2 AND NOT pooled"#,
            &[&outpoint.txid.to_string(), &outpoint.vout.cast_signed()],
        )
        .await?;
        if marked == 0 {
            return Ok(false);
        }
        leaf_store.insert_leaves(&db, leaves).await?;
        let reported: Vec<TreeNode> = leaves.iter().map(|leaf| leaf.leaf.clone()).collect();
        leaf_store.observe(&db, &reported, queried_at).await?;
        db.commit().await?;
        leaf_store.wake_spend_check();
        Ok(true)
    }
}

#[async_trait::async_trait]
impl LeafSigningKeys for PoolRepository {
    async fn mark_signing_under_node_id(&self, node_ids: &[String]) -> Result<(), String> {
        execute(
            &self.pool.get().await.map_err(|e| e.to_string())?,
            r#"DELETE FROM brz_ssp_spark_leaf_id_map WHERE node_id = ANY($1)"#,
            &[&node_ids],
        )
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn get_signing_leaf_ids(
        &self,
        node_ids: &[String],
    ) -> Result<HashMap<String, String>, String> {
        let rows = query(
            &self.pool.get().await.map_err(|e| e.to_string())?,
            r#"SELECT node_id, deposit_leaf_id FROM brz_ssp_spark_leaf_id_map
               WHERE node_id = ANY($1)"#,
            &[&node_ids],
        )
        .await
        .map_err(|e| e.to_string())?;
        rows.iter()
            .map(|row| Ok((row.try_get(0)?, row.try_get(1)?)))
            .collect::<Result<_, tokio_postgres::Error>>()
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bitcoin::absolute::LockTime;
    use bitcoin::hashes::Hash;
    use bitcoin::transaction::Version;
    use bitcoin::{Amount, BlockHash, Network, ScriptBuf, Transaction, TxIn, TxOut};
    use chrono::Utc;
    use spark::tree::TreeNodeId;
    use uuid::Uuid;

    use super::*;
    use crate::chain::{AddressUtxo, BlockHeader, ChainRepository as _, Txo};
    use crate::postgresql::ChainRepository;
    use crate::postgresql::leaf_store::tests::{leaf, node, ssp};
    use crate::postgresql::query_one;
    use crate::postgresql::testing::TestDatabase;
    use crate::wallet::onchain::OnchainWallet;

    struct Funding {
        deposit: DepositTx,
        nodes: Vec<TreeNodes>,
        signing_leaf_ids: Vec<(TreeNodeId, TreeNodeId)>,
    }

    impl Funding {
        fn new(coins: &[OutPoint], trees: &[(u64, u32)]) -> Self {
            let mut outputs: Vec<TxOut> = trees
                .iter()
                .map(|(denomination, leaf_count)| {
                    output(denomination.saturating_mul(u64::from(*leaf_count)))
                })
                .collect();
            outputs.push(output(1_000));
            let tx = transaction(coins.iter().copied(), outputs);
            let txid = tx.compute_txid();
            let mut funding = Self {
                deposit: DepositTx {
                    tx,
                    fee_sats: 300,
                    stored_height: 100,
                    latest_bump: None,
                    trees: Vec::new(),
                },
                nodes: Vec::new(),
                signing_leaf_ids: Vec::new(),
            };
            for (vout, (denomination, leaf_count)) in (0..).zip(trees) {
                let outpoint = OutPoint::new(txid, vout);
                let tree = Uuid::now_v7().to_string();
                let values: Vec<u64> = (0..*leaf_count).map(|_| *denomination).collect();
                let root = node(&tree, None, outpoint, &values);
                let root_txid = root.node_tx.compute_txid();
                let leaves: Vec<TreeNode> = (0..*leaf_count)
                    .map(|index| {
                        let spent = OutPoint::new(root_txid, index);
                        leaf(&tree, Some(&root), spent, *denomination, 2000)
                    })
                    .collect();
                for leaf in &leaves {
                    funding
                        .signing_leaf_ids
                        .push((leaf.id.clone(), TreeNodeId::generate()));
                }
                funding.deposit.trees.push(DepositTree {
                    outpoint,
                    deposit_address: format!("bcrt1tree{vout}"),
                    denomination: *denomination,
                    leaf_count: *leaf_count,
                });
                funding.nodes.push(TreeNodes {
                    leaves,
                    branches: vec![root],
                });
            }
            funding
        }

        fn txid(&self) -> Txid {
            self.deposit.tx.compute_txid()
        }

        fn bump(&self, coins: &[OutPoint], fee_sats: u64, height: u64) -> FundingBump {
            let change = u32::try_from(self.deposit.trees.len()).unwrap();
            let inputs =
                std::iter::once(OutPoint::new(self.txid(), change)).chain(coins.iter().copied());
            FundingBump {
                tx: transaction(inputs, vec![output(10_000_u64.saturating_sub(fee_sats))]),
                fee_sats,
                height,
            }
        }

        fn pedigrees(&self, index: usize) -> Vec<LeafPedigree> {
            let nodes = &self.nodes[index];
            nodes
                .leaves
                .iter()
                .map(|leaf| LeafPedigree {
                    leaf: leaf.clone(),
                    ancestors: nodes.branches.clone(),
                })
                .collect()
        }

        fn leaf_ids(&self, index: usize) -> Vec<TreeNodeId> {
            let mut ids: Vec<TreeNodeId> = self.nodes[index]
                .leaves
                .iter()
                .map(|leaf| leaf.id.clone())
                .collect();
            ids.sort();
            ids
        }

        fn node_ids(&self) -> Vec<String> {
            self.signing_leaf_ids
                .iter()
                .map(|(node_id, _)| node_id.to_string())
                .collect()
        }

        fn signing_leaf_ids(&self, indexes: &[usize]) -> HashMap<String, String> {
            indexes
                .iter()
                .map(|index| {
                    let (node_id, leaf_id) = &self.signing_leaf_ids[*index];
                    (node_id.to_string(), leaf_id.to_string())
                })
                .collect()
        }
    }

    struct Fixture {
        store: PoolRepository,
        leaf_store: PostgresLeafStore,
        chain: Arc<ChainRepository>,
        wallet: OnchainWallet<ChainRepository>,
        pool: Arc<Pool>,
        _database: TestDatabase,
    }

    impl Fixture {
        async fn new() -> Self {
            let database = TestDatabase::new().await;
            let pool = Arc::clone(&database.pool);
            let chain = Arc::new(ChainRepository::new(Arc::clone(&pool), Network::Regtest));
            let wallet = OnchainWallet::new(
                &[7; 32],
                Network::Regtest,
                Arc::clone(&chain),
                Arc::clone(&pool),
            )
            .unwrap();
            Self {
                store: PoolRepository::new(Arc::clone(&pool)),
                leaf_store: PostgresLeafStore::new(Arc::clone(&pool), ssp(), Arc::clone(&chain)),
                chain,
                wallet,
                pool,
                _database: database,
            }
        }

        async fn insert(&self, funding: &Funding) {
            self.store
                .insert_deposit(&funding.deposit, &funding.nodes, &funding.signing_leaf_ids)
                .await
                .unwrap();
        }

        async fn open(&self) -> Vec<DepositTx> {
            self.store.open_deposits().await.unwrap()
        }

        async fn pool_tree(&self, funding: &Funding, index: usize) -> bool {
            let outpoint = &funding.deposit.trees[index].outpoint;
            let leaves = funding.pedigrees(index);
            self.store
                .pool_tree(outpoint, &self.leaf_store, &leaves, Utc::now())
                .await
                .unwrap()
        }

        async fn leaf_ids(&self) -> Vec<TreeNodeId> {
            let mut ids = self.leaf_store.node_ids(None, usize::MAX).await.unwrap();
            ids.sort();
            ids
        }

        /// Records, as the spend check would, that no unchecked output is
        /// spent, before it reads which leaves are available.
        async fn available(&self) -> Vec<TreeNodeId> {
            let unchecked = self.chain.next_unchecked_outputs(u32::MAX).await.unwrap();
            self.chain
                .record_checked_outputs(&unchecked, &[])
                .await
                .unwrap();
            self.chain.drop_unconfirmed_outputs().await.unwrap();
            let ids = self.leaf_ids().await;
            let mut available = self.leaf_store.available_among(&ids).await.unwrap();
            available.sort();
            available
        }

        async fn coin(&self, height: u8) -> OutPoint {
            let (address, _) = self.wallet.next_address().await.unwrap();
            let coin = outpoint(height);
            let block = BlockHeader {
                hash: BlockHash::from_byte_array([height; 32]),
                height: u64::from(height),
            };
            let paid = AddressUtxo {
                utxo: Txo {
                    block_height: block.height,
                    outpoint: coin,
                    tx_out: TxOut {
                        value: Amount::from_sat(50_000),
                        script_pubkey: address.script_pubkey(),
                    },
                },
                address,
            };
            self.chain.add_block(&block, &[paid], &[]).await.unwrap();
            coin
        }

        async fn coins(&self) -> Vec<OutPoint> {
            let coins = self.wallet.list_utxos().await.unwrap();
            let mut coins: Vec<OutPoint> = coins.iter().map(|coin| coin.outpoint).collect();
            coins.sort();
            coins
        }

        async fn count(&self, table: &str) -> i64 {
            let client = self.pool.get().await.unwrap();
            let sql = format!("SELECT COUNT(*) FROM {table}");
            query_one(&client, &sql, &[]).await.unwrap().get(0)
        }
    }

    fn output(sats: u64) -> TxOut {
        TxOut {
            value: Amount::from_sat(sats),
            script_pubkey: ScriptBuf::new(),
        }
    }

    fn transaction(inputs: impl Iterator<Item = OutPoint>, outputs: Vec<TxOut>) -> Transaction {
        Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: inputs
                .map(|previous_output| TxIn {
                    previous_output,
                    ..Default::default()
                })
                .collect(),
            output: outputs,
        }
    }

    fn outpoint(byte: u8) -> OutPoint {
        OutPoint::new(Txid::from_byte_array([byte; 32]), 0)
    }

    fn txids(deposits: &[DepositTx]) -> Vec<Txid> {
        deposits
            .iter()
            .map(|deposit| deposit.tx.compute_txid())
            .collect()
    }

    fn trees(deposit: &DepositTx) -> Vec<(OutPoint, String, u64, u32)> {
        let mut trees: Vec<_> = deposit
            .trees
            .iter()
            .map(|tree| {
                (
                    tree.outpoint,
                    tree.deposit_address.clone(),
                    tree.denomination,
                    tree.leaf_count,
                )
            })
            .collect();
        trees.sort();
        trees
    }

    fn latest_bump(deposit: &DepositTx) -> Option<(Transaction, u64, u64)> {
        deposit.latest_bump.as_ref().map(fields)
    }

    fn fields(bump: &FundingBump) -> (Transaction, u64, u64) {
        (bump.tx.clone(), bump.fee_sats, bump.height)
    }

    #[tokio::test]
    async fn a_stored_funding_transaction_is_open_with_its_fee_and_height() {
        let fixture = Fixture::new().await;
        let mut funding = Funding::new(&[outpoint(1)], &[(1000, 2)]);
        funding.deposit.fee_sats = 321;
        funding.deposit.stored_height = 654;

        fixture.insert(&funding).await;

        let open = fixture.open().await;
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].tx, funding.deposit.tx);
        assert_eq!(open[0].fee_sats, 321);
        assert_eq!(open[0].stored_height, 654);
        assert!(open[0].latest_bump.is_none());
    }

    #[tokio::test]
    async fn each_tree_is_stored_with_its_own_output_address_denomination_and_leaf_count() {
        let fixture = Fixture::new().await;
        let funding = Funding::new(&[outpoint(1)], &[(1000, 2), (2000, 3), (4000, 1)]);

        fixture.insert(&funding).await;

        let open = fixture.open().await;
        assert_eq!(open.len(), 1);
        assert_eq!(trees(&open[0]), trees(&funding.deposit));
    }

    #[tokio::test]
    async fn each_tree_reads_back_its_own_leaves_and_branches() {
        let fixture = Fixture::new().await;
        let funding = Funding::new(&[outpoint(1)], &[(1000, 2), (2000, 3)]);

        fixture.insert(&funding).await;

        for (tree, nodes) in funding.deposit.trees.iter().zip(&funding.nodes) {
            let stored = fixture.store.tree_nodes(&tree.outpoint).await.unwrap();
            let stored = stored.unwrap();
            assert_eq!(stored.leaves, nodes.leaves);
            assert_eq!(stored.branches, nodes.branches);
        }
    }

    #[tokio::test]
    async fn the_nodes_of_a_tree_the_store_does_not_have_cannot_be_read() {
        let fixture = Fixture::new().await;
        let funding = Funding::new(&[outpoint(1)], &[(1000, 2)]);
        fixture.insert(&funding).await;
        let not_a_tree = OutPoint::new(funding.txid(), 1);

        let nodes = fixture.store.tree_nodes(&not_a_tree).await;

        assert!(nodes.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_funding_transaction_without_trees_is_stored_but_not_open() {
        let fixture = Fixture::new().await;
        let coin = fixture.coin(1).await;
        let funding = Funding::new(&[coin], &[]);

        fixture.insert(&funding).await;

        assert!(fixture.open().await.is_empty());
        assert!(fixture.coins().await.is_empty());
    }

    #[tokio::test]
    async fn the_signing_leaf_id_of_each_leaf_is_stored_with_its_funding_transaction() {
        let fixture = Fixture::new().await;
        let funding = Funding::new(&[outpoint(1)], &[(1000, 2), (2000, 1)]);

        fixture.insert(&funding).await;

        let stored = fixture
            .store
            .get_signing_leaf_ids(&funding.node_ids())
            .await;
        assert_eq!(stored.unwrap(), funding.signing_leaf_ids(&[0, 1, 2]));
    }

    #[tokio::test]
    async fn the_coins_a_funding_transaction_spends_are_kept_out_of_coin_selection() {
        let fixture = Fixture::new().await;
        let (first, second, other) = (
            fixture.coin(1).await,
            fixture.coin(2).await,
            fixture.coin(3).await,
        );
        let funding = Funding::new(&[first, second], &[(1000, 2)]);

        fixture.insert(&funding).await;

        assert_eq!(fixture.coins().await, [other]);
    }

    #[tokio::test]
    async fn nothing_of_a_funding_transaction_is_stored_when_a_part_of_it_cannot_be() {
        let fixture = Fixture::new().await;
        let coin = fixture.coin(1).await;
        let stored = Funding::new(&[outpoint(9)], &[(1000, 1)]);
        fixture.insert(&stored).await;
        let mut refused = Funding::new(&[coin], &[(2000, 2)]);
        // The store has a signing leaf id for this node already.
        refused
            .signing_leaf_ids
            .push(stored.signing_leaf_ids[0].clone());

        let inserted = fixture
            .store
            .insert_deposit(&refused.deposit, &refused.nodes, &refused.signing_leaf_ids)
            .await;

        assert!(inserted.is_err());
        assert_eq!(txids(&fixture.open().await), [stored.txid()]);
        let tree = &refused.deposit.trees[0].outpoint;
        assert!(fixture.store.tree_nodes(tree).await.unwrap().is_none());
        assert_eq!(fixture.coins().await, [coin]);
    }

    #[tokio::test]
    async fn each_open_funding_transaction_carries_its_own_trees() {
        let fixture = Fixture::new().await;
        let first = Funding::new(&[outpoint(1)], &[(1000, 2), (2000, 1)]);
        let second = Funding::new(&[outpoint(2)], &[(4000, 3)]);

        fixture.insert(&first).await;
        fixture.insert(&second).await;

        let open = fixture.open().await;
        assert_eq!(open.len(), 2);
        for funding in [&first, &second] {
            let stored = open.iter().find(|deposit| deposit.tx == funding.deposit.tx);
            assert_eq!(trees(stored.unwrap()), trees(&funding.deposit));
        }
    }

    #[tokio::test]
    async fn an_open_funding_transaction_no_longer_carries_a_tree_that_is_pooled() {
        let fixture = Fixture::new().await;
        let funding = Funding::new(&[outpoint(1)], &[(1000, 2), (2000, 1)]);
        fixture.insert(&funding).await;

        assert!(fixture.pool_tree(&funding, 0).await);

        let open = fixture.open().await;
        assert_eq!(open.len(), 1);
        assert_eq!(trees(&open[0]), trees(&funding.deposit)[1..]);
    }

    #[tokio::test]
    async fn a_funding_transaction_whose_trees_are_all_pooled_is_no_longer_open() {
        let fixture = Fixture::new().await;
        let pooled = Funding::new(&[outpoint(1)], &[(1000, 2), (2000, 1)]);
        let other = Funding::new(&[outpoint(2)], &[(4000, 1)]);
        fixture.insert(&pooled).await;
        fixture.insert(&other).await;

        assert!(fixture.pool_tree(&pooled, 0).await);
        assert!(fixture.pool_tree(&pooled, 1).await);

        assert_eq!(txids(&fixture.open().await), [other.txid()]);
    }

    #[tokio::test]
    async fn the_leaves_of_a_pooled_tree_join_the_pool() {
        let fixture = Fixture::new().await;
        let funding = Funding::new(&[outpoint(1)], &[(1000, 2), (2000, 1)]);
        fixture.insert(&funding).await;

        assert!(fixture.pool_tree(&funding, 0).await);

        assert_eq!(fixture.available().await, funding.leaf_ids(0));
    }

    #[tokio::test]
    async fn a_tree_is_pooled_only_once() {
        let fixture = Fixture::new().await;
        let funding = Funding::new(&[outpoint(1)], &[(1000, 2)]);
        let other = Funding::new(&[outpoint(2)], &[(1000, 2)]);
        fixture.insert(&funding).await;
        assert!(fixture.pool_tree(&funding, 0).await);

        let tree = &funding.deposit.trees[0].outpoint;
        let again = fixture
            .store
            .pool_tree(tree, &fixture.leaf_store, &other.pedigrees(0), Utc::now())
            .await;

        assert!(!again.unwrap());
        assert_eq!(fixture.leaf_ids().await, funding.leaf_ids(0));
    }

    #[tokio::test]
    async fn the_leaves_of_a_retired_funding_transactions_tree_stay_out_of_the_pool() {
        let fixture = Fixture::new().await;
        let funding = Funding::new(&[outpoint(1)], &[(1000, 2)]);
        fixture.insert(&funding).await;
        fixture
            .store
            .retire_deposit(&funding.deposit)
            .await
            .unwrap();

        assert!(!fixture.pool_tree(&funding, 0).await);

        assert!(fixture.leaf_ids().await.is_empty());
    }

    #[tokio::test]
    async fn a_tree_stays_unpooled_when_the_leaf_store_refuses_its_leaves() {
        let fixture = Fixture::new().await;
        let funding = Funding::new(&[outpoint(1)], &[(1000, 2)]);
        fixture.insert(&funding).await;
        let tree = &funding.deposit.trees[0].outpoint;
        let mut leaves = funding.pedigrees(0);
        leaves[1].leaf.refund_tx = None;

        let pooled = fixture
            .store
            .pool_tree(tree, &fixture.leaf_store, &leaves, Utc::now())
            .await;

        assert!(matches!(pooled, Err(LeafStoreError::Invalid(_))));
        assert!(fixture.leaf_ids().await.is_empty());
        assert!(fixture.pool_tree(&funding, 0).await);
    }

    #[tokio::test]
    async fn the_spend_check_is_woken_only_when_a_tree_joins_the_pool() {
        let fixture = Fixture::new().await;
        let funding = Funding::new(&[outpoint(1)], &[(1000, 2)]);
        fixture.insert(&funding).await;
        let spend_check = fixture.chain.watched();
        let woken = async || {
            tokio::time::timeout(Duration::from_millis(100), spend_check.waited())
                .await
                .is_ok()
        };

        assert!(fixture.pool_tree(&funding, 0).await);
        assert!(woken().await);

        assert!(!fixture.pool_tree(&funding, 0).await);
        assert!(!woken().await);
    }

    #[tokio::test]
    async fn an_open_funding_transaction_carries_its_latest_fee_bump() {
        let fixture = Fixture::new().await;
        let funding = Funding::new(&[outpoint(1)], &[(1000, 2)]);
        fixture.insert(&funding).await;
        let earlier = funding.bump(&[outpoint(2)], 500, 106);
        let later = funding.bump(&[outpoint(2), outpoint(3)], 900, 112);

        fixture
            .store
            .add_bump(&funding.txid(), &earlier)
            .await
            .unwrap();
        assert_eq!(
            latest_bump(&fixture.open().await[0]),
            Some(fields(&earlier))
        );

        fixture
            .store
            .add_bump(&funding.txid(), &later)
            .await
            .unwrap();
        assert_eq!(latest_bump(&fixture.open().await[0]), Some(fields(&later)));
    }

    #[tokio::test]
    async fn a_funding_transaction_with_an_unsettled_fee_bump_stays_open_once_its_trees_are_pooled()
    {
        let fixture = Fixture::new().await;
        let funding = Funding::new(&[outpoint(1)], &[(1000, 2)]);
        fixture.insert(&funding).await;
        let bump = funding.bump(&[outpoint(2)], 500, 106);
        fixture
            .store
            .add_bump(&funding.txid(), &bump)
            .await
            .unwrap();

        assert!(fixture.pool_tree(&funding, 0).await);

        let open = fixture.open().await;
        assert_eq!(txids(&open), [funding.txid()]);
        assert!(open[0].trees.is_empty());
        assert_eq!(latest_bump(&open[0]), Some(fields(&bump)));
    }

    #[tokio::test]
    async fn a_fee_bump_stored_again_is_stored_once() {
        let fixture = Fixture::new().await;
        let coin = fixture.coin(1).await;
        let funding = Funding::new(&[outpoint(9)], &[(1000, 2)]);
        fixture.insert(&funding).await;
        let bump = funding.bump(&[coin], 500, 106);

        for _ in 0..2 {
            fixture
                .store
                .add_bump(&funding.txid(), &bump)
                .await
                .unwrap();
        }

        assert_eq!(fixture.count("brz_ssp_spark_pool_funding_bumps").await, 1);
        assert_eq!(latest_bump(&fixture.open().await[0]), Some(fields(&bump)));
        assert!(fixture.coins().await.is_empty());
    }

    #[tokio::test]
    async fn the_coins_of_every_fee_bump_are_kept_out_of_coin_selection() {
        let fixture = Fixture::new().await;
        let (first, second, other) = (
            fixture.coin(1).await,
            fixture.coin(2).await,
            fixture.coin(3).await,
        );
        let funding = Funding::new(&[outpoint(9)], &[(1000, 2)]);
        fixture.insert(&funding).await;

        for bump in [
            funding.bump(&[first], 500, 106),
            funding.bump(&[second], 900, 112),
        ] {
            fixture
                .store
                .add_bump(&funding.txid(), &bump)
                .await
                .unwrap();
        }

        assert_eq!(fixture.coins().await, [other]);
    }

    #[tokio::test]
    async fn a_fee_bump_of_a_funding_transaction_the_store_does_not_have_is_not_stored() {
        let fixture = Fixture::new().await;
        let coin = fixture.coin(1).await;
        let not_stored = Funding::new(&[outpoint(9)], &[(1000, 2)]);
        let bump = not_stored.bump(&[coin], 500, 106);

        let added = fixture.store.add_bump(&not_stored.txid(), &bump).await;

        assert!(added.is_err());
        assert_eq!(fixture.count("brz_ssp_spark_pool_funding_bumps").await, 0);
        assert_eq!(fixture.coins().await, [coin]);
    }

    #[tokio::test]
    async fn settling_drops_every_fee_bump_but_the_confirmed_one() {
        let fixture = Fixture::new().await;
        let funding = Funding::new(&[outpoint(1)], &[(1000, 2)]);
        fixture.insert(&funding).await;
        let earlier = funding.bump(&[outpoint(2)], 500, 106);
        let confirmed = funding.bump(&[outpoint(2)], 700, 112);
        let later = funding.bump(&[outpoint(2)], 900, 118);
        for bump in [&earlier, &confirmed, &later] {
            fixture.store.add_bump(&funding.txid(), bump).await.unwrap();
        }

        fixture
            .store
            .settle_bumps(&funding.txid(), &confirmed.tx.compute_txid())
            .await
            .unwrap();

        assert_eq!(fixture.count("brz_ssp_spark_pool_funding_bumps").await, 1);
        assert_eq!(
            latest_bump(&fixture.open().await[0]),
            Some(fields(&confirmed))
        );
    }

    #[tokio::test]
    async fn settling_frees_the_coins_only_the_dropped_fee_bumps_spend() {
        let fixture = Fixture::new().await;
        let (in_both, in_the_dropped_one) = (fixture.coin(1).await, fixture.coin(2).await);
        let funding = Funding::new(&[outpoint(9)], &[(1000, 2)]);
        fixture.insert(&funding).await;
        let confirmed = funding.bump(&[in_both], 500, 106);
        let dropped = funding.bump(&[in_both, in_the_dropped_one], 900, 112);
        for bump in [&confirmed, &dropped] {
            fixture.store.add_bump(&funding.txid(), bump).await.unwrap();
        }
        assert!(fixture.coins().await.is_empty());

        fixture
            .store
            .settle_bumps(&funding.txid(), &confirmed.tx.compute_txid())
            .await
            .unwrap();

        assert_eq!(fixture.coins().await, [in_the_dropped_one]);
    }

    #[tokio::test]
    async fn a_funding_transaction_with_settled_fee_bumps_and_pooled_trees_is_no_longer_open() {
        let fixture = Fixture::new().await;
        let funding = Funding::new(&[outpoint(1)], &[(1000, 2)]);
        fixture.insert(&funding).await;
        let bump = funding.bump(&[outpoint(2)], 500, 106);
        fixture
            .store
            .add_bump(&funding.txid(), &bump)
            .await
            .unwrap();
        assert!(fixture.pool_tree(&funding, 0).await);
        assert_eq!(txids(&fixture.open().await), [funding.txid()]);

        fixture
            .store
            .settle_bumps(&funding.txid(), &bump.tx.compute_txid())
            .await
            .unwrap();

        assert!(fixture.open().await.is_empty());
    }

    #[tokio::test]
    async fn settling_leaves_the_fee_bumps_of_another_funding_transaction_alone() {
        let fixture = Fixture::new().await;
        let (earlier_coin, later_coin) = (fixture.coin(1).await, fixture.coin(2).await);
        let settled = Funding::new(&[outpoint(8)], &[(1000, 2)]);
        let other = Funding::new(&[outpoint(9)], &[(2000, 1)]);
        fixture.insert(&settled).await;
        fixture.insert(&other).await;
        let confirmed = settled.bump(&[outpoint(7)], 500, 106);
        let earlier = other.bump(&[earlier_coin], 500, 106);
        let later = other.bump(&[later_coin], 900, 112);
        for (funding, bump) in [(&settled, &confirmed), (&other, &earlier), (&other, &later)] {
            fixture.store.add_bump(&funding.txid(), bump).await.unwrap();
        }
        assert!(fixture.pool_tree(&other, 0).await);

        fixture
            .store
            .settle_bumps(&settled.txid(), &confirmed.tx.compute_txid())
            .await
            .unwrap();

        let open = fixture.open().await;
        let still_open = open.iter().find(|deposit| deposit.tx == other.deposit.tx);
        assert_eq!(latest_bump(still_open.unwrap()), Some(fields(&later)));
        assert_eq!(fixture.count("brz_ssp_spark_pool_funding_bumps").await, 3);
        assert!(fixture.coins().await.is_empty());
    }

    #[tokio::test]
    async fn a_retired_funding_transaction_is_gone_with_its_trees_and_fee_bumps() {
        let fixture = Fixture::new().await;
        let funding = Funding::new(&[outpoint(1)], &[(1000, 2), (2000, 1)]);
        fixture.insert(&funding).await;
        let bump = funding.bump(&[outpoint(2)], 500, 106);
        fixture
            .store
            .add_bump(&funding.txid(), &bump)
            .await
            .unwrap();
        assert!(fixture.pool_tree(&funding, 0).await);

        fixture
            .store
            .retire_deposit(&funding.deposit)
            .await
            .unwrap();

        assert!(fixture.open().await.is_empty());
        for table in [
            "brz_ssp_spark_pool_deposit_txs",
            "brz_ssp_spark_pool_deposit_trees",
            "brz_ssp_spark_pool_funding_bumps",
        ] {
            assert_eq!(fixture.count(table).await, 0, "{table}");
        }
    }

    #[tokio::test]
    async fn retiring_frees_the_coins_of_the_funding_transaction_and_of_its_fee_bumps() {
        let fixture = Fixture::new().await;
        let (funded, bumped) = (fixture.coin(1).await, fixture.coin(2).await);
        let funding = Funding::new(&[funded], &[(1000, 2)]);
        fixture.insert(&funding).await;
        let bump = funding.bump(&[bumped], 500, 106);
        fixture
            .store
            .add_bump(&funding.txid(), &bump)
            .await
            .unwrap();
        assert!(fixture.coins().await.is_empty());

        fixture
            .store
            .retire_deposit(&funding.deposit)
            .await
            .unwrap();

        assert_eq!(fixture.coins().await, [funded, bumped]);
    }

    #[tokio::test]
    async fn retiring_a_funding_transaction_leaves_another_alone() {
        let fixture = Fixture::new().await;
        let (retired_coin, kept_coin, kept_bump_coin) = (
            fixture.coin(1).await,
            fixture.coin(2).await,
            fixture.coin(3).await,
        );
        let retired = Funding::new(&[retired_coin], &[(1000, 2)]);
        let kept = Funding::new(&[kept_coin], &[(2000, 1), (4000, 2)]);
        fixture.insert(&retired).await;
        fixture.insert(&kept).await;
        let retired_bump = retired.bump(&[outpoint(9)], 500, 106);
        let kept_bump = kept.bump(&[kept_bump_coin], 500, 106);
        for (funding, bump) in [(&retired, &retired_bump), (&kept, &kept_bump)] {
            fixture.store.add_bump(&funding.txid(), bump).await.unwrap();
        }

        fixture
            .store
            .retire_deposit(&retired.deposit)
            .await
            .unwrap();

        let open = fixture.open().await;
        assert_eq!(txids(&open), [kept.txid()]);
        assert_eq!(trees(&open[0]), trees(&kept.deposit));
        assert_eq!(latest_bump(&open[0]), Some(fields(&kept_bump)));
        for (tree, nodes) in kept.deposit.trees.iter().zip(&kept.nodes) {
            let stored = fixture.store.tree_nodes(&tree.outpoint).await.unwrap();
            assert_eq!(stored.unwrap().leaves, nodes.leaves);
        }
        assert_eq!(fixture.coins().await, [retired_coin]);
    }

    #[tokio::test]
    async fn only_the_given_nodes_are_read_with_their_signing_leaf_id() {
        let fixture = Fixture::new().await;
        let funding = Funding::new(&[outpoint(1)], &[(1000, 3)]);
        fixture.insert(&funding).await;
        let node_ids = funding.node_ids();

        let none = fixture.store.get_signing_leaf_ids(&[]).await;
        let one = fixture.store.get_signing_leaf_ids(&node_ids[1..2]).await;

        assert!(none.unwrap().is_empty());
        assert_eq!(one.unwrap(), funding.signing_leaf_ids(&[1]));
    }

    #[tokio::test]
    async fn a_node_without_a_signing_leaf_id_is_left_out() {
        let fixture = Fixture::new().await;
        let funding = Funding::new(&[outpoint(1)], &[(1000, 3)]);
        fixture.insert(&funding).await;
        let mut node_ids = funding.node_ids();
        node_ids.push(TreeNodeId::generate().to_string());

        let stored = fixture.store.get_signing_leaf_ids(&node_ids).await;

        assert_eq!(stored.unwrap(), funding.signing_leaf_ids(&[0, 1, 2]));
    }

    #[tokio::test]
    async fn a_node_given_twice_is_read_with_its_one_signing_leaf_id() {
        let fixture = Fixture::new().await;
        let funding = Funding::new(&[outpoint(1)], &[(1000, 3)]);
        fixture.insert(&funding).await;
        let node_id = funding.node_ids().remove(0);

        let stored = fixture
            .store
            .get_signing_leaf_ids(&[node_id.clone(), node_id])
            .await;

        assert_eq!(stored.unwrap(), funding.signing_leaf_ids(&[0]));
    }

    #[tokio::test]
    async fn a_node_marked_as_signing_under_its_node_id_has_no_signing_leaf_id_any_more() {
        let fixture = Fixture::new().await;
        let funding = Funding::new(&[outpoint(1)], &[(1000, 3)]);
        fixture.insert(&funding).await;
        let node_ids = funding.node_ids();
        let marked = [
            node_ids[0].clone(),
            node_ids[1].clone(),
            node_ids[1].clone(),
        ];

        fixture
            .store
            .mark_signing_under_node_id(&marked)
            .await
            .unwrap();

        let stored = fixture.store.get_signing_leaf_ids(&node_ids).await;
        assert_eq!(stored.unwrap(), funding.signing_leaf_ids(&[2]));
    }

    #[tokio::test]
    async fn marking_no_node_or_one_without_a_signing_leaf_id_changes_nothing() {
        let fixture = Fixture::new().await;
        let funding = Funding::new(&[outpoint(1)], &[(1000, 3)]);
        fixture.insert(&funding).await;
        let store = &fixture.store;

        store.mark_signing_under_node_id(&[]).await.unwrap();
        store
            .mark_signing_under_node_id(&[TreeNodeId::generate().to_string()])
            .await
            .unwrap();

        let stored = store.get_signing_leaf_ids(&funding.node_ids()).await;
        assert_eq!(stored.unwrap(), funding.signing_leaf_ids(&[0, 1, 2]));
    }
}
