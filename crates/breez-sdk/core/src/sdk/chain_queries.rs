use std::{collections::HashSet, str::FromStr, sync::Arc};

use bitcoin::{Address, OutPoint, Transaction, Txid, consensus::encode::deserialize_hex};
use futures::StreamExt;
use spark_wallet::{AddressUtxo, ChainQuery, ChainResult, Observation, SpendInfo, TreeNodeId};
use tracing::{trace, warn};

use crate::chain::{BitcoinChainService, ChainServiceError, Outspend, Utxo};

/// How many requests the SDK has open at the chain service at the same time.
const CONCURRENT_CHAIN_REQUESTS: usize = 10;

/// The batches in which a sync checks `leaves`, one after the other: as many
/// leaves per batch as the SDK has requests open. A failed request then costs
/// the unfinished results of that many leaves at most.
pub(crate) fn batches<T>(leaves: &[T]) -> std::slice::Chunks<'_, T> {
    leaves.chunks(CONCURRENT_CHAIN_REQUESTS)
}

/// The chain queries of one call and their results. The scans of a call share
/// it, so the SDK sends no query twice.
pub(crate) struct ChainQueries {
    chain: Arc<dyn BitcoinChainService>,
    /// Every query executed so far with its result. A query has
    /// [`ChainResult::Unavailable`] when the chain service failed its request
    /// or the SDK did not send it.
    observed: Vec<Observation>,
    /// Set once the chain service failed a request. The SDK then sends it no
    /// other one: a chain service that limits requests gets no more.
    failed: bool,
}

impl ChainQueries {
    pub(crate) fn new(chain: Arc<dyn BitcoinChainService>) -> Self {
        Self {
            chain,
            observed: Vec::new(),
            failed: false,
        }
    }

    pub(crate) fn chain_service(&self) -> &dyn BitcoinChainService {
        self.chain.as_ref()
    }

    pub(crate) fn observed(&self) -> &[Observation] {
        &self.observed
    }

    /// Whether the chain service failed a request of this call.
    pub(crate) fn failed(&self) -> bool {
        self.failed
    }

    /// The queries the chain service returned a result for.
    pub(crate) fn fetched(&self) -> Vec<Observation> {
        self.observed
            .iter()
            .filter(|observation| !matches!(observation.result, ChainResult::Unavailable))
            .cloned()
            .collect()
    }

    /// Runs `scan` over the results so far, executes the queries it returns, and
    /// repeats until it returns none. Returns the state of the last run.
    pub(crate) async fn resolve<T>(
        &mut self,
        mut scan: impl FnMut(&[Observation]) -> (T, Vec<ChainQuery>),
    ) -> T {
        loop {
            let (state, pending) = scan(&self.observed);
            if pending.is_empty() {
                return state;
            }
            self.execute(pending).await;
        }
    }

    /// Records a result for each of `queries`: the one the chain service
    /// returned, or [`ChainResult::Unavailable`].
    async fn execute(&mut self, queries: Vec<ChainQuery>) {
        let mut recorded: HashSet<ChainQuery> = HashSet::new();
        if !self.failed {
            let mut results = futures::stream::iter(queries.iter().cloned().map(|query| {
                let chain = self.chain.clone();
                async move {
                    let result = execute_chain_query(chain.as_ref(), &query).await;
                    (query, result)
                }
            }))
            .buffer_unordered(CONCURRENT_CHAIN_REQUESTS);
            while let Some((query, result)) = results.next().await {
                let Some(result) = result else {
                    self.failed = true;
                    break;
                };
                recorded.insert(query.clone());
                self.observed.push(Observation { query, result });
            }
        }
        for query in queries {
            if recorded.insert(query.clone()) {
                self.observed.push(Observation {
                    query,
                    result: ChainResult::Unavailable,
                });
            }
        }
    }
}

/// The result of `query` among `observed`.
pub(crate) fn result_of<'a>(
    observed: &'a [Observation],
    query: &ChainQuery,
) -> Option<&'a ChainResult> {
    observed
        .iter()
        .find(|observation| observation.query == *query)
        .map(|observation| &observation.result)
}

