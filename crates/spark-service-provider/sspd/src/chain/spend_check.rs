//! The chain monitor records the spend of a watched output only from the
//! blocks it reads after sspd started to watch the output. The spend check
//! queries bitcoind for a spend in an earlier block. It skips an output of an
//! unconfirmed transaction, because no block contains such an output.

use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info};

use crate::chain::{ChainClient, ChainTip};
use crate::postgresql::ChainRepository;
use crate::wakeup::Wakeup;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

const CHECK_INTERVAL: Duration = Duration::from_secs(10);

/// The most outputs sspd checks with bitcoind in one call.
pub(crate) const MAX_OUTPUTS_PER_QUERY: u32 = 1000;

pub struct SpendCheckDeps {
    pub chain_repository: Arc<ChainRepository>,
    pub chain_tip: ChainTip,
    pub chain_client: Arc<dyn ChainClient + Send + Sync>,
    pub blocks: Wakeup,
}

pub async fn run_spend_check_loop(deps: SpendCheckDeps, token: CancellationToken) {
    let watched = deps.chain_repository.watched();
    loop {
        if let Err(e) = check_queued(&deps).await {
            error!("could not check newly watched outputs: {e}");
        }
        tokio::select! {
            () = token.cancelled() => return,
            () = deps.blocks.waited() => {}
            () = watched.waited() => {}
            () = tokio::time::sleep(CHECK_INTERVAL) => {}
        }
    }
}

