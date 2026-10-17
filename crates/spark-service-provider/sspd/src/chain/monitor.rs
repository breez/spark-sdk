use std::{collections::HashSet, sync::Arc, time::Duration};

use bitcoin::{Address, Block, Network, OutPoint};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::chain::{AddressUtxo, BlockHeader, ChainClient, ChainRepository, SpentTxo, Txo};
use crate::wakeup::Wakeup;

use super::ChainTip;
use super::{ChainError, Synced};

/// How far below the node's tip a new chain record starts. The record's first block
/// is never undone, so this assumes no reorg is that deep.
const BIRTHDAY_DEPTH: u64 = 20;

const INITIAL_RETRY_DELAY: Duration = Duration::from_secs(1);

struct StoredTip {
    height: u64,
    /// Whether bitcoind reported that it is synced, and the stored tip is at or
    /// above the height bitcoind reported.
    at_synced_tip: bool,
}

pub struct ChainMonitor<C, R>
where
    C: ChainClient,
    R: ChainRepository,
{
    network: Network,
    chain_client: Arc<C>,
    chain_repository: Arc<R>,
    poll_interval: Duration,
    chain_advanced: Vec<Wakeup>,
    chain_tip: ChainTip,
    synced: Synced,
}

impl<C, R> ChainMonitor<C, R>
where
    C: ChainClient + Sync + Send + 'static,
    R: ChainRepository + Sync + Send + 'static,
{
    pub fn new(
        network: Network,
        chain_client: Arc<C>,
        chain_repository: Arc<R>,
        poll_interval: Duration,
        chain_advanced: Vec<Wakeup>,
        chain_tip: ChainTip,
        synced: Synced,
    ) -> Self {
        Self {
            network,
            chain_client,
            chain_repository,
            poll_interval,
            chain_advanced,
            chain_tip,
            synced,
        }
    }

    /// Follows the chain until `token` is cancelled.
    pub async fn start(self: Arc<Self>, token: CancellationToken) {
        let mut retry = INITIAL_RETRY_DELAY;
        loop {
            let mut changed = false;
            let stored_tip = self.sync(&token, &mut changed).await;
            if changed {
                for wakeup in &self.chain_advanced {
                    wakeup.wake();
                }
            }
            let wait = async {
                match stored_tip {
                    Ok(stored_tip) => {
                        if let Err(e) = self.wait_for_block(&stored_tip).await {
                            warn!("waiting for a new block failed: {e}");
                            self.back_off(&mut retry).await;
                        } else {
                            retry = INITIAL_RETRY_DELAY;
                        }
                    }
                    Err(e) => {
                        error!("chain sync failed, retrying: {e}");
                        self.back_off(&mut retry).await;
                    }
                }
            };
            tokio::select! {
                () = token.cancelled() => return,
                () = wait => {}
            }
        }
    }

    async fn back_off(&self, retry: &mut Duration) {
        tokio::time::sleep((*retry).min(self.poll_interval)).await;
        *retry = retry.saturating_mul(2);
    }

    async fn wait_for_block(&self, stored_tip: &StoredTip) -> Result<(), ChainError> {
        if stored_tip.at_synced_tip {
            self.synced.waiting();
        }
        let waited = self
            .chain_client
            .wait_for_block_height(stored_tip.height.saturating_add(1), self.poll_interval)
            .await;
        self.synced.stopped();
        waited
    }

    /// Brings the stored chain in line with bitcoind's and returns the stored
    /// tip. Every stored block is its own change, so a sync that stops partway
    /// leaves a record the next one continues from.
    async fn sync(
        &self,
        token: &CancellationToken,
        changed: &mut bool,
    ) -> Result<StoredTip, ChainError> {
        let bitcoind = self.chain_client.sync_state().await?;
        let node_height = bitcoind.height;
        self.chain_tip.set_bitcoind_sync_state(bitcoind);
        let stored_tip = if let Some(tip) = self.chain_repository.get_tip().await? {
            tip
        } else {
            let height = node_height.saturating_sub(BIRTHDAY_DEPTH);
            let hash = self
                .chain_client
                .get_block_hash(height)
                .await?
                .ok_or_else(|| {
                    ChainError::General(format!("no block at height {height}").into())
                })?;
            let birthday = BlockHeader { hash, height };
            self.chain_repository.add_block(&birthday, &[], &[]).await?;
            info!(%hash, "chain record starts at block {height}");
            birthday
        };
        self.chain_tip.stored_is(Some(stored_tip.clone()));

        let Some(fork) = self.rewind(stored_tip, changed).await? else {
            return Ok(StoredTip {
                height: self.chain_tip.record().map_or(0, |tip| tip.height),
                at_synced_tip: false,
            });
        };

        let mut tip = fork;
        while tip.height < node_height && !token.is_cancelled() {
            let height = tip.height.saturating_add(1);
            let Some(hash) = self.chain_client.get_block_hash(height).await? else {
                break;
            };
            let block = self.chain_client.get_block(&hash).await?;
            // The node reorganised while this sync ran; the next one rewinds.
            if block.header.prev_blockhash != tip.hash {
                break;
            }
            let header = BlockHeader { hash, height };
            self.apply_block(&block, &header).await?;
            *changed = true;
            tip = header;
        }
        Ok(StoredTip {
            height: tip.height,
            at_synced_tip: bitcoind.synced && tip.height >= node_height,
        })
    }

    /// Undoes the stored blocks the node's chain does not have, from the top down,
    /// and returns the highest stored block it does have. `None` when the node does
    /// not have the lowest stored block, as while it reindexes: that block is never
    /// undone.
    async fn rewind(
        &self,
        stored_tip: BlockHeader,
        changed: &mut bool,
    ) -> Result<Option<BlockHeader>, ChainError> {
        let mut height = stored_tip.height;
        loop {
            let node_hash = self.chain_client.get_block_hash(height).await?;
            let stored = self.chain_repository.get_block_hashes(height).await?;
            let kept = node_hash.filter(|hash| stored.contains(hash));
            if kept.is_none() {
                let base = self.chain_repository.get_base().await?;
                if base.is_none_or(|base| height <= base.height) {
                    warn!(
                        height,
                        "the node's chain does not have the chain record's first block"
                    );
                    return Ok(None);
                }
            }
            if stored.iter().any(|hash| Some(*hash) != kept) {
                let tip = self.tip_without(height, kept).await?;
                self.chain_tip.writing(tip);
                for hash in stored.iter().filter(|hash| Some(**hash) != kept) {
                    info!(%hash, "block {height} left the chain, undoing it");
                    self.chain_repository.undo_block(*hash).await?;
                    *changed = true;
                }
                self.chain_tip.written();
            }
            if let Some(hash) = kept {
                return Ok(Some(BlockHeader { hash, height }));
            }
            height = height.saturating_sub(1);
        }
    }

    /// The stored tip once the monitor has undone every stored block at
    /// `height` except `kept`.
    async fn tip_without(
        &self,
        height: u64,
        kept: Option<bitcoin::BlockHash>,
    ) -> Result<Option<BlockHeader>, ChainError> {
        if let Some(hash) = kept {
            return Ok(Some(BlockHeader { hash, height }));
        }
        let height = height.saturating_sub(1);
        let below = self.chain_repository.get_block_hashes(height).await?;
        Ok(below.first().map(|hash| BlockHeader {
            hash: *hash,
            height,
        }))
    }

    #[allow(clippy::cast_possible_truncation)]
    async fn apply_block(&self, block: &Block, header: &BlockHeader) -> Result<(), ChainError> {
        let mut outputs = Vec::new();
        let mut spends = Vec::new();
        for tx in &block.txdata {
            let txid = tx.compute_txid();
            for (vout, output) in tx.output.iter().enumerate() {
                if let Ok(address) = Address::from_script(&output.script_pubkey, self.network) {
                    outputs.push(AddressUtxo {
                        address,
                        utxo: Txo {
                            block_height: header.height,
                            outpoint: OutPoint::new(txid, vout as u32),
                            tx_out: output.clone(),
                        },
                    });
                }
            }
            for (vin, input) in tx.input.iter().enumerate() {
                spends.push(SpentTxo {
                    spending_tx: txid,
                    spending_input_index: vin as u32,
                    outpoint: input.previous_output,
                });
            }
        }

        let addresses: Vec<Address> = outputs.iter().map(|o| o.address.clone()).collect();
        let watched: HashSet<Address> = self
            .chain_repository
            .filter_watch_addresses(&addresses)
            .await?
            .into_iter()
            .collect();
        outputs.retain(|output| watched.contains(&output.address));
        for output in &outputs {
            info!(
                "block {} ({}) contains output {} for address {}, amount {}",
                header.height,
                header.hash,
                output.utxo.outpoint,
                output.address,
                output.utxo.tx_out.value
            );
        }

        self.chain_tip.writing(Some(header.clone()));
        self.chain_repository
            .add_block(header, &outputs, &spends)
            .await?;
        self.chain_tip.written();
        Ok(())
    }
}