/// The queries of `queries` that `observed` has no result for.
pub(crate) fn without_result(
    queries: Vec<ChainQuery>,
    observed: &[Observation],
) -> Vec<ChainQuery> {
    queries
        .into_iter()
        .filter(|query| result_of(observed, query).is_none())
        .collect()
}

/// Executes one [`ChainQuery`] and returns the result in the wallet's types.
/// `None` when the chain service failed the request. A transaction or output
/// the chain service does not know is a result.
async fn execute_chain_query(
    chain: &dyn BitcoinChainService,
    query: &ChainQuery,
) -> Option<ChainResult> {
    match query {
        ChainQuery::TxConfirmed(txid) => get_confirmation(chain, txid).await,
        ChainQuery::Outspend(outpoint) => get_outspend(chain, outpoint).await,
        ChainQuery::Transaction(txid) => get_transaction(chain, txid).await,
        ChainQuery::RefundAddress { leaf_id, address } => {
            get_refund_address_txos(chain, leaf_id, address).await
        }
    }
}

async fn get_confirmation(chain: &dyn BitcoinChainService, txid: &Txid) -> Option<ChainResult> {
    match chain.get_transaction_status(txid.to_string()).await {
        Ok(status) => Some(ChainResult::Confirmed {
            confirmed: status.confirmed,
            block_height: status.block_height,
        }),
        Err(ChainServiceError::NotFound(_)) => Some(ChainResult::Confirmed {
            confirmed: false,
            block_height: None,
        }),
        Err(e) => {
            warn!(%txid, error = %e, "chain lookup failed: transaction status");
            None
        }
    }
}

async fn get_outspend(chain: &dyn BitcoinChainService, outpoint: &OutPoint) -> Option<ChainResult> {
    let outspend = match chain
        .get_outspend(outpoint.txid.to_string(), outpoint.vout)
        .await
    {
        Ok(outspend) => outspend,
        Err(ChainServiceError::NotFound(_)) => Outspend::Unspent,
        Err(e) => {
            warn!("get_outspend for {outpoint} failed: {e}");
            return None;
        }
    };
    let Outspend::Spent { txid, status, .. } = outspend else {
        trace!(%outpoint, "chain: outpoint unspent");
        return Some(ChainResult::Spend(None));
    };
    let spender_txid = match Txid::from_str(&txid) {
        Ok(spender_txid) => spender_txid,
        Err(e) => {
            warn!("outspend of {outpoint} has an unparsable spender txid {txid}: {e}");
            return Some(ChainResult::Unavailable);
        }
    };
    trace!(%outpoint, spender = %spender_txid, confirmed = status.confirmed, "chain: outpoint spent");
    Some(ChainResult::Spend(Some(SpendInfo {
        spender_txid,
        confirmed: status.confirmed,
        block_height: status.block_height,
    })))
}

async fn get_transaction(chain: &dyn BitcoinChainService, txid: &Txid) -> Option<ChainResult> {
    let hex = match chain.get_transaction_hex(txid.to_string()).await {
        Ok(hex) => hex,
        Err(ChainServiceError::NotFound(_)) => return Some(ChainResult::Unavailable),
        Err(e) => {
            warn!("get_transaction_hex for {txid} failed: {e}");
            return None;
        }
    };
    match deserialize_hex::<Transaction>(&hex) {
        Ok(tx) => {
            trace!(%txid, outputs = tx.output.len(), "chain: transaction fetched");
            Some(ChainResult::Transaction(tx))
        }
        Err(e) => {
            warn!("failed to decode transaction {txid}: {e}");
            Some(ChainResult::Unavailable)
        }
    }
}

async fn get_refund_address_txos(
    chain: &dyn BitcoinChainService,
    leaf_id: &TreeNodeId,
    address: &Address,
) -> Option<ChainResult> {
    let txos = match chain.get_address_txos(address.to_string()).await {
        Ok(txos) => txos,
        Err(ChainServiceError::NotFound(_)) => return Some(ChainResult::Unavailable),
        Err(e) => {
            warn!("get_address_txos for leaf {leaf_id} failed: {e}");
            return None;
        }
    };
    let txos: Vec<AddressUtxo> = txos
        .iter()
        .filter_map(|txo| address_utxo(txo, leaf_id))
        .collect();
    trace!(
        %leaf_id,
        txos = txos.len(),
        confirmed = txos.iter().filter(|u| u.confirmed).count(),
        "chain: refund address scanned"
    );
    Some(ChainResult::AddressUtxos(txos))
}

