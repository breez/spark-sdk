use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use bitcoin::absolute::LockTime;
use bitcoin::hashes::Hash;
use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
use bitcoin::transaction::Version;
use bitcoin::{
    Amount, BlockHash, Network, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Txid,
    Witness,
};
use chrono::{Duration, Utc};
use frost_secp256k1_tr::Identifier;
use spark::services::TransferId;
use spark::tree::{LeafPedigree, SigningKeyshare, TreeNode, TreeNodeId, TreeNodeStatus};
use sqlx::PgPool;
use uuid::Uuid;

use super::{Dispute, LeafStoreError, PostgresLeafStore, RenewalCursor, Reported, exit_chain};
use crate::chain::ConfirmedOutput;
use crate::chain::{BlockHeader, ChainRepository as _, OutputSpend, Spender, SpentTxo};
use crate::postgresql::ChainRepository;
use crate::postgresql::testing::TestDatabase;

struct Fixture {
    store: Arc<PostgresLeafStore>,
    chain: Arc<ChainRepository>,
    pool: Arc<PgPool>,
    _database: TestDatabase,
}

fn alone(leaf: &TreeNode) -> LeafPedigree {
    LeafPedigree {
        leaf: leaf.clone(),
        ancestors: Vec::new(),
    }
}

impl Fixture {
    async fn new() -> Self {
        let database = TestDatabase::new().await;
        let pool = Arc::clone(&database.pool);
        let chain = Arc::new(ChainRepository::new(Arc::clone(&pool), Network::Regtest));
        Self {
            store: Arc::new(PostgresLeafStore::new(
                Arc::clone(&pool),
                ssp(),
                Arc::clone(&chain),
            )),
            chain,
            pool,
            _database: database,
        }
    }

    async fn count(&self, table: &str) -> i64 {
        sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(&*self.pool)
            .await
            .unwrap()
    }

    async fn own(&self, leaves: &[TreeNode]) -> Result<(), LeafStoreError> {
        self.own_unchecked(leaves).await?;
        self.unspent().await;
        Ok(())
    }

    async fn own_unchecked(&self, leaves: &[TreeNode]) -> Result<(), LeafStoreError> {
        let pedigrees: Vec<LeafPedigree> = leaves.iter().map(alone).collect();
        self.own_pedigrees(&pedigrees).await
    }

    async fn own_pedigrees(&self, pedigrees: &[LeafPedigree]) -> Result<(), LeafStoreError> {
        let leaves: Vec<TreeNode> = pedigrees.iter().map(|p| p.leaf.clone()).collect();
        let mut db = self.pool.begin().await?;
        self.store.insert_leaves(&mut db, pedigrees).await?;
        self.store.observe(&mut db, &leaves, Utc::now()).await?;
        db.commit().await?;
        Ok(())
    }

    async fn store_ancestors(&self, pedigrees: &[LeafPedigree]) {
        let mut db = self.pool.begin().await.unwrap();
        self.store.insert_leaves(&mut db, pedigrees).await.unwrap();
        db.commit().await.unwrap();
    }

    async fn unspent(&self) {
        let unchecked = self.chain.next_unchecked_outputs(u32::MAX).await.unwrap();
        self.chain
            .record_checked_outputs(&unchecked, &[])
            .await
            .unwrap();
        self.chain.drop_unconfirmed_outputs().await.unwrap();
    }

    async fn available_leaves(&self) -> Vec<TreeNode> {
        let available = self.available().await;
        let leaves = self.store.leaves().await.unwrap();
        leaves
            .into_iter()
            .filter(|leaf| available.contains(&leaf.id))
            .collect()
    }

    /// The store works out which leaves are available leaf by leaf when it
    /// reserves, and from the spent outputs down when it counts the pool. This
    /// function checks that the store finds the same number both ways.
    async fn available(&self) -> Vec<TreeNodeId> {
        let all = self.store.node_ids(None, usize::MAX).await.unwrap();
        let available = self.store.available_among(&all).await.unwrap();
        let counted: u32 = self.store.counts().await.unwrap().available.values().sum();
        assert_eq!(counted as usize, available.len());
        available
    }

    async fn disputes(&self) -> Vec<Dispute> {
        let disputes = self.store.all_disputes().await.unwrap();
        disputes.into_iter().map(|d| d.dispute).collect()
    }

    async fn unspent_outputs(&self) -> Vec<ConfirmedOutput> {
        self.chain.confirmed_unspent_outputs().await.unwrap()
    }

    async fn confirm_spend(&self, spent: OutPoint, spender: Txid) -> BlockHash {
        self.confirm_spends(&[(spent, spender)]).await
    }

    async fn confirm_spends(&self, spends: &[(OutPoint, Txid)]) -> BlockHash {
        let block = next_block();
        let inputs: Vec<SpentTxo> = spends
            .iter()
            .map(|(spent, spender)| SpentTxo {
                outpoint: *spent,
                spending_tx: *spender,
                spending_input_index: 0,
            })
            .collect();
        self.chain.add_block(&block, &[], &inputs).await.unwrap();
        block.hash
    }

    async fn height(&self, block: BlockHash) -> u64 {
        let height: i64 =
            sqlx::query_scalar("SELECT height FROM brz_ssp_chain_blocks WHERE block_hash = $1")
                .bind(block.to_string())
                .fetch_one(&*self.pool)
                .await
                .unwrap();
        height.cast_unsigned()
    }

    fn chain(&self) -> &ChainRepository {
        &self.chain
    }
}

fn key(byte: u8) -> PublicKey {
    SecretKey::from_slice(&[byte; 32])
        .unwrap()
        .public_key(&Secp256k1::new())
}

pub(crate) fn ssp() -> PublicKey {
    key(1)
}

fn next_block() -> BlockHeader {
    static NEXT: AtomicU8 = AtomicU8::new(1);
    let height = NEXT.fetch_add(1, Ordering::Relaxed);
    BlockHeader {
        hash: BlockHash::from_byte_array([height; 32]),
        height: u64::from(height),
    }
}

fn funding() -> OutPoint {
    static NEXT: AtomicU8 = AtomicU8::new(1);
    OutPoint::new(
        Txid::from_byte_array([NEXT.fetch_add(1, Ordering::Relaxed); 32]),
        0,
    )
}

fn spend(previous: OutPoint, timelock: u16, values: &[u64]) -> Transaction {
    Transaction {
        version: Version::non_standard(3),
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: previous,
            sequence: Sequence::from_height(timelock),
            witness: Witness::from_slice(&[[7u8; 64]]),
            ..Default::default()
        }],
        output: values
            .iter()
            .map(|value| TxOut {
                value: Amount::from_sat(*value),
                script_pubkey: ScriptBuf::new_p2tr(
                    &Secp256k1::new(),
                    key(2).x_only_public_key().0,
                    None,
                ),
            })
            .collect(),
    }
}

fn unsigned(mut transaction: Transaction) -> Transaction {
    transaction.input[0].witness = Witness::new();
    transaction
}

fn output(transaction: &Transaction, index: u32) -> OutPoint {
    OutPoint::new(transaction.compute_txid(), index)
}

pub(crate) fn node(
    tree_id: &str,
    parent: Option<&TreeNode>,
    spent: OutPoint,
    values: &[u64],
) -> TreeNode {
    let node_tx = spend(spent, 0, values);
    TreeNode {
        id: Uuid::now_v7().to_string().parse().unwrap(),
        tree_id: tree_id.to_string(),
        value: values.iter().sum(),
        parent_node_id: parent.map(|parent| parent.id.clone()),
        direct_tx: Some(unsigned(spend(spent, 50, values))),
        node_tx,
        refund_tx: None,
        direct_refund_tx: None,
        direct_from_cpfp_refund_tx: None,
        vout: spent.vout,
        verifying_public_key: key(3),
        owner_identity_public_key: None,
        signing_keyshare: SigningKeyshare {
            owner_identifiers: vec![
                Identifier::try_from(1u16).unwrap(),
                Identifier::try_from(2u16).unwrap(),
            ],
            threshold: 2,
            public_key: key(4),
        },
        status: TreeNodeStatus::Splitted,
    }
}