/// The spend check queries bitcoind only while bitcoind's tip is the chain
/// record's tip. bitcoind reports the spends in the blocks up to its own tip,
/// and the chain monitor records the spends in the blocks after the record's
/// tip. With the two tips at the same block, bitcoind and the chain monitor
/// together cover every block.
async fn check_queued(deps: &SpendCheckDeps) -> Result<(), BoxError> {
    loop {
        deps.chain_repository.drop_unconfirmed_outputs().await?;
        let queued = deps
            .chain_repository
            .next_unchecked_outputs(MAX_OUTPUTS_PER_QUERY)
            .await?;
        if queued.is_empty() {
            return Ok(());
        }
        // The spend check needs the exact stored tip. bitcoind reports no spend
        // in a stored block that it has dropped from its chain, and the chain
        // monitor does not read that block again when bitcoind's chain later
        // contains it.
        let Some(tip) = deps.chain_tip.stored() else {
            return Ok(());
        };
        if deps.chain_client.get_best_block_hash().await? != tip.hash {
            debug!("bitcoind and the chain record are at different blocks, checking later");
            return Ok(());
        }
        let spends = deps.chain_client.get_spends(&queued).await?;
        if deps.chain_client.get_best_block_hash().await? != tip.hash {
            debug!("bitcoind changed its tip during the query, checking again later");
            return Ok(());
        }
        let recorded = deps
            .chain_repository
            .record_checked_outputs(&queued, &spends)
            .await?;
        debug!(
            checked = queued.len(),
            recorded, "checked queued outputs with bitcoind"
        );
        if recorded > 0 {
            info!("{recorded} outputs were spent before the SSP watched them");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use bitcoin::hashes::Hash;
    use bitcoin::{Block, BlockHash, FeeRate, Network, OutPoint, Transaction, Txid};
    use sqlx::PgPool;

    use super::*;
    use crate::chain::{
        BlockHeader, BroadcastError, ChainError, ChainRepository as _, OutputSpend, WatchedOutput,
    };
    use crate::postgresql::testing::TestDatabase;

    struct Bitcoind {
        tip: Mutex<BlockHash>,
        next: Option<BlockHash>,
        spends: Vec<OutputSpend>,
        queried: Arc<Mutex<Vec<Vec<OutPoint>>>>,
    }

    #[async_trait::async_trait]
    impl ChainClient for Bitcoind {
        async fn broadcast_tx(&self, _tx: Transaction) -> Result<(), BroadcastError> {
            unimplemented!()
        }

        async fn broadcast_package(&self, _txs: &[Transaction]) -> Result<(), BroadcastError> {
            unimplemented!()
        }

        async fn estimate_fee_rate(&self, _conf_target: u32) -> Result<FeeRate, ChainError> {
            unimplemented!()
        }

        async fn sync_state(&self) -> Result<crate::chain::SyncState, ChainError> {
            unimplemented!()
        }

        async fn get_best_block_hash(&self) -> Result<BlockHash, ChainError> {
            Ok(*self.tip.lock().unwrap())
        }

        async fn get_block_hash(&self, _height: u64) -> Result<Option<BlockHash>, ChainError> {
            unimplemented!()
        }

        async fn get_block(&self, _hash: &BlockHash) -> Result<Block, ChainError> {
            unimplemented!()
        }

        async fn get_spends(&self, outpoints: &[OutPoint]) -> Result<Vec<OutputSpend>, ChainError> {
            self.queried.lock().unwrap().push(outpoints.to_vec());
            if let Some(next) = self.next {
                *self.tip.lock().unwrap() = next;
            }
            Ok(self
                .spends
                .iter()
                .filter(|spend| outpoints.contains(&spend.outpoint))
                .cloned()
                .collect())
        }

        async fn wait_for_block_height(
            &self,
            _height: u64,
            _timeout: Duration,
        ) -> Result<(), ChainError> {
            unimplemented!()
        }
    }

    struct Fixture {
        pool: Arc<PgPool>,
        tip: BlockHeader,
        queued: OutPoint,
        _database: TestDatabase,
    }

    impl Fixture {
        async fn new() -> Self {
            let database = TestDatabase::new().await;
            let pool = Arc::clone(&database.pool);
            let tip = BlockHeader {
                hash: BlockHash::from_byte_array([1; 32]),
                height: 100,
            };
            let queued = OutPoint::new(Txid::from_byte_array([2; 32]), 0);
            let fixture = Self {
                pool,
                tip,
                queued,
                _database: database,
            };
            fixture
                .chain()
                .add_block(&fixture.tip, &[], &[])
                .await
                .unwrap();
            fixture
                .watch(WatchedOutput {
                    outpoint: queued,
                    input: None,
                })
                .await;
            fixture
        }

        async fn watch(&self, output: WatchedOutput) {
            let mut db = self.pool.acquire().await.unwrap();
            self.chain()
                .watch_outputs(&mut db, &[output])
                .await
                .unwrap();
        }

        fn chain(&self) -> ChainRepository {
            ChainRepository::new(Arc::clone(&self.pool), Network::Regtest)
        }

        fn deps(&self, bitcoind: Bitcoind) -> SpendCheckDeps {
            SpendCheckDeps {
                chain_repository: Arc::new(self.chain()),
                chain_tip: ChainTip::at(self.tip.clone()),
                chain_client: Arc::new(bitcoind),
                blocks: Wakeup::new(),
            }
        }

        fn spend(&self) -> OutputSpend {
            OutputSpend {
                outpoint: self.queued,
                spender: Txid::from_byte_array([3; 32]),
                block: Some(self.tip.clone()),
            }
        }

        async fn count(&self, table: &str) -> i64 {
            sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
                .fetch_one(&*self.pool)
                .await
                .unwrap()
        }

        async fn watch_spender(&self) -> OutPoint {
            let output = OutPoint::new(Txid::from_byte_array([4; 32]), 0);
            self.watch(WatchedOutput {
                outpoint: output,
                input: Some(self.queued),
            })
            .await;
            output
        }
    }

    #[tokio::test]
    async fn the_spend_of_a_queued_output_is_recorded() {
        let fixture = Fixture::new().await;
        let bitcoind = Bitcoind {
            tip: Mutex::new(fixture.tip.hash),
            next: None,
            spends: vec![fixture.spend()],
            queried: Arc::default(),
        };

        check_queued(&fixture.deps(bitcoind)).await.unwrap();

        assert_eq!(fixture.count("brz_ssp_chain_unchecked_outputs").await, 0);
        assert_eq!(fixture.count("brz_ssp_chain_spent_outputs").await, 1);
    }

    #[tokio::test]
    async fn the_output_of_an_unconfirmed_transaction_is_not_queried() {
        let fixture = Fixture::new().await;
        fixture.watch_spender().await;
        let queried = Arc::default();
        let bitcoind = Bitcoind {
            tip: Mutex::new(fixture.tip.hash),
            next: None,
            spends: Vec::new(),
            queried: Arc::clone(&queried),
        };

        check_queued(&fixture.deps(bitcoind)).await.unwrap();

        assert_eq!(*queried.lock().unwrap(), [[fixture.queued]]);
        assert_eq!(fixture.count("brz_ssp_chain_unchecked_outputs").await, 0);
    }

    #[tokio::test]
    async fn the_output_of_a_transaction_that_confirmed_is_queried_next() {
        let fixture = Fixture::new().await;
        let spender_output = fixture.watch_spender().await;
        let queried = Arc::default();
        let bitcoind = Bitcoind {
            tip: Mutex::new(fixture.tip.hash),
            next: None,
            spends: vec![OutputSpend {
                spender: spender_output.txid,
                ..fixture.spend()
            }],
            queried: Arc::clone(&queried),
        };

        check_queued(&fixture.deps(bitcoind)).await.unwrap();

        assert_eq!(
            *queried.lock().unwrap(),
            [[fixture.queued], [spender_output]]
        );
        assert_eq!(fixture.count("brz_ssp_chain_unchecked_outputs").await, 0);
        assert_eq!(fixture.count("brz_ssp_chain_spent_outputs").await, 1);
    }

    #[tokio::test]
    async fn a_child_that_confirmed_in_its_parents_block_is_found() {
        let fixture = Fixture::new().await;
        let spender_output = fixture.watch_spender().await;
        let queried = Arc::default();
        let bitcoind = Bitcoind {
            tip: Mutex::new(fixture.tip.hash),
            next: None,
            spends: vec![
                OutputSpend {
                    spender: spender_output.txid,
                    ..fixture.spend()
                },
                OutputSpend {
                    outpoint: spender_output,
                    spender: Txid::from_byte_array([5; 32]),
                    block: Some(fixture.tip.clone()),
                },
            ],
            queried: Arc::clone(&queried),
        };

        check_queued(&fixture.deps(bitcoind)).await.unwrap();

        assert_eq!(
            *queried.lock().unwrap(),
            [[fixture.queued], [spender_output]]
        );
        assert_eq!(fixture.count("brz_ssp_chain_unchecked_outputs").await, 0);
        let blocks: Vec<Option<String>> =
            sqlx::query_scalar("SELECT block_hash FROM brz_ssp_chain_spent_outputs")
                .fetch_all(&*fixture.pool)
                .await
                .unwrap();
        let tip = Some(fixture.tip.hash.to_string());
        assert_eq!(blocks, [tip.clone(), tip]);
    }

    #[tokio::test]
    async fn nothing_is_recorded_while_bitcoind_is_at_another_block_than_the_chain_record() {
        let fixture = Fixture::new().await;
        let bitcoind = Bitcoind {
            tip: Mutex::new(BlockHash::from_byte_array([9; 32])),
            next: None,
            spends: vec![fixture.spend()],
            queried: Arc::default(),
        };

        check_queued(&fixture.deps(bitcoind)).await.unwrap();

        assert_eq!(fixture.count("brz_ssp_chain_unchecked_outputs").await, 1);
        assert_eq!(fixture.count("brz_ssp_chain_spent_outputs").await, 0);
    }

    #[tokio::test]
    async fn nothing_is_queried_while_the_chain_monitor_writes_a_block() {
        let fixture = Fixture::new().await;
        // The monitor is undoing the record's tip, or has failed to undo it.
        // bitcoind is at the block below, and the record may still have the tip.
        let below = BlockHeader {
            hash: BlockHash::from_byte_array([8; 32]),
            height: 99,
        };
        let queried = Arc::default();
        let bitcoind = Bitcoind {
            tip: Mutex::new(below.hash),
            next: None,
            spends: Vec::new(),
            queried: Arc::clone(&queried),
        };
        let deps = fixture.deps(bitcoind);
        deps.chain_tip.writing(Some(below));

        check_queued(&deps).await.unwrap();

        assert!(queried.lock().unwrap().is_empty());
        assert_eq!(fixture.count("brz_ssp_chain_unchecked_outputs").await, 1);
        assert_eq!(fixture.count("brz_ssp_chain_spent_outputs").await, 0);
    }

    #[tokio::test]
    async fn a_result_is_dropped_when_bitcoinds_tip_changed_during_the_query() {
        let fixture = Fixture::new().await;
        let bitcoind = Bitcoind {
            tip: Mutex::new(fixture.tip.hash),
            next: Some(BlockHash::from_byte_array([9; 32])),
            spends: vec![fixture.spend()],
            queried: Arc::default(),
        };

        check_queued(&fixture.deps(bitcoind)).await.unwrap();

        assert_eq!(fixture.count("brz_ssp_chain_unchecked_outputs").await, 1);
        assert_eq!(fixture.count("brz_ssp_chain_spent_outputs").await, 0);
    }
}