/// `None` for an output whose txid does not parse.
fn address_utxo(txo: &Utxo, leaf_id: &TreeNodeId) -> Option<AddressUtxo> {
    match Txid::from_str(&txo.txid) {
        Ok(txid) => Some(AddressUtxo {
            txid,
            vout: txo.vout,
            value: txo.value,
            confirmed: txo.status.confirmed,
            block_height: txo.status.block_height,
        }),
        Err(e) => {
            warn!("skipping refund txo {} for leaf {leaf_id}: {e}", txo.txid);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use bitcoin::hashes::Hash;

    use crate::chain::{RecommendedFees, TxStatus, Utxo};

    use super::*;

    /// A chain service that returns a transaction status after yielding once,
    /// so requests overlap. It fails the request for the txid of the `failing`
    /// byte, and does not know the txid of the `unknown` byte.
    #[derive(Default)]
    struct SlowChain {
        failing: Option<u8>,
        unknown: Option<u8>,
        running: AtomicUsize,
        most_running: AtomicUsize,
        requests: AtomicUsize,
    }

    async fn yield_once() {
        let mut yielded = false;
        std::future::poll_fn(|cx| {
            if yielded {
                return std::task::Poll::Ready(());
            }
            yielded = true;
            cx.waker().wake_by_ref();
            std::task::Poll::Pending
        })
        .await;
    }

    fn unreachable_chain() -> ChainServiceError {
        ChainServiceError::ServiceConnectivity("unreachable".to_string())
    }

    #[macros::async_trait]
    impl BitcoinChainService for SlowChain {
        async fn get_address_utxos(&self, _: String) -> Result<Vec<Utxo>, ChainServiceError> {
            Err(unreachable_chain())
        }

        async fn get_address_txos(&self, _: String) -> Result<Vec<Utxo>, ChainServiceError> {
            Err(unreachable_chain())
        }

        async fn get_transaction_status(
            &self,
            txid: String,
        ) -> Result<TxStatus, ChainServiceError> {
            self.requests.fetch_add(1, Ordering::SeqCst);
            let running = self
                .running
                .fetch_add(1, Ordering::SeqCst)
                .saturating_add(1);
            self.most_running.fetch_max(running, Ordering::SeqCst);
            yield_once().await;
            self.running.fetch_sub(1, Ordering::SeqCst);
            let is = |byte: Option<u8>| byte.is_some_and(|byte| txid == txid_of(byte).to_string());
            if is(self.failing) {
                return Err(unreachable_chain());
            }
            if is(self.unknown) {
                return Err(ChainServiceError::NotFound(txid));
            }
            Ok(TxStatus {
                confirmed: true,
                block_height: Some(100),
                block_time: None,
            })
        }

        async fn tip_height(&self) -> Result<u32, ChainServiceError> {
            Err(unreachable_chain())
        }

        async fn get_transaction_hex(&self, _: String) -> Result<String, ChainServiceError> {
            Err(unreachable_chain())
        }

        async fn get_outspend(&self, _: String, _: u32) -> Result<Outspend, ChainServiceError> {
            Err(unreachable_chain())
        }

        async fn broadcast_transaction(&self, _: String) -> Result<(), ChainServiceError> {
            Err(unreachable_chain())
        }

        async fn recommended_fees(&self) -> Result<RecommendedFees, ChainServiceError> {
            Err(unreachable_chain())
        }
    }

    fn txid_of(byte: u8) -> Txid {
        Txid::from_byte_array([byte; 32])
    }

    fn query(byte: u8) -> ChainQuery {
        ChainQuery::TxConfirmed(txid_of(byte))
    }

    #[macros::async_test_all]
    async fn a_scan_runs_until_it_returns_no_query() {
        let chain = Arc::new(SlowChain::default());
        let mut queries = ChainQueries::new(chain.clone());
        let mut rounds: u32 = 0;

        // The scan returns the second query only once it has the result of the
        // first.
        let state = queries
            .resolve(|observed| {
                rounds = rounds.saturating_add(1);
                let wanted: Vec<ChainQuery> = [query(1), query(2)]
                    .into_iter()
                    .take(observed.len().saturating_add(1))
                    .collect();
                (observed.len(), without_result(wanted.clone(), observed))
            })
            .await;

        assert_eq!(rounds, 3);
        assert_eq!(state, 2);
        assert_eq!(queries.fetched().len(), 2);
        assert!(!queries.failed());
        assert_eq!(chain.requests.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_batch_holds_as_many_leaves_as_the_sdk_has_requests_open() {
        let leaves: Vec<u8> = (0..25).collect();

        let sizes: Vec<usize> = batches(&leaves).map(<[u8]>::len).collect();

        assert_eq!(sizes, vec![10, 10, 5]);
        assert_eq!(batches::<u8>(&[]).count(), 0);
    }

    #[macros::async_test_all]
    async fn a_second_scan_reads_the_results_of_the_first() {
        let chain = Arc::new(SlowChain::default());
        let mut queries = ChainQueries::new(chain.clone());
        let first: Vec<ChainQuery> = (0..3).map(query).collect();
        let second: Vec<ChainQuery> = (2..5).map(query).collect();

        queries
            .resolve(|observed| ((), without_result(first.clone(), observed)))
            .await;
        queries
            .resolve(|observed| ((), without_result(second.clone(), observed)))
            .await;

        assert_eq!(chain.requests.load(Ordering::SeqCst), 5);
        assert_eq!(queries.fetched().len(), 5);
    }

    #[macros::async_test_all]
    async fn no_more_than_the_cap_of_requests_run_at_once() {
        let chain = Arc::new(SlowChain::default());
        let mut queries = ChainQueries::new(chain.clone());
        let wanted: Vec<ChainQuery> = (0..30).map(query).collect();

        queries
            .resolve(|observed| ((), without_result(wanted.clone(), observed)))
            .await;

        assert_eq!(queries.fetched().len(), 30);
        assert_eq!(
            chain.most_running.load(Ordering::SeqCst),
            CONCURRENT_CHAIN_REQUESTS
        );
    }

    #[macros::async_test_all]
    async fn a_failed_request_leaves_the_rest_unsent() {
        let chain = Arc::new(SlowChain {
            failing: Some(0),
            ..Default::default()
        });
        let mut queries = ChainQueries::new(chain.clone());
        let wanted: Vec<ChainQuery> = (0..30).map(query).collect();

        queries
            .resolve(|observed| ((), without_result(wanted.clone(), observed)))
            .await;

        // The SDK sent one more request for each result it read, until it read
        // the failure.
        let requests = chain.requests.load(Ordering::SeqCst);
        let fetched = queries.fetched().len();
        assert!(requests <= fetched.saturating_add(CONCURRENT_CHAIN_REQUESTS));
        assert!(requests < 30);
        assert!(queries.failed());
        assert_eq!(queries.observed().len(), 30);
        assert_eq!(
            result_of(queries.observed(), &query(0)),
            Some(&ChainResult::Unavailable)
        );

        // Nor does it send one for a later scan of the same call.
        queries
            .resolve(|observed| ((), without_result(vec![query(40)], observed)))
            .await;
        assert_eq!(chain.requests.load(Ordering::SeqCst), requests);
    }

    #[macros::async_test_all]
    async fn a_transaction_the_chain_service_does_not_know_is_a_result() {
        let chain = Arc::new(SlowChain {
            unknown: Some(0),
            ..Default::default()
        });
        let mut queries = ChainQueries::new(chain.clone());
        let wanted: Vec<ChainQuery> = (0..30).map(query).collect();

        queries
            .resolve(|observed| ((), without_result(wanted.clone(), observed)))
            .await;

        assert_eq!(chain.requests.load(Ordering::SeqCst), 30);
        assert_eq!(queries.fetched().len(), 30);
        assert_eq!(
            result_of(queries.observed(), &query(0)),
            Some(&ChainResult::Confirmed {
                confirmed: false,
                block_height: None,
            })
        );
    }
}