/// A node the operators' watchtower has no transaction for.
fn branch(tree_id: &str, parent: Option<&TreeNode>, spent: OutPoint, values: &[u64]) -> TreeNode {
    TreeNode {
        direct_tx: None,
        ..node(tree_id, parent, spent, values)
    }
}

pub(crate) fn leaf(
    tree_id: &str,
    parent: Option<&TreeNode>,
    spent: OutPoint,
    value: u64,
    refund_timelock: u16,
) -> TreeNode {
    let mut leaf = node(tree_id, parent, spent, &[value]);
    let node_output = output(&leaf.node_tx, 0);
    let direct_output = output(leaf.direct_tx.as_ref().unwrap(), 0);
    leaf.refund_tx = Some(spend(node_output, refund_timelock, &[value]));
    leaf.direct_refund_tx = Some(unsigned(spend(
        direct_output,
        refund_timelock.saturating_add(50),
        &[value],
    )));
    leaf.direct_from_cpfp_refund_tx = Some(unsigned(spend(
        node_output,
        refund_timelock.saturating_add(50),
        &[value],
    )));
    leaf.owner_identity_public_key = Some(ssp());
    leaf.status = TreeNodeStatus::Available;
    leaf
}

fn tree_id() -> String {
    Uuid::now_v7().to_string()
}

fn root_leaf(value: u64) -> TreeNode {
    leaf(&tree_id(), None, funding(), value, 2000)
}

struct Reservation {
    id: TransferId,
    leaves: Vec<TreeNode>,
}

async fn reserve(store: &PostgresLeafStore, denominations: &[u64]) -> Option<Reservation> {
    let id = TransferId::generate();
    let leaves = store.reserve(denominations, &id).await.unwrap()?;
    Some(Reservation { id, leaves })
}

#[tokio::test]
async fn a_leaf_reads_back_as_it_was_stored() {
    let fixture = Fixture::new().await;
    let tree = tree_id();
    let root = node(&tree, None, funding(), &[3000, 5000]);
    let leaf = leaf(&tree, Some(&root), output(&root.node_tx, 1), 5000, 1900);
    let pedigree = LeafPedigree {
        leaf: leaf.clone(),
        ancestors: vec![root.clone()],
    };

    fixture
        .own_pedigrees(std::slice::from_ref(&pedigree))
        .await
        .unwrap();
    fixture.unspent().await;

    assert_eq!(
        fixture.available_leaves().await,
        std::slice::from_ref(&leaf)
    );
    let pedigrees = fixture
        .store
        .pedigrees(std::slice::from_ref(&leaf.id))
        .await
        .unwrap();
    assert_eq!(pedigrees.len(), 1);
    assert_eq!(pedigrees[0].leaf, leaf);
    assert_eq!(pedigrees[0].ancestors, [root]);
}

#[tokio::test]
async fn an_unsigned_transaction_is_not_stored() {
    let fixture = Fixture::new().await;
    let mut leaf = root_leaf(1000);
    leaf.refund_tx = leaf.refund_tx.map(unsigned);

    let stored = fixture.own(&[leaf]).await;

    assert!(stored.is_err());
    assert_eq!(fixture.count("brz_ssp_spark_txs").await, 0);
    assert_eq!(fixture.count("brz_ssp_spark_leaves").await, 0);
}

#[tokio::test]
async fn a_leaf_is_available_only_while_the_operators_agree() {
    let fixture = Fixture::new().await;
    let leaf = root_leaf(1000);
    let pedigree = LeafPedigree {
        leaf: leaf.clone(),
        ancestors: Vec::new(),
    };
    let mut db = fixture.pool.acquire().await.unwrap();
    fixture
        .store
        .insert_leaves(&mut db, std::slice::from_ref(&pedigree))
        .await
        .unwrap();
    fixture.unspent().await;
    assert!(fixture.available().await.is_empty(), "nothing observed yet");

    let now = Utc::now();
    let observe = async |leaf: &TreeNode, seconds: i64| {
        let mut db = fixture.pool.acquire().await.unwrap();
        fixture
            .store
            .observe(
                &mut db,
                std::slice::from_ref(leaf),
                now + Duration::seconds(seconds),
            )
            .await
            .unwrap();
    };

    observe(&leaf, 1).await;
    assert_eq!(fixture.available().await, std::slice::from_ref(&leaf.id));

    let mut locked = leaf.clone();
    locked.status = TreeNodeStatus::TransferLocked;
    observe(&locked, 2).await;
    assert!(
        fixture.available().await.is_empty(),
        "locked by the operators"
    );

    observe(&leaf, 1).await;
    assert!(fixture.available().await.is_empty(), "an older observation");

    let mut another_owner = leaf.clone();
    another_owner.owner_identity_public_key = Some(key(9));
    observe(&another_owner, 3).await;
    assert!(fixture.available().await.is_empty(), "another owner");

    let mut another_refund = leaf.clone();
    another_refund.refund_tx = Some(spend(output(&leaf.node_tx, 0), 1900, &[1000]));
    observe(&another_refund, 4).await;
    assert!(fixture.available().await.is_empty(), "another refund");

    observe(&leaf, 5).await;
    assert_eq!(fixture.available().await, [leaf.id]);
}

#[tokio::test]
async fn a_report_replaces_what_the_operators_reported_before_the_query() {
    let fixture = Fixture::new().await;
    let (exited, sent, kept) = (root_leaf(1000), root_leaf(1000), root_leaf(1000));
    let leaves = [exited.clone(), sent.clone(), kept.clone()];
    fixture.own(&leaves).await.unwrap();
    let queried: Vec<TreeNodeId> = leaves.iter().map(|leaf| leaf.id.clone()).collect();

    let mut exited_now = exited.clone();
    exited_now.status = TreeNodeStatus::ParentExited;
    fixture
        .store
        .operators_reported(
            &queried,
            &[exited_now, kept.clone()],
            Utc::now() + Duration::seconds(10),
        )
        .await
        .unwrap();

    assert_eq!(fixture.available().await, std::slice::from_ref(&kept.id));
    let mut disputes = fixture.disputes().await;
    disputes.sort_by_key(|dispute| dispute.node_id != exited.id);
    assert_eq!(
        disputes,
        [
            Dispute {
                node_id: exited.id,
                reported: Reported::Status("ParentExited".to_string()),
            },
            Dispute {
                node_id: sent.id,
                reported: Reported::Unreported,
            },
        ]
    );
}

#[tokio::test]
async fn a_report_rewrites_only_what_changed() {
    let fixture = Fixture::new().await;
    let (same, locked) = (root_leaf(1000), root_leaf(1000));
    fixture.own(&[same.clone(), locked.clone()]).await.unwrap();
    let written_at = async |leaf: &TreeNode| -> chrono::DateTime<Utc> {
        sqlx::query_scalar(
            "SELECT observed_at FROM brz_ssp_spark_operator_nodes WHERE node_id = $1",
        )
        .bind(Uuid::parse_str(&leaf.id.to_string()).unwrap())
        .fetch_one(&*fixture.pool)
        .await
        .unwrap()
    };
    let (same_before, locked_before) = (written_at(&same).await, written_at(&locked).await);

    let mut locked_now = locked.clone();
    locked_now.status = TreeNodeStatus::TransferLocked;
    fixture
        .store
        .operators_reported(
            &[same.id.clone(), locked.id.clone()],
            &[same.clone(), locked_now],
            Utc::now() + Duration::seconds(10),
        )
        .await
        .unwrap();

    assert_eq!(written_at(&same).await, same_before);
    assert!(written_at(&locked).await > locked_before);
}