impl From<super::ChainRepositoryError> for ChainError {
    fn from(value: super::ChainRepositoryError) -> Self {
        ChainError::Database(value)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    use bitcoin::{
        BlockHash, CompactTarget, FeeRate, Transaction, TxIn, TxMerkleNode, absolute::LockTime,
        block, hashes::Hash, script::Builder, transaction,
    };

    use super::*;
    use crate::chain::SyncState;
    use crate::chain::{BroadcastError, ChainRepositoryError};

    #[derive(Default)]
    struct Node {
        blocks: Mutex<HashMap<BlockHash, (Block, u64)>>,
        active: Mutex<BTreeMap<u64, BlockHash>>,
        unavailable: Mutex<Option<BlockHash>>,
        syncing: AtomicBool,
    }

    impl Node {
        fn mine(
            &self,
            parent: BlockHash,
            parent_height: u64,
            count: u64,
            branch: u32,
        ) -> Vec<BlockHash> {
            let mut hashes = Vec::new();
            let mut prev = parent;
            for height in (1..=count).map(|i| parent_height.saturating_add(i)) {
                let coinbase = Transaction {
                    version: transaction::Version::ONE,
                    lock_time: LockTime::ZERO,
                    input: vec![TxIn {
                        script_sig: Builder::new()
                            .push_int(i64::try_from(height).unwrap())
                            .into_script(),
                        ..TxIn::default()
                    }],
                    output: Vec::new(),
                };
                let block = Block {
                    header: block::Header {
                        version: block::Version::TWO,
                        prev_blockhash: prev,
                        merkle_root: TxMerkleNode::all_zeros(),
                        time: 0,
                        bits: CompactTarget::from_consensus(0),
                        nonce: branch,
                    },
                    txdata: vec![coinbase],
                };
                prev = block.block_hash();
                hashes.push(prev);
                self.blocks.lock().unwrap().insert(prev, (block, height));
            }
            hashes
        }

        /// Makes the chain ending in `tip` the active one.
        fn set_tip(&self, tip: BlockHash) {
            let blocks = self.blocks.lock().unwrap();
            let mut active = BTreeMap::new();
            let mut current = Some(tip);
            while let Some(hash) = current {
                let Some((block, height)) = blocks.get(&hash) else {
                    break;
                };
                active.insert(*height, hash);
                current = Some(block.header.prev_blockhash);
            }
            *self.active.lock().unwrap() = active;
        }

        fn height(&self) -> u64 {
            self.active
                .lock()
                .unwrap()
                .keys()
                .next_back()
                .copied()
                .unwrap_or(0)
        }
    }

    #[async_trait::async_trait]
    impl ChainClient for Node {
        async fn broadcast_tx(&self, _tx: Transaction) -> Result<(), BroadcastError> {
            unimplemented!()
        }

        async fn broadcast_package(&self, _txs: &[Transaction]) -> Result<(), BroadcastError> {
            unimplemented!()
        }

        async fn estimate_fee_rate(&self, _conf_target: u32) -> Result<FeeRate, ChainError> {
            unimplemented!()
        }

        async fn get_blockheight(&self) -> Result<u64, ChainError> {
            Ok(self.height())
        }

        async fn sync_state(&self) -> Result<SyncState, ChainError> {
            Ok(SyncState {
                height: self.height(),
                synced: !self.syncing.load(Ordering::SeqCst),
            })
        }

        async fn get_block_hash(&self, height: u64) -> Result<Option<BlockHash>, ChainError> {
            Ok(self.active.lock().unwrap().get(&height).copied())
        }

        async fn get_block(&self, hash: &BlockHash) -> Result<Block, ChainError> {
            if *self.unavailable.lock().unwrap() == Some(*hash) {
                return Err(ChainError::General("block unavailable".into()));
            }
            Ok(self.blocks.lock().unwrap()[hash].0.clone())
        }

        async fn wait_for_block_height(
            &self,
            height: u64,
            timeout: Duration,
        ) -> Result<(), ChainError> {
            let deadline = tokio::time::Instant::now().checked_add(timeout).unwrap();
            while self.height() < height && tokio::time::Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            Ok(())
        }
    }

    #[derive(Default)]
    struct Store(Mutex<HashMap<BlockHash, u64>>);

    impl Store {
        fn chain(&self) -> BTreeMap<u64, Vec<BlockHash>> {
            let mut chain: BTreeMap<u64, Vec<BlockHash>> = BTreeMap::new();
            for (hash, height) in self.0.lock().unwrap().iter() {
                chain.entry(*height).or_default().push(*hash);
            }
            chain
        }
    }

    #[async_trait::async_trait]
    impl ChainRepository for Store {
        async fn add_block(
            &self,
            block: &BlockHeader,
            _tx_outputs: &[AddressUtxo],
            _tx_inputs: &[SpentTxo],
        ) -> Result<(), ChainRepositoryError> {
            self.0.lock().unwrap().insert(block.hash, block.height);
            Ok(())
        }

        async fn undo_block(&self, hash: BlockHash) -> Result<(), ChainRepositoryError> {
            self.0.lock().unwrap().remove(&hash);
            Ok(())
        }

        async fn get_block_hashes(
            &self,
            height: u64,
        ) -> Result<Vec<BlockHash>, ChainRepositoryError> {
            Ok(self.chain().remove(&height).unwrap_or_default())
        }

        async fn get_tip(&self) -> Result<Option<BlockHeader>, ChainRepositoryError> {
            Ok(self
                .chain()
                .into_iter()
                .next_back()
                .map(|(height, hashes)| BlockHeader {
                    hash: hashes[0],
                    height,
                }))
        }

        async fn get_base(&self) -> Result<Option<BlockHeader>, ChainRepositoryError> {
            Ok(self
                .chain()
                .into_iter()
                .next()
                .map(|(height, hashes)| BlockHeader {
                    hash: hashes[0],
                    height,
                }))
        }

        async fn add_watch_addresses(
            &self,
            _addresses: &[Address],
        ) -> Result<(), ChainRepositoryError> {
            unimplemented!()
        }

        async fn filter_watch_addresses(
            &self,
            _addresses: &[Address],
        ) -> Result<Vec<Address>, ChainRepositoryError> {
            Ok(Vec::new())
        }

        async fn get_txos_for_addresses(
            &self,
            _addresses: &[Address],
        ) -> Result<Vec<Txo>, ChainRepositoryError> {
            unimplemented!()
        }

        async fn get_spenders(
            &self,
            _outpoints: &[OutPoint],
        ) -> Result<Vec<crate::chain::Spender>, ChainRepositoryError> {
            unimplemented!()
        }
    }

    fn monitor<R: ChainRepository + Sync + Send + 'static>(
        node: &Arc<Node>,
        store: &Arc<R>,
        wakeups: Vec<Wakeup>,
    ) -> Arc<ChainMonitor<Node, R>> {
        Arc::new(ChainMonitor::new(
            Network::Regtest,
            Arc::clone(node),
            Arc::clone(store),
            Duration::from_millis(10),
            wakeups,
            ChainTip::default(),
            Synced::new(),
        ))
    }

