use std::{collections::HashSet, sync::Arc, time::Duration};

use bitcoin::{Address, Block, Network, OutPoint};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::chain::{AddressUtxo, BlockHeader, ChainClient, ChainRepository, SpentTxo, Txo};
use crate::wakeup::Wakeup;

use super::ChainError;

/// How far below the node's tip a new chain record starts. The record's first block
/// is never undone, so this assumes no reorg is that deep.
const BIRTHDAY_DEPTH: u64 = 20;

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
    ) -> Self {
        Self {
            network,
            chain_client,
            chain_repository,
            poll_interval,
            chain_advanced,
        }
    }

    /// Follows the chain until `token` is cancelled.
    pub async fn start(self: Arc<Self>, token: CancellationToken) {
        loop {
            let mut changed = false;
            let synced = self.sync(&token, &mut changed).await;
            if changed {
                for wakeup in &self.chain_advanced {
                    wakeup.wake();
                }
            }
            let wait = async {
                match synced {
                    Ok(tip_height) => {
                        if let Err(e) = self
                            .chain_client
                            .wait_for_block_height(tip_height.saturating_add(1), self.poll_interval)
                            .await
                        {
                            warn!("waiting for a new block failed: {e}");
                            tokio::time::sleep(self.poll_interval).await;
                        }
                    }
                    Err(e) => {
                        error!("chain sync failed, retrying: {e}");
                        tokio::time::sleep(self.poll_interval).await;
                    }
                }
            };
            tokio::select! {
                () = token.cancelled() => return,
                () = wait => {}
            }
        }
    }

    /// Brings the stored chain in line with the node's and returns the stored tip's
    /// height. Every stored block is its own change, so a sync that stops partway
    /// leaves a record the next one continues from.
    async fn sync(&self, token: &CancellationToken, changed: &mut bool) -> Result<u64, ChainError> {
        let node_height = self.chain_client.get_blockheight().await?;
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

        let Some(fork) = self.rewind(stored_tip, changed).await? else {
            return Ok(self
                .chain_repository
                .get_tip()
                .await?
                .map_or(0, |tip| tip.height));
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
        Ok(tip.height)
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
            for hash in stored.iter().filter(|hash| Some(**hash) != kept) {
                info!(%hash, "block {height} left the chain, undoing it");
                self.chain_repository.undo_block(*hash).await?;
                *changed = true;
            }
            if let Some(hash) = kept {
                return Ok(Some(BlockHeader { hash, height }));
            }
            height = height.saturating_sub(1);
        }
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

        self.chain_repository
            .add_block(header, &outputs, &spends)
            .await?;
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

    use bitcoin::{
        BlockHash, CompactTarget, Transaction, TxIn, TxMerkleNode, absolute::LockTime, block,
        hashes::Hash, script::Builder, transaction,
    };

    use super::*;
    use crate::chain::{BroadcastError, ChainRepositoryError};

    #[derive(Default)]
    struct Node {
        blocks: Mutex<HashMap<BlockHash, (Block, u64)>>,
        active: Mutex<BTreeMap<u64, BlockHash>>,
        unavailable: Mutex<Option<BlockHash>>,
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

        async fn estimate_fee_rate(&self, _conf_target: u32) -> Result<u64, ChainError> {
            unimplemented!()
        }

        async fn get_blockheight(&self) -> Result<u64, ChainError> {
            Ok(self.height())
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

        async fn add_watch_address(&self, _address: &Address) -> Result<(), ChainRepositoryError> {
            unimplemented!()
        }

        async fn filter_watch_addresses(
            &self,
            _addresses: &[Address],
        ) -> Result<Vec<Address>, ChainRepositoryError> {
            Ok(Vec::new())
        }

        async fn get_txos_for_address(
            &self,
            _address: &Address,
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

    fn monitor(
        node: &Arc<Node>,
        store: &Arc<Store>,
        wakeups: Vec<Wakeup>,
    ) -> Arc<ChainMonitor<Node, Store>> {
        Arc::new(ChainMonitor::new(
            Network::Regtest,
            Arc::clone(node),
            Arc::clone(store),
            Duration::from_millis(10),
            wakeups,
        ))
    }

    async fn sync(monitor: &ChainMonitor<Node, Store>) -> Result<u64, ChainError> {
        monitor.sync(&CancellationToken::new(), &mut false).await
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
}