#[tokio::test]
async fn a_report_skips_a_leaf_removed_while_it_is_written() {
    let fixture = Fixture::new().await;
    let leaf = root_leaf(1000);
    let pedigree = LeafPedigree {
        leaf: leaf.clone(),
        ancestors: Vec::new(),
    };
    let mut db = fixture.pool.acquire().await.unwrap();
    fixture
        .store
        .insert_leaves(&mut db, std::slice::from_ref(&pedigree))
        .await
        .unwrap();

    let mut removing = fixture.pool.begin().await.unwrap();
    sqlx::query("DELETE FROM brz_ssp_spark_leaves")
        .execute(&mut *removing)
        .await
        .unwrap();
    let store = Arc::clone(&fixture.store);
    let (queried, reported) = (vec![leaf.id.clone()], vec![leaf]);
    let writing = tokio::spawn(async move {
        store
            .operators_reported(&queried, &reported, Utc::now())
            .await
    });
    // The test sleeps while Postgres makes the write of the report wait for the
    // removal.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    removing.commit().await.unwrap();

    writing.await.unwrap().unwrap();
    assert_eq!(fixture.count("brz_ssp_spark_operator_nodes").await, 0);
}

#[tokio::test]
async fn a_report_leaves_the_leaves_free_to_be_reserved_and_removed() {
    let fixture = Fixture::new().await;
    let (kept, sent) = (root_leaf(1000), root_leaf(2000));
    fixture.own(&[kept.clone(), sent.clone()]).await.unwrap();
    let reservation = reserve(&fixture.store, &[2000]).await.unwrap();

    let mut writing = fixture.pool.begin().await.unwrap();
    let mut locked = kept.clone();
    locked.status = TreeNodeStatus::TransferLocked;
    super::rows::observe(
        &mut writing,
        &[locked, sent.clone()],
        &[],
        Utc::now() + Duration::seconds(10),
        false,
    )
    .await
    .unwrap();

    fixture.store.transferred(&reservation.id).await.unwrap();
    assert_eq!(
        reserve(&fixture.store, &[1000]).await.unwrap().leaves,
        [kept]
    );
    writing.commit().await.unwrap();
}

#[tokio::test]
async fn a_report_does_not_undo_what_the_operators_reported_after_the_query() {
    let fixture = Fixture::new().await;
    let (reported, unreported) = (root_leaf(1000), root_leaf(1000));
    let queried_at = Utc::now() - Duration::seconds(10);
    fixture
        .own(&[reported.clone(), unreported.clone()])
        .await
        .unwrap();

    let mut locked = reported.clone();
    locked.status = TreeNodeStatus::TransferLocked;
    fixture
        .store
        .operators_reported(
            &[reported.id.clone(), unreported.id.clone()],
            &[locked],
            queried_at,
        )
        .await
        .unwrap();

    assert_eq!(fixture.available().await.len(), 2);
    assert!(fixture.disputes().await.is_empty());
}

#[tokio::test]
async fn a_reserved_leaf_is_not_in_dispute() {
    let fixture = Fixture::new().await;
    let leaf = root_leaf(1000);
    fixture.own(std::slice::from_ref(&leaf)).await.unwrap();
    let Some(_) = reserve(&fixture.store, &[1000]).await else {
        panic!("the pool has the leaf");
    };

    let mut locked = leaf.clone();
    locked.status = TreeNodeStatus::TransferLocked;
    fixture
        .store
        .operators_reported(
            std::slice::from_ref(&leaf.id),
            &[locked],
            Utc::now() + Duration::seconds(10),
        )
        .await
        .unwrap();

    assert!(fixture.disputes().await.is_empty());
}

#[tokio::test]
async fn what_the_operators_reported_is_kept_only_for_the_ssps_leaves() {
    let fixture = Fixture::new().await;
    let (leaf, not_the_ssps) = (root_leaf(1000), root_leaf(1000));
    fixture.own(std::slice::from_ref(&leaf)).await.unwrap();
    fixture
        .store
        .operators_reported(&[], &[not_the_ssps], Utc::now() + Duration::seconds(10))
        .await
        .unwrap();
    assert_eq!(fixture.count("brz_ssp_spark_operator_nodes").await, 1);

    let Some(reservation) = reserve(&fixture.store, &[1000]).await else {
        panic!("the pool has the leaf");
    };
    fixture.store.transferred(&reservation.id).await.unwrap();

    assert_eq!(fixture.count("brz_ssp_spark_operator_nodes").await, 0);
}

#[tokio::test]
async fn the_store_reserves_a_leaf_of_each_denomination_or_none() {
    let fixture = Fixture::new().await;
    let leaves = [root_leaf(1000), root_leaf(1000), root_leaf(2000)];
    fixture.own(&leaves).await.unwrap();

    let Some(reservation) = reserve(&fixture.store, &[1000, 2000]).await else {
        panic!("the pool has both denominations");
    };
    let mut values: Vec<u64> = reservation.leaves.iter().map(|leaf| leaf.value).collect();
    values.sort_unstable();
    assert_eq!(values, [1000, 2000]);

    assert!(reserve(&fixture.store, &[1000, 2000]).await.is_none());
    assert_eq!(fixture.available().await.len(), 1);

    assert_eq!(fixture.count("brz_ssp_spark_reservations").await, 2);
}

#[tokio::test]
async fn the_store_reserves_the_leaf_with_the_lowest_refund_timelock() {
    let fixture = Fixture::new().await;
    let never_transferred = root_leaf(1000);
    let transferred_before = leaf(&tree_id(), None, funding(), 1000, 1800);
    let transferred_more_often = leaf(&tree_id(), None, funding(), 1000, 1600);
    fixture
        .own(&[
            never_transferred.clone(),
            transferred_more_often.clone(),
            transferred_before.clone(),
        ])
        .await
        .unwrap();

    for expected in [
        transferred_more_often,
        transferred_before,
        never_transferred,
    ] {
        let Some(reservation) = reserve(&fixture.store, &[1000]).await else {
            panic!("the pool has a leaf");
        };
        assert_eq!(reservation.leaves, [expected]);
    }
}

#[tokio::test]
async fn the_store_reserves_no_more_for_a_transfer_that_has_leaves_reserved() {
    let fixture = Fixture::new().await;
    let leaves = [root_leaf(1000), root_leaf(1000)];
    fixture.own(&leaves).await.unwrap();
    let transfer = TransferId::generate();

    let first = fixture.store.reserve(&[1000], &transfer).await.unwrap();
    let again = fixture.store.reserve(&[1000], &transfer).await.unwrap();

    assert_eq!(first, again);
    assert_eq!(fixture.store.reserved(&transfer).await.unwrap().len(), 1);
    assert_eq!(fixture.available().await.len(), 1);
}

#[tokio::test]
async fn available_leaves_are_counted_by_value() {
    let fixture = Fixture::new().await;
    let leaves = [root_leaf(1000), root_leaf(1000), root_leaf(2000)];
    fixture.own(&leaves).await.unwrap();
    reserve(&fixture.store, &[1000]).await.unwrap();

    let counts = fixture.store.counts().await.unwrap();

    assert_eq!(
        counts.available,
        [(1000, 1), (2000, 1)].into_iter().collect()
    );
    assert!(counts.unchecked.is_empty());
}

#[tokio::test]
async fn two_reservations_never_have_the_same_leaf() {
    let fixture = Fixture::new().await;
    let leaves: Vec<TreeNode> = (0..20).map(|_| root_leaf(1000)).collect();
    fixture.own(&leaves).await.unwrap();

    // The test collects the attempts so that it spawns all of them before it
    // waits for any of them.
    let attempts: Vec<_> = (0..30)
        .map(|_| {
            let store = Arc::clone(&fixture.store);
            tokio::spawn(async move { reserve(&store, &[1000]).await })
        })
        .collect();
    let mut reserved = Vec::new();
    for attempt in attempts {
        if let Some(reservation) = attempt.await.unwrap() {
            reserved.extend(reservation.leaves.into_iter().map(|leaf| leaf.id));
        }
    }

    assert_eq!(reserved.len(), 20, "every leaf was reserved");
    let distinct: HashSet<&TreeNodeId> = reserved.iter().collect();
    assert_eq!(distinct.len(), 20, "and none of them twice");
}

#[tokio::test]
async fn a_released_leaf_is_available_again() {
    let fixture = Fixture::new().await;
    let leaf = root_leaf(1000);
    fixture.own(std::slice::from_ref(&leaf)).await.unwrap();
    let Some(reservation) = reserve(&fixture.store, &[1000]).await else {
        panic!("the pool has the leaf");
    };
    assert!(fixture.available().await.is_empty());

    fixture.store.release(&reservation.id).await.unwrap();

    assert_eq!(fixture.available().await, [leaf.id]);
}