    async fn sync<R: ChainRepository + Sync + Send + 'static>(
        monitor: &ChainMonitor<Node, R>,
    ) -> Result<u64, ChainError> {
        monitor
            .sync(&CancellationToken::new(), &mut false)
            .await
            .map(|stored_tip| stored_tip.height)
    }

    fn stored(store: &Store) -> Vec<BlockHash> {
        store.chain().into_values().flatten().collect()
    }

    #[tokio::test]
    async fn a_reorg_is_undone_to_the_fork_and_the_new_chain_applied() {
        let node = Arc::new(Node::default());
        let a = node.mine(BlockHash::all_zeros(), 99, 30, 0);
        let b = node.mine(a[25], 125, 8, 1);
        let store = Arc::new(Store::default());
        let monitor = monitor(&node, &store, Vec::new());

        node.set_tip(a[29]);
        sync(&monitor).await.unwrap();
        assert_eq!(stored(&store), a[9..].to_vec());

        node.set_tip(b[7]);
        assert_eq!(sync(&monitor).await.unwrap(), 133);
        let expected: Vec<BlockHash> = a[9..=25].iter().chain(&b).copied().collect();
        assert_eq!(stored(&store), expected);
    }

    #[tokio::test]
    async fn a_sync_failing_partway_is_continued_by_the_next() {
        let node = Arc::new(Node::default());
        let a = node.mine(BlockHash::all_zeros(), 99, 30, 0);
        let b = node.mine(a[25], 125, 8, 1);
        let store = Arc::new(Store::default());
        let monitor = monitor(&node, &store, Vec::new());
        node.set_tip(a[29]);
        sync(&monitor).await.unwrap();

        node.set_tip(b[7]);
        *node.unavailable.lock().unwrap() = Some(b[2]);
        assert!(sync(&monitor).await.is_err());

        *node.unavailable.lock().unwrap() = None;
        sync(&monitor).await.unwrap();
        let expected: Vec<BlockHash> = a[9..=25].iter().chain(&b).copied().collect();
        assert_eq!(stored(&store), expected);
    }

    #[tokio::test]
    async fn a_block_stored_by_a_sync_that_failed_is_undone_once_reorged_out() {
        let node = Arc::new(Node::default());
        let a = node.mine(BlockHash::all_zeros(), 99, 30, 0);
        let b = node.mine(a[25], 125, 4, 1);
        let store = Arc::new(Store::default());
        let monitor = monitor(&node, &store, Vec::new());
        node.set_tip(a[25]);
        sync(&monitor).await.unwrap();
        store
            .add_block(
                &BlockHeader {
                    hash: b[0],
                    height: 126,
                },
                &[],
                &[],
            )
            .await
            .unwrap();

        node.set_tip(a[29]);
        sync(&monitor).await.unwrap();
        assert_eq!(stored(&store), a[5..].to_vec());
    }

    #[tokio::test]
    async fn the_first_stored_block_is_never_undone() {
        let node = Arc::new(Node::default());
        let a = node.mine(BlockHash::all_zeros(), 99, 30, 0);
        let other = node.mine(BlockHash::all_zeros(), 99, 30, 1);
        let store = Arc::new(Store::default());
        let monitor = monitor(&node, &store, Vec::new());
        node.set_tip(a[29]);
        sync(&monitor).await.unwrap();

        node.set_tip(other[29]);
        assert_eq!(sync(&monitor).await.unwrap(), 109);
        assert_eq!(stored(&store), vec![a[9]]);

        node.set_tip(a[29]);
        sync(&monitor).await.unwrap();
        assert_eq!(stored(&store), a[9..].to_vec());
    }

    #[tokio::test]
    async fn the_tip_in_memory_is_the_stored_tip_after_every_sync() {
        let bitcoind = Arc::new(Node::default());
        let a = bitcoind.mine(BlockHash::all_zeros(), 99, 30, 0);
        let b = bitcoind.mine(a[25], 125, 8, 1);
        let other = bitcoind.mine(BlockHash::all_zeros(), 99, 30, 2);
        let store = Arc::new(Store::default());
        let monitor = monitor(&bitcoind, &store, Vec::new());
        assert_eq!(monitor.chain_tip.record(), None);

        bitcoind.set_tip(a[29]);
        sync(&monitor).await.unwrap();
        assert_eq!(monitor.chain_tip.record().unwrap().hash, a[29]);
        assert_eq!(monitor.chain_tip.stored(), monitor.chain_tip.record());
        let reported = monitor.chain_tip.bitcoind_sync_state().unwrap();
        assert_eq!((reported.height, reported.synced), (129, true));

        // A reorg: the monitor stores two blocks of the new chain, then fails to
        // get the third.
        bitcoind.set_tip(b[7]);
        *bitcoind.unavailable.lock().unwrap() = Some(b[2]);
        assert!(sync(&monitor).await.is_err());
        assert_eq!(monitor.chain_tip.record().unwrap().hash, b[1]);
        assert_eq!(monitor.chain_tip.stored(), store.get_tip().await.unwrap());

        // bitcoind's chain lacks the record's first block.
        bitcoind.set_tip(other[29]);
        sync(&monitor).await.unwrap();
        assert_eq!(monitor.chain_tip.record().unwrap().hash, a[9]);
        assert_eq!(monitor.chain_tip.stored(), store.get_tip().await.unwrap());
    }

    #[derive(Default)]
    struct Checked {
        store: Store,
        chain_tip: std::sync::OnceLock<ChainTip>,
        undo_fails: AtomicBool,
    }

    impl Checked {
        fn tip_in_memory(&self) -> Option<BlockHeader> {
            let chain_tip = self.chain_tip.get()?;
            assert_eq!(chain_tip.stored(), None);
            chain_tip.record()
        }
    }

    #[async_trait::async_trait]
    impl ChainRepository for Checked {
        async fn add_block(
            &self,
            block: &BlockHeader,
            tx_outputs: &[AddressUtxo],
            tx_inputs: &[SpentTxo],
        ) -> Result<(), ChainRepositoryError> {
            assert!(
                self.tip_in_memory()
                    .is_none_or(|tip| tip.height < block.height)
            );
            self.store.add_block(block, tx_outputs, tx_inputs).await
        }

        async fn undo_block(&self, hash: BlockHash) -> Result<(), ChainRepositoryError> {
            let height = self.store.0.lock().unwrap()[&hash];
            let tip = self.tip_in_memory().unwrap();
            assert!(tip.height < height || (tip.height == height && tip.hash != hash));
            if self.undo_fails.swap(false, Ordering::SeqCst) {
                return Err(ChainRepositoryError::General("no connection".into()));
            }
            self.store.undo_block(hash).await
        }

        async fn get_block_hashes(
            &self,
            height: u64,
        ) -> Result<Vec<BlockHash>, ChainRepositoryError> {
            self.store.get_block_hashes(height).await
        }

        async fn get_tip(&self) -> Result<Option<BlockHeader>, ChainRepositoryError> {
            self.store.get_tip().await
        }

        async fn get_base(&self) -> Result<Option<BlockHeader>, ChainRepositoryError> {
            self.store.get_base().await
        }

        async fn add_watch_addresses(
            &self,
            addresses: &[Address],
        ) -> Result<(), ChainRepositoryError> {
            self.store.add_watch_addresses(addresses).await
        }

        async fn filter_watch_addresses(
            &self,
            addresses: &[Address],
        ) -> Result<Vec<Address>, ChainRepositoryError> {
            self.store.filter_watch_addresses(addresses).await
        }

        async fn get_txos_for_addresses(
            &self,
            addresses: &[Address],
        ) -> Result<Vec<Txo>, ChainRepositoryError> {
            self.store.get_txos_for_addresses(addresses).await
        }

        async fn get_spenders(
            &self,
            outpoints: &[OutPoint],
        ) -> Result<Vec<crate::chain::Spender>, ChainRepositoryError> {
            self.store.get_spenders(outpoints).await
        }
    }

    #[tokio::test]
    async fn the_tip_in_memory_is_never_ahead_of_the_stored_chain() {
        let bitcoind = Arc::new(Node::default());
        let a = bitcoind.mine(BlockHash::all_zeros(), 99, 30, 0);
        let b = bitcoind.mine(a[25], 125, 8, 1);
        let store = Arc::new(Checked::default());
        let monitor = monitor(&bitcoind, &store, Vec::new());
        assert!(store.chain_tip.set(monitor.chain_tip.clone()).is_ok());

        bitcoind.set_tip(a[29]);
        monitor
            .sync(&CancellationToken::new(), &mut false)
            .await
            .unwrap();
        bitcoind.set_tip(b[7]);
        monitor
            .sync(&CancellationToken::new(), &mut false)
            .await
            .unwrap();
        assert_eq!(monitor.chain_tip.record().unwrap().hash, b[7]);
    }

    #[tokio::test]
    async fn from_an_undo_that_failed_until_the_next_sync_the_stored_tip_is_not_known() {
        let bitcoind = Arc::new(Node::default());
        let a = bitcoind.mine(BlockHash::all_zeros(), 99, 30, 0);
        let store = Arc::new(Checked::default());
        let monitor = monitor(&bitcoind, &store, Vec::new());
        assert!(store.chain_tip.set(monitor.chain_tip.clone()).is_ok());
        bitcoind.set_tip(a[29]);
        sync(&monitor).await.unwrap();

        // bitcoind no longer has the record's tip, and the store fails to undo
        // it.
        bitcoind.set_tip(a[28]);
        store.undo_fails.store(true, Ordering::SeqCst);
        assert!(sync(&monitor).await.is_err());
        assert_eq!(store.get_tip().await.unwrap().unwrap().hash, a[29]);
        assert_eq!(monitor.chain_tip.record().unwrap().hash, a[28]);
        assert_eq!(monitor.chain_tip.stored(), None);

        sync(&monitor).await.unwrap();
        assert_eq!(monitor.chain_tip.stored().unwrap().hash, a[28]);
        assert_eq!(monitor.chain_tip.stored(), store.get_tip().await.unwrap());
    }

    #[tokio::test]
    async fn only_a_new_block_wakes_the_workers() {
        let node = Arc::new(Node::default());
        let blocks = node.mine(BlockHash::all_zeros(), 99, 30, 0);
        node.set_tip(blocks[28]);
        let store = Arc::new(Store::default());
        let wakeup = Wakeup::new();
        let monitor = monitor(&node, &store, vec![wakeup.clone()]);
        sync(&monitor).await.unwrap();
        let token = CancellationToken::new();
        let running = tokio::spawn(Arc::clone(&monitor).start(token.clone()));

        assert!(
            tokio::time::timeout(Duration::from_millis(200), wakeup.waited())
                .await
                .is_err(),
            "a poll that finds no new block woke the workers"
        );

        node.set_tip(blocks[29]);
        tokio::time::timeout(Duration::from_secs(5), wakeup.waited())
            .await
            .expect("a new block did not wake the workers");

        token.cancel();
        running.await.unwrap();
    }

    fn slow_monitor(bitcoind: &Arc<Node>, store: &Arc<Store>) -> Arc<ChainMonitor<Node, Store>> {
        Arc::new(ChainMonitor::new(
            Network::Regtest,
            Arc::clone(bitcoind),
            Arc::clone(store),
            Duration::from_secs(60),
            Vec::new(),
            ChainTip::default(),
            Synced::new(),
        ))
    }

    #[tokio::test(start_paused = true)]
    async fn the_monitor_calls_bitcoind_again_a_second_after_an_error() {
        let bitcoind = Arc::new(Node::default());
        let blocks = bitcoind.mine(BlockHash::all_zeros(), 99, 30, 0);
        bitcoind.set_tip(blocks[29]);
        *bitcoind.unavailable.lock().unwrap() = Some(blocks[29]);
        let store = Arc::new(Store::default());
        let monitor = slow_monitor(&bitcoind, &store);
        let token = CancellationToken::new();
        let running = tokio::spawn(Arc::clone(&monitor).start(token.clone()));

        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(!stored(&store).contains(&blocks[29]));

        *bitcoind.unavailable.lock().unwrap() = None;
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert!(stored(&store).contains(&blocks[29]));

        token.cancel();
        running.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn sspd_is_synced_while_the_monitor_waits_at_the_tip_of_a_synced_bitcoind() {
        let bitcoind = Arc::new(Node::default());
        let blocks = bitcoind.mine(BlockHash::all_zeros(), 99, 30, 0);
        bitcoind.set_tip(blocks[29]);
        bitcoind.syncing.store(true, Ordering::SeqCst);
        let monitor = slow_monitor(&bitcoind, &Arc::new(Store::default()));
        let token = CancellationToken::new();
        let running = tokio::spawn(Arc::clone(&monitor).start(token.clone()));

        tokio::time::sleep(Duration::from_secs(90)).await;
        assert!(!monitor.synced.is_synced(), "bitcoind is still syncing");

        bitcoind.syncing.store(false, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_secs(90)).await;
        assert!(monitor.synced.is_synced());

        token.cancel();
        running.await.unwrap();
    }
}