#[tokio::test]
async fn a_transferred_leaf_is_no_longer_the_ssps_and_keeps_its_transactions() {
    let fixture = Fixture::new().await;
    let sent = root_leaf(1000);
    let kept = root_leaf(2000);
    fixture.own(&[sent.clone(), kept.clone()]).await.unwrap();
    let transactions = fixture.count("brz_ssp_spark_txs").await;
    let Some(reservation) = reserve(&fixture.store, &[1000]).await else {
        panic!("the pool has the leaf");
    };

    fixture.store.transferred(&reservation.id).await.unwrap();

    assert_eq!(fixture.store.node_ids(None, 100).await.unwrap(), [kept.id]);
    assert_eq!(fixture.count("brz_ssp_spark_reservations").await, 0);
    assert_eq!(fixture.count("brz_ssp_spark_txs").await, transactions);
}

#[tokio::test]
async fn the_ssp_cannot_store_a_leaf_it_transferred_with_its_old_refund() {
    let fixture = Fixture::new().await;
    let leaf = root_leaf(1000);
    fixture.own(std::slice::from_ref(&leaf)).await.unwrap();
    let reservation = reserve(&fixture.store, &[1000]).await.unwrap();
    fixture.store.transferred(&reservation.id).await.unwrap();

    let mut db = fixture.pool.begin().await.unwrap();
    let stored = fixture
        .store
        .insert_leaves(
            &mut db,
            &[LeafPedigree {
                leaf: leaf.clone(),
                ancestors: Vec::new(),
            }],
        )
        .await;
    drop(db);

    assert!(matches!(stored, Err(LeafStoreError::Transferred(id)) if id == leaf.id));
    assert!(fixture.store.node_ids(None, 100).await.unwrap().is_empty());
}

#[tokio::test]
async fn the_store_keeps_its_record_of_a_leaf_over_a_different_one() {
    let fixture = Fixture::new().await;
    let leaf = root_leaf(1000);
    fixture.own(std::slice::from_ref(&leaf)).await.unwrap();
    let mut newer = leaf.clone();
    newer.refund_tx = Some(spend(output(&leaf.node_tx, 0), 1900, &[1000]));

    let mut db = fixture.pool.begin().await.unwrap();
    let stored = fixture
        .store
        .insert_leaves(
            &mut db,
            &[LeafPedigree {
                leaf: newer,
                ancestors: Vec::new(),
            }],
        )
        .await;
    drop(db);

    assert!(matches!(stored, Err(LeafStoreError::DifferentRecord(id)) if id == leaf.id));
    assert_eq!(fixture.available_leaves().await, [leaf]);
}

#[tokio::test]
async fn a_leaf_whose_parent_is_not_stored_is_listed() {
    let fixture = Fixture::new().await;
    let tree = tree_id();
    let root = node(&tree, None, funding(), &[1000, 1000]);
    let below = leaf(&tree, Some(&root), output(&root.node_tx, 0), 1000, 2000);
    let alone = root_leaf(1000);
    fixture.own(&[below.clone(), alone]).await.unwrap();

    assert_eq!(
        fixture.store.leaves_missing_exit_chains().await.unwrap(),
        std::slice::from_ref(&below.id)
    );

    fixture
        .store_ancestors(&[LeafPedigree {
            leaf: below,
            ancestors: vec![root],
        }])
        .await;
    assert!(
        fixture
            .store
            .leaves_missing_exit_chains()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_leaf_whose_parent_confirmed_is_not_available() {
    let fixture = Fixture::new().await;
    let tree = tree_id();
    let funding = funding();
    let root = node(&tree, None, funding, &[1000, 1000]);
    let below = leaf(&tree, Some(&root), output(&root.node_tx, 0), 1000, 2000);
    let elsewhere = root_leaf(1000);
    let pedigrees = [
        LeafPedigree {
            leaf: below.clone(),
            ancestors: vec![root.clone()],
        },
        alone(&elsewhere),
    ];
    fixture.own_pedigrees(&pedigrees).await.unwrap();
    fixture.unspent().await;

    let block = fixture
        .confirm_spend(funding, root.node_tx.compute_txid())
        .await;

    assert_eq!(
        fixture.available().await,
        std::slice::from_ref(&elsewhere.id)
    );
    let disputes = fixture.store.all_disputes().await.unwrap();
    assert_eq!(disputes.len(), 1);
    assert_eq!(
        disputes[0].dispute,
        Dispute {
            node_id: below.id.clone(),
            reported: Reported::Available,
        }
    );
    assert_eq!(
        disputes[0].exiting,
        Some(Spender {
            outpoint: funding,
            txid: root.node_tx.compute_txid(),
            block_height: fixture.height(block).await,
        })
    );
    assert!(reserve(&fixture.store, &[1000, 1000]).await.is_none());

    fixture.chain().undo_block(block).await.unwrap();
    assert_eq!(
        fixture.available().await.len(),
        2,
        "the block left the chain"
    );
}

#[tokio::test]
async fn a_leaf_stays_available_when_a_transaction_above_its_parent_confirms() {
    let fixture = Fixture::new().await;
    let tree = tree_id();
    let funding = funding();
    let root = branch(&tree, None, funding, &[2000]);
    let parent = branch(&tree, Some(&root), output(&root.node_tx, 0), &[1000, 1000]);
    let leaf = leaf(&tree, Some(&parent), output(&parent.node_tx, 0), 1000, 2000);
    let pedigree = LeafPedigree {
        leaf: leaf.clone(),
        ancestors: vec![parent.clone(), root.clone()],
    };
    fixture.own_pedigrees(&[pedigree]).await.unwrap();
    fixture.unspent().await;

    fixture
        .confirm_spend(funding, root.node_tx.compute_txid())
        .await;
    assert_eq!(fixture.available().await, std::slice::from_ref(&leaf.id));
    assert!(fixture.disputes().await.is_empty());

    fixture
        .confirm_spend(output(&root.node_tx, 0), parent.node_tx.compute_txid())
        .await;
    assert!(fixture.available().await.is_empty());
}

#[tokio::test]
async fn a_leaf_is_not_available_once_the_watchtower_can_broadcast_above_it() {
    let fixture = Fixture::new().await;
    let tree = tree_id();
    let funding = funding();
    let root = branch(&tree, None, funding, &[2000]);
    // The operators put a node like this above a leaf in a renewal. The
    // watchtower broadcasts its direct transaction once that transaction is
    // mature.
    let split_node = node(&tree, Some(&root), output(&root.node_tx, 0), &[2000]);
    let split_output = output(&split_node.node_tx, 0);
    let parent = branch(&tree, Some(&split_node), split_output, &[1000, 1000]);
    let leaf = leaf(&tree, Some(&parent), output(&parent.node_tx, 0), 1000, 2000);
    let pedigree = LeafPedigree {
        leaf: leaf.clone(),
        ancestors: vec![parent.clone(), split_node.clone(), root.clone()],
    };
    fixture
        .own_pedigrees(std::slice::from_ref(&pedigree))
        .await
        .unwrap();
    let below_the_root: Vec<OutPoint> = exit_chain(&pedigree)
        .into_iter()
        .map(|step| step.spends)
        .filter(|output| *output != funding)
        .collect();
    fixture
        .chain()
        .record_checked_outputs(&below_the_root, &[])
        .await
        .unwrap();
    assert!(
        fixture.available().await.is_empty(),
        "the SSP has not queried bitcoind for the output the root spends"
    );
    fixture.unspent().await;
    assert_eq!(fixture.available().await, std::slice::from_ref(&leaf.id));

    let block = fixture
        .confirm_spend(funding, root.node_tx.compute_txid())
        .await;
    assert!(fixture.available().await.is_empty());

    fixture.chain().undo_block(block).await.unwrap();
    assert_eq!(
        fixture.available().await,
        [leaf.id],
        "the block left the chain"
    );
}

#[tokio::test]
async fn a_leaf_is_not_available_once_another_transaction_spent_its_node_output() {
    let fixture = Fixture::new().await;
    let leaf = root_leaf(1000);
    fixture.own(std::slice::from_ref(&leaf)).await.unwrap();

    let another = Txid::from_byte_array([0xee; 32]);
    fixture
        .confirm_spend(output(&leaf.node_tx, 0), another)
        .await;

    assert!(fixture.available().await.is_empty());
}

#[tokio::test]
async fn only_spends_of_watched_outputs_are_recorded() {
    let fixture = Fixture::new().await;
    let leaf = root_leaf(1000);
    fixture.own(std::slice::from_ref(&leaf)).await.unwrap();
    let spender = Txid::from_byte_array([0xee; 32]);

    fixture.confirm_spend(funding(), spender).await;
    assert_eq!(fixture.count("brz_ssp_chain_spent_outputs").await, 0);

    // The input of the leaf's direct refund is the direct node transaction's
    // output.
    let direct_output = output(leaf.direct_tx.as_ref().unwrap(), 0);
    fixture.confirm_spend(direct_output, spender).await;
    assert_eq!(fixture.count("brz_ssp_chain_spent_outputs").await, 1);
    assert_eq!(
        fixture.available().await,
        [leaf.id],
        "it is the input of no cpfp transaction"
    );
}

#[tokio::test]
async fn a_claimed_transfer_is_stored_once() {
    let fixture = Fixture::new().await;
    let (claimed, later) = (root_leaf(1000), root_leaf(1000));
    let transfer = TransferId::generate();
    let store = &fixture.store;

    let (claimed_leaves, later_leaves) = ([alone(&claimed)], [alone(&later)]);
    let first = store.claimed(&transfer, &claimed_leaves, Utc::now());
    assert!(first.await.unwrap());
    let again = store.claimed(&transfer, &later_leaves, Utc::now());
    assert!(!again.await.unwrap());

    fixture.unspent().await;
    assert_eq!(fixture.available().await, [claimed.id]);
}

#[tokio::test]
async fn a_leaf_whose_refund_needs_renewal_is_unavailable_until_renewed() {
    let fixture = Fixture::new().await;
    let tree = tree_id();
    let low = leaf(&tree, None, funding(), 1000, 100);
    let high = root_leaf(1000);
    assert!(low.needs_refund_tx_renewed().unwrap());
    assert!(!high.needs_refund_tx_renewed().unwrap());
    fixture.own(&[low.clone(), high.clone()]).await.unwrap();
    let store = &fixture.store;
    let needing_renewal = async || {
        store
            .leaves_needing_renewal(RenewalCursor::default(), 10)
            .await
            .unwrap()
    };

    assert_eq!(fixture.available().await, std::slice::from_ref(&high.id));
    assert!(fixture.disputes().await.is_empty());
    let (leaves, next) = needing_renewal().await;
    assert_eq!(leaves.len(), 1);
    assert_eq!(leaves[0].leaf, low);
    assert!(next.is_none(), "the store has passed every such leaf");

    // In the renewal the operators give the leaf's node transaction to a new node
    // above it.
    let mut above = low.clone();
    above.id = Uuid::now_v7().to_string().parse().unwrap();
    above.refund_tx = None;
    above.direct_refund_tx = None;
    above.direct_from_cpfp_refund_tx = None;
    let mut renewed = leaf(&tree, Some(&above), output(&low.node_tx, 0), 1000, 2000);
    renewed.id = low.id.clone();
    let after = [LeafPedigree {
        leaf: renewed.clone(),
        ancestors: vec![above],
    }];
    let before = std::slice::from_ref(&low);
    store.renewed(before, &after, Utc::now()).await.unwrap();
    fixture.unspent().await;

    assert_eq!(fixture.available().await.len(), 2);
    assert!(needing_renewal().await.0.is_empty());
    assert!(store.leaves_missing_exit_chains().await.unwrap().is_empty());

    let stale = store.renewed(before, &after, Utc::now()).await;
    assert!(matches!(stale, Err(LeafStoreError::ChangedDuringRenewal(id)) if id == low.id));
    assert_eq!(fixture.available().await.len(), 2);
}

#[tokio::test]
async fn a_renewal_pass_goes_on_from_where_it_was() {
    let fixture = Fixture::new().await;
    let tree = tree_id();
    let lows: Vec<TreeNode> = [100, 100, 0]
        .into_iter()
        .map(|timelock| leaf(&tree, None, funding(), 1000, timelock))
        .collect();
    fixture.own(&lows).await.unwrap();
    // The operators renewed one of the leaves, but the SSP did not record the
    // renewal. The store still lists that leaf for renewal, so the SSP can
    // request the renewal again and get the renewed leaf back.
    let mut renewed_there = lows[0].clone();
    renewed_there.refund_tx = Some(spend(output(&lows[0].node_tx, 0), 2000, &[1000]));
    let mut db = fixture.pool.acquire().await.unwrap();
    let later = Utc::now() + Duration::seconds(10);
    fixture
        .store
        .observe(&mut db, &[renewed_there], later)
        .await
        .unwrap();

    let mut cursor = RenewalCursor::default();
    let mut batches: Vec<Vec<TreeNodeId>> = Vec::new();
    loop {
        let (leaves, next) = fixture
            .store
            .leaves_needing_renewal(cursor, 2)
            .await
            .unwrap();
        batches.push(
            leaves
                .into_iter()
                .map(|pedigree| pedigree.leaf.id)
                .collect(),
        );
        match next {
            Some(next) => cursor = next,
            None => break,
        }
    }

    assert_eq!(batches.len(), 2);
    assert!(
        batches[0].contains(&lows[2].id),
        "the lowest timelock first"
    );
    let seen: HashSet<&TreeNodeId> = batches.iter().flatten().collect();
    assert_eq!(seen, lows.iter().map(|leaf| &leaf.id).collect());
}

#[tokio::test]
async fn a_leaf_whose_exit_chain_does_not_reach_the_root_is_listed() {
    let fixture = Fixture::new().await;
    let tree = tree_id();
    let root = node(&tree, None, funding(), &[2000]);
    let middle = node(&tree, Some(&root), output(&root.node_tx, 0), &[1000, 1000]);
    let below = leaf(&tree, Some(&middle), output(&middle.node_tx, 0), 1000, 2000);
    let store = &fixture.store;
    fixture.own(std::slice::from_ref(&below)).await.unwrap();
    let with = |ancestors: Vec<TreeNode>| LeafPedigree {
        leaf: below.clone(),
        ancestors,
    };

    fixture.store_ancestors(&[with(vec![middle])]).await;
    assert_eq!(
        store.leaves_missing_exit_chains().await.unwrap(),
        std::slice::from_ref(&below.id)
    );

    fixture.store_ancestors(&[with(vec![root])]).await;
    assert!(store.leaves_missing_exit_chains().await.unwrap().is_empty());
}

#[tokio::test]
async fn the_transaction_below_a_confirmed_one_is_mature_for_the_leaf_the_ssp_exits_unilaterally() {
    let fixture = Fixture::new().await;
    let tree = tree_id();
    let funding = funding();
    let root = node(&tree, None, funding, &[1000, 1000]);
    let below = leaf(&tree, Some(&root), output(&root.node_tx, 0), 1000, 1900);
    let refund = below.refund_tx.clone().unwrap();
    let pedigrees = [
        LeafPedigree {
            leaf: below.clone(),
            ancestors: vec![root.clone()],
        },
        alone(&root_leaf(1000)),
    ];
    fixture.own_pedigrees(&pedigrees).await.unwrap();
    fixture.unspent().await;
    let store = &fixture.store;
    assert!(fixture.unspent_outputs().await.is_empty());

    let root_block = fixture
        .confirm_spend(funding, root.node_tx.compute_txid())
        .await;
    let height = fixture.height(root_block).await;

    let unspent = fixture.unspent_outputs().await;
    let mature = store.mature_transactions(&unspent, height).await.unwrap();
    assert_eq!(mature.len(), 1);
    assert_eq!(mature[0].transaction, below.node_tx);
    assert_eq!(mature[0].timelock, 0);
    assert_eq!(mature[0].input_confirmation_height, height);
    assert_eq!(mature[0].direct_timelock, Some(50));
    assert_eq!(mature[0].leaves, std::slice::from_ref(&below.id));
    assert_eq!(mature[0].value, 1000);
    assert_eq!(mature[0].remaining, [below.node_tx.clone(), refund.clone()]);

    // The node transaction confirms. Its refund can be in a block 1900 blocks
    // higher, so the refund is mature once the tip is 1899 blocks higher.
    let node_block = fixture
        .confirm_spend(output(&root.node_tx, 0), below.node_tx.compute_txid())
        .await;
    let height = fixture.height(node_block).await;
    let unspent = fixture.unspent_outputs().await;
    let early = store.mature_transactions(&unspent, height + 1898).await;
    assert!(early.unwrap().is_empty());
    let mature = store
        .mature_transactions(&unspent, height + 1899)
        .await
        .unwrap();
    assert_eq!(mature.len(), 1);
    assert_eq!(mature[0].transaction, refund);
    assert_eq!(mature[0].timelock, 1900);
    assert_eq!(mature[0].direct_timelock, Some(1950));
    assert_eq!(mature[0].remaining, std::slice::from_ref(&refund));

    fixture
        .confirm_spend(output(&below.node_tx, 0), refund.compute_txid())
        .await;
    let unspent = fixture.unspent_outputs().await;
    let mature = store.mature_transactions(&unspent, height + 5000).await;
    assert!(mature.unwrap().is_empty());
}

#[tokio::test]
async fn nothing_to_broadcast_for_a_leaf_while_only_a_transaction_above_its_parent_confirmed() {
    let fixture = Fixture::new().await;
    let tree = tree_id();
    let funding = funding();
    let root = branch(&tree, None, funding, &[2000]);
    let parent = branch(&tree, Some(&root), output(&root.node_tx, 0), &[1000, 1000]);
    let leaf = leaf(&tree, Some(&parent), output(&parent.node_tx, 0), 1000, 2000);
    let pedigree = LeafPedigree {
        leaf: leaf.clone(),
        ancestors: vec![parent.clone(), root.clone()],
    };
    fixture.own_pedigrees(&[pedigree]).await.unwrap();
    fixture.unspent().await;
    let store = &fixture.store;

    let block = fixture
        .confirm_spend(funding, root.node_tx.compute_txid())
        .await;
    let height = fixture.height(block).await;
    // The parent's node transaction is mature, but the SSP does not broadcast
    // it: the leaf stays available until that transaction confirms.
    let unspent = fixture.unspent_outputs().await;
    let mature = store.mature_transactions(&unspent, height).await;
    assert!(mature.unwrap().is_empty());

    let block = fixture
        .confirm_spend(output(&root.node_tx, 0), parent.node_tx.compute_txid())
        .await;
    let height = fixture.height(block).await;
    let unspent = fixture.unspent_outputs().await;
    let mature = store.mature_transactions(&unspent, height).await.unwrap();
    assert_eq!(mature.len(), 1);
    assert_eq!(mature[0].transaction, leaf.node_tx);
    assert_eq!(mature[0].leaves, [leaf.id]);
}

#[tokio::test]
async fn every_transaction_down_to_the_refund_is_listed_below_a_node_with_a_direct_transaction() {
    let fixture = Fixture::new().await;
    let tree = tree_id();
    let funding = funding();
    let root = branch(&tree, None, funding, &[2000]);
    // The operators put a node like this above a leaf in a renewal. The
    // watchtower broadcasts its direct transaction once that transaction is
    // mature.
    let split_node = node(&tree, Some(&root), output(&root.node_tx, 0), &[2000]);
    let split_output = output(&split_node.node_tx, 0);
    let parent = branch(&tree, Some(&split_node), split_output, &[1000, 1000]);
    let leaf = leaf(&tree, Some(&parent), output(&parent.node_tx, 0), 1000, 2000);
    let pedigree = LeafPedigree {
        leaf: leaf.clone(),
        ancestors: vec![parent.clone(), split_node.clone(), root.clone()],
    };
    fixture.own_pedigrees(&[pedigree]).await.unwrap();
    fixture.unspent().await;

    let block = fixture
        .confirm_spend(funding, root.node_tx.compute_txid())
        .await;
    let height = fixture.height(block).await;

    let unspent = fixture.unspent_outputs().await;
    let mature = fixture
        .store
        .mature_transactions(&unspent, height)
        .await
        .unwrap();
    assert_eq!(mature.len(), 1);
    assert_eq!(mature[0].transaction, split_node.node_tx);
    assert_eq!(mature[0].direct_timelock, Some(50));
    assert_eq!(mature[0].input_confirmation_height, height);
    assert_eq!(mature[0].leaves, std::slice::from_ref(&leaf.id));
    assert_eq!(mature[0].value, 1000);
    assert_eq!(
        mature[0].remaining,
        [
            split_node.node_tx.clone(),
            parent.node_tx.clone(),
            leaf.node_tx.clone(),
            leaf.refund_tx.clone().unwrap(),
        ]
    );
}

#[tokio::test]
async fn nothing_to_broadcast_below_the_watchtowers_direct_transaction() {
    let fixture = Fixture::new().await;
    let tree = tree_id();
    let funding = funding();
    let root = node(&tree, None, funding, &[1000, 1000]);
    let below = leaf(&tree, Some(&root), output(&root.node_tx, 0), 1000, 1900);
    let pedigree = LeafPedigree {
        leaf: below.clone(),
        ancestors: vec![root.clone()],
    };
    fixture.own_pedigrees(&[pedigree]).await.unwrap();
    fixture.unspent().await;
    fixture
        .confirm_spend(funding, root.node_tx.compute_txid())
        .await;

    // The watchtower's direct transaction confirms instead of the leaf's node
    // transaction, which the SSP has a signature for.
    let direct = below.direct_tx.as_ref().unwrap().compute_txid();
    let block = fixture
        .confirm_spend(output(&root.node_tx, 0), direct)
        .await;
    let height = fixture.height(block).await;

    let unspent = fixture.unspent_outputs().await;
    let mature = fixture
        .store
        .mature_transactions(&unspent, height + 5000)
        .await;
    assert!(mature.unwrap().is_empty());
    assert!(fixture.available().await.is_empty());
}

#[tokio::test]
async fn an_old_refund_is_mature_once_an_old_node_transaction_confirmed() {
    let fixture = Fixture::new().await;
    let tree = tree_id();
    let funding = funding();
    let root = branch(&tree, None, funding, &[1000]);
    let root_output = output(&root.node_tx, 0);
    // This is the leaf as it was before a renewal.
    let mut old = leaf(&tree, Some(&root), root_output, 1000, 100);
    old.node_tx = spend(root_output, 1900, &[1000]);
    old.direct_tx = None;
    let old_refund = spend(output(&old.node_tx, 0), 100, &[1000]);
    old.refund_tx = Some(old_refund.clone());
    old.direct_refund_tx = None;
    old.direct_from_cpfp_refund_tx = None;
    fixture
        .own_pedigrees(&[LeafPedigree {
            leaf: old.clone(),
            ancestors: vec![root.clone()],
        }])
        .await
        .unwrap();
    // In the renewal the operators lower the node transaction's timelock and
    // reset the refund's timelock.
    let mut renewed = old.clone();
    renewed.node_tx = spend(root_output, 1800, &[1000]);
    renewed.refund_tx = Some(spend(output(&renewed.node_tx, 0), 2000, &[1000]));
    let after = [LeafPedigree {
        leaf: renewed.clone(),
        ancestors: vec![root.clone()],
    }];
    let store = &fixture.store;
    store
        .renewed(std::slice::from_ref(&old), &after, Utc::now())
        .await
        .unwrap();
    fixture.unspent().await;

    let block = fixture
        .confirm_spends(&[
            (funding, root.node_tx.compute_txid()),
            // The old node transaction confirms instead of the renewed node
            // transaction.
            (root_output, old.node_tx.compute_txid()),
        ])
        .await;
    let height = fixture.height(block).await;

    let unspent = fixture.unspent_outputs().await;
    assert!(
        store
            .mature_transactions(&unspent, height + 98)
            .await
            .unwrap()
            .is_empty()
    );
    let mature = store
        .mature_transactions(&unspent, height + 99)
        .await
        .unwrap();
    assert_eq!(mature.len(), 1);
    assert_eq!(mature[0].transaction, old_refund);
    assert_eq!(mature[0].leaves, std::slice::from_ref(&renewed.id));
    assert_eq!(mature[0].value, 1000);
    assert_eq!(mature[0].remaining, std::slice::from_ref(&old_refund));
}

#[tokio::test]
async fn the_refund_of_a_leaf_the_ssp_transferred_is_never_to_be_broadcast() {
    let fixture = Fixture::new().await;
    let leaf = root_leaf(1000);
    fixture.own(std::slice::from_ref(&leaf)).await.unwrap();
    let reservation = reserve(&fixture.store, &[1000]).await.unwrap();
    fixture.store.transferred(&reservation.id).await.unwrap();

    let funding = leaf.node_tx.input[0].previous_output;
    let block = fixture
        .confirm_spend(funding, leaf.node_tx.compute_txid())
        .await;
    let height = fixture.height(block).await;

    let unspent = fixture.unspent_outputs().await;
    assert_eq!(unspent.len(), 1, "the node transaction's output");
    let mature = fixture
        .store
        .mature_transactions(&unspent, height + 5000)
        .await;
    assert!(mature.unwrap().is_empty());
}

#[tokio::test]
async fn a_refund_is_mature_below_a_node_transaction_that_confirmed_in_its_parents_block() {
    let fixture = Fixture::new().await;
    let tree = tree_id();
    let root = branch(&tree, None, funding(), &[2000]);
    let parent = branch(&tree, Some(&root), output(&root.node_tx, 0), &[1000, 1000]);
    let leaf = leaf(&tree, Some(&parent), output(&parent.node_tx, 0), 1000, 2000);
    let pedigree = LeafPedigree {
        leaf: leaf.clone(),
        ancestors: vec![parent.clone(), root.clone()],
    };
    fixture.own_pedigrees(&[pedigree]).await.unwrap();
    fixture.unspent().await;

    let block = fixture
        .confirm_spends(&[
            (output(&root.node_tx, 0), parent.node_tx.compute_txid()),
            (output(&parent.node_tx, 0), leaf.node_tx.compute_txid()),
        ])
        .await;
    let height = fixture.height(block).await;

    assert_eq!(fixture.count("brz_ssp_chain_spent_outputs").await, 2);
    let unspent = fixture.unspent_outputs().await;
    let mature = fixture
        .store
        .mature_transactions(&unspent, height + 1999)
        .await
        .unwrap();
    assert_eq!(mature.len(), 1);
    assert_eq!(mature[0].transaction, leaf.refund_tx.clone().unwrap());
    assert_eq!(mature[0].input_confirmation_height, height);
}

#[tokio::test]
async fn an_output_is_queued_when_it_is_first_watched() {
    let fixture = Fixture::new().await;
    let store = &fixture.store;
    let chain = fixture.chain();
    let (leaf, other) = (root_leaf(1000), root_leaf(1000));
    fixture
        .own_unchecked(&[leaf.clone(), other.clone()])
        .await
        .unwrap();
    assert!(
        fixture.available().await.is_empty(),
        "the SSP has not queried bitcoind yet"
    );
    assert!(fixture.disputes().await.is_empty());

    // The chain record queues six outputs for each leaf. Three of them are
    // inputs: the node transaction and its direct transaction have the same
    // input, the refund and the direct refund made from it have the same input,
    // and the other direct refund has the direct node transaction's output as
    // its input. The rest are the first outputs of the leaf's three refunds.
    assert_eq!(fixture.count("brz_ssp_chain_unchecked_outputs").await, 12);
    // Only the input of each node transaction can already be spent. Every other
    // output is on chain only once its own transaction has confirmed.
    let funding = leaf.node_tx.input[0].previous_output;
    let tops: HashSet<OutPoint> = [funding, other.node_tx.input[0].previous_output].into();
    let unchecked = chain.next_unchecked_outputs(100).await.unwrap();
    assert_eq!(unchecked.iter().copied().collect::<HashSet<_>>(), tops);
    assert_eq!(chain.next_unchecked_outputs(1).await.unwrap().len(), 1);
    assert_eq!(chain.drop_unconfirmed_outputs().await.unwrap(), 0);

    let recorded = chain.record_checked_outputs(&[funding], &[]).await;
    assert_eq!(recorded.unwrap(), 0);
    assert_eq!(chain.drop_unconfirmed_outputs().await.unwrap(), 5);
    assert_eq!(fixture.count("brz_ssp_chain_unchecked_outputs").await, 6);
    assert_eq!(fixture.available().await, std::slice::from_ref(&leaf.id));

    // The new refunds have an input that the chain record already watches, so
    // it queues only their first outputs.
    let node_output = output(&leaf.node_tx, 0);
    let mut renewed = leaf.clone();
    renewed.refund_tx = Some(spend(node_output, 1900, &[1000]));
    renewed.direct_from_cpfp_refund_tx = Some(unsigned(spend(node_output, 1950, &[1000])));
    let after = LeafPedigree {
        leaf: renewed,
        ancestors: Vec::new(),
    };
    store.renewed(&[leaf], &[after], Utc::now()).await.unwrap();
    assert_eq!(fixture.count("brz_ssp_chain_unchecked_outputs").await, 8);
}

#[tokio::test]
async fn bitcoind_is_queried_only_for_the_outputs_of_confirmed_transactions() {
    let fixture = Fixture::new().await;
    let chain = fixture.chain();
    let block = next_block();
    chain.add_block(&block, &[], &[]).await.unwrap();
    let tree = tree_id();
    let funding = funding();
    let root = branch(&tree, None, funding, &[2000]);
    let root_output = output(&root.node_tx, 0);
    let parent = branch(&tree, Some(&root), root_output, &[1000, 1000]);
    let leaf = leaf(&tree, Some(&parent), output(&parent.node_tx, 0), 1000, 2000);
    fixture
        .own_pedigrees(&[LeafPedigree {
            leaf: leaf.clone(),
            ancestors: vec![parent, root.clone()],
        }])
        .await
        .unwrap();

    // The chain record queues the inputs of the root, of the parent and of the
    // leaf's node transaction, refund and direct refund. It also queues the
    // first outputs of the leaf's three refunds.
    assert_eq!(fixture.count("brz_ssp_chain_unchecked_outputs").await, 8);
    assert_eq!(chain.next_unchecked_outputs(100).await.unwrap(), [funding]);

    // The root confirmed before the SSP stored it, so its output can be spent too.
    let spends = [OutputSpend {
        outpoint: funding,
        spender: root.node_tx.compute_txid(),
        block: Some(block),
    }];
    let recorded = chain.record_checked_outputs(&[funding], &spends).await;
    assert_eq!(recorded.unwrap(), 1);
    assert_eq!(chain.drop_unconfirmed_outputs().await.unwrap(), 0);
    let next = chain.next_unchecked_outputs(100).await.unwrap();
    assert_eq!(next, [root_output]);
    assert!(fixture.available().await.is_empty());

    // The parent has not confirmed, so no block contains an output below it.
    let recorded = chain.record_checked_outputs(&[root_output], &[]).await;
    assert_eq!(recorded.unwrap(), 0);
    assert_eq!(chain.drop_unconfirmed_outputs().await.unwrap(), 6);
    assert_eq!(fixture.count("brz_ssp_chain_unchecked_outputs").await, 0);
    assert_eq!(fixture.available().await, [leaf.id]);
}

#[tokio::test]
async fn an_unchecked_leaf_is_counted_separately() {
    let fixture = Fixture::new().await;
    let store = &fixture.store;
    fixture.own(&[root_leaf(1000)]).await.unwrap();
    fixture
        .own_unchecked(&[root_leaf(1000), root_leaf(2000)])
        .await
        .unwrap();

    let counts = store.counts().await.unwrap();
    assert_eq!(counts.available, [(1000, 1)].into_iter().collect());
    let unchecked = [(1000, 1), (2000, 1)].into_iter().collect();
    assert_eq!(counts.unchecked, unchecked);

    fixture.unspent().await;
    let counts = store.counts().await.unwrap();
    assert_eq!(
        counts.available,
        [(1000, 2), (2000, 1)].into_iter().collect()
    );
    assert!(counts.unchecked.is_empty());
}

#[tokio::test]
async fn a_spend_bitcoind_reports_is_recorded_with_its_block() {
    let fixture = Fixture::new().await;
    let chain = fixture.chain();
    // The chain monitor reads the block before the SSP stores the leaf, so it
    // records nothing for the leaf.
    let block = next_block();
    chain.add_block(&block, &[], &[]).await.unwrap();
    let leaf = root_leaf(1000);
    fixture
        .own_unchecked(std::slice::from_ref(&leaf))
        .await
        .unwrap();

    let funding = leaf.node_tx.input[0].previous_output;
    let node_output = output(&leaf.node_tx, 0);
    let spends = [
        OutputSpend {
            outpoint: funding,
            spender: leaf.node_tx.compute_txid(),
            block: Some(block.clone()),
        },
        OutputSpend {
            outpoint: node_output,
            spender: leaf.refund_tx.as_ref().unwrap().compute_txid(),
            block: None,
        },
    ];
    let recorded = chain.record_checked_outputs(&[funding], &spends).await;
    assert_eq!(recorded.unwrap(), 1);

    // The node transaction confirmed, so the spend check queries bitcoind for
    // its output next. The direct node transaction can no longer confirm, and
    // neither can the direct refund whose input is that transaction's output.
    assert_eq!(chain.drop_unconfirmed_outputs().await.unwrap(), 2);
    let next = chain.next_unchecked_outputs(100).await.unwrap();
    assert_eq!(next, [node_output]);
    let recorded = chain.record_checked_outputs(&[node_output], &spends).await;
    assert_eq!(recorded.unwrap(), 0);
    // No refund has confirmed, so no block contains their outputs.
    assert_eq!(chain.drop_unconfirmed_outputs().await.unwrap(), 2);
    assert_eq!(fixture.count("brz_ssp_chain_unchecked_outputs").await, 0);
    assert!(fixture.available().await.is_empty());
    let disputes = fixture.store.all_disputes().await.unwrap();
    assert_eq!(disputes.len(), 1);
    assert_eq!(
        disputes[0].exiting,
        Some(Spender {
            outpoint: funding,
            txid: leaf.node_tx.compute_txid(),
            block_height: block.height,
        })
    );

    chain.undo_block(block.hash).await.unwrap();
    assert_eq!(
        fixture.available().await,
        [leaf.id],
        "the block left the chain"
    );
}

#[tokio::test]
async fn a_spend_older_than_the_chain_record_is_recorded_without_a_block() {
    let fixture = Fixture::new().await;
    let chain = fixture.chain();
    let first = next_block();
    chain.add_block(&first, &[], &[]).await.unwrap();
    let old = root_leaf(1000);
    let reorged = root_leaf(1000);
    fixture
        .own_unchecked(&[old.clone(), reorged.clone()])
        .await
        .unwrap();

    let unchecked = chain.next_unchecked_outputs(100).await.unwrap();
    let spends = [
        OutputSpend {
            outpoint: old.node_tx.input[0].previous_output,
            spender: old.node_tx.compute_txid(),
            block: Some(BlockHeader {
                hash: BlockHash::from_byte_array([0xaa; 32]),
                height: 0,
            }),
        },
        // This spend is in a block that has left the chain: the chain record
        // has another block at that height.
        OutputSpend {
            outpoint: reorged.node_tx.input[0].previous_output,
            spender: reorged.node_tx.compute_txid(),
            block: Some(BlockHeader {
                hash: BlockHash::from_byte_array([0xbb; 32]),
                height: first.height,
            }),
        },
    ];
    let recorded = chain.record_checked_outputs(&unchecked, &spends).await;
    assert_eq!(recorded.unwrap(), 1);

    fixture.unspent().await;
    assert_eq!(fixture.available().await, std::slice::from_ref(&reorged.id));
    chain.undo_block(first.hash).await.unwrap();
    assert_eq!(
        fixture.available().await,
        [reorged.id],
        "no reorg undoes it"
    );
}

#[tokio::test]
async fn a_reservation_skips_an_unchecked_leaf() {
    let fixture = Fixture::new().await;
    // The store selects the leaf with the lower refund timelock first, unless
    // the SSP has yet to check that leaf's outputs with bitcoind.
    let transferred_before = leaf(&tree_id(), None, funding(), 1000, 1800);
    let never_transferred = root_leaf(1000);
    fixture
        .own(std::slice::from_ref(&never_transferred))
        .await
        .unwrap();
    fixture
        .own_unchecked(std::slice::from_ref(&transferred_before))
        .await
        .unwrap();

    let reservation = reserve(&fixture.store, &[1000]).await.unwrap();
    assert_eq!(reservation.leaves, [never_transferred]);
    assert!(reserve(&fixture.store, &[1000]).await.is_none());

    fixture.unspent().await;
    let reservation = reserve(&fixture.store, &[1000]).await.unwrap();
    assert_eq!(reservation.leaves, [transferred_before]);
}

#[tokio::test]
async fn a_reservation_skips_many_exiting_leaves() {
    let fixture = Fixture::new().await;
    let exiting: Vec<TreeNode> = (0..9)
        .map(|_| leaf(&tree_id(), None, funding(), 1000, 1800))
        .collect();
    let never_transferred = [root_leaf(1000), root_leaf(1000)];
    fixture.own(&exiting).await.unwrap();
    fixture.own(&never_transferred).await.unwrap();
    // The operators have yet to stop reporting the exiting leaves as AVAILABLE.
    for leaf in &exiting {
        let funding = leaf.node_tx.input[0].previous_output;
        fixture
            .confirm_spend(funding, leaf.node_tx.compute_txid())
            .await;
    }

    let reservation = reserve(&fixture.store, &[1000, 1000]).await.unwrap();

    let mut reserved: Vec<TreeNodeId> = reservation.leaves.into_iter().map(|l| l.id).collect();
    reserved.sort();
    let mut expected: Vec<TreeNodeId> = never_transferred
        .iter()
        .map(|leaf| leaf.id.clone())
        .collect();
    expected.sort();
    assert_eq!(reserved, expected);
    assert!(reserve(&fixture.store, &[1000]).await.is_none());
}

#[tokio::test]
async fn the_transfers_that_have_leaves_reserved_are_listed() {
    let fixture = Fixture::new().await;
    fixture
        .own(&[root_leaf(1000), root_leaf(2000), root_leaf(4000)])
        .await
        .unwrap();
    let first = reserve(&fixture.store, &[1000, 2000]).await.unwrap();
    let second = reserve(&fixture.store, &[4000]).await.unwrap();

    let reservations = fixture.store.reservations().await.unwrap();

    let listed: Vec<(TransferId, u32, u64)> = reservations
        .into_iter()
        .map(|r| (r.transfer_id, r.leaves, r.value))
        .collect();
    assert_eq!(listed, [(first.id, 2, 3000), (second.id, 1, 4000)]);
}

#[tokio::test]
async fn a_node_reported_as_before_is_not_written_again() {
    let fixture = Fixture::new().await;
    let tree = tree_id();
    let root = node(&tree, None, funding(), &[1000, 1000]);
    let first = leaf(&tree, Some(&root), output(&root.node_tx, 0), 1000, 2000);
    let second = leaf(&tree, Some(&root), output(&root.node_tx, 1), 1000, 2000);
    let with_root = |leaf: &TreeNode| LeafPedigree {
        leaf: leaf.clone(),
        ancestors: vec![root.clone()],
    };
    fixture.own_pedigrees(&[with_root(&first)]).await.unwrap();
    let xmin = async || -> String {
        sqlx::query_scalar("SELECT xmin::text FROM brz_ssp_spark_nodes WHERE node_id = $1")
            .bind(Uuid::parse_str(&root.id.to_string()).unwrap())
            .fetch_one(&*fixture.pool)
            .await
            .unwrap()
    };
    let before = xmin().await;

    fixture.own_pedigrees(&[with_root(&second)]).await.unwrap();

    assert_eq!(xmin().await, before);
}
