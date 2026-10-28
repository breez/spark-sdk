#![allow(clippy::needless_raw_string_hashes)]

use std::collections::HashMap;
use std::sync::Arc;

use bitcoin::{Address, Amount, BlockHash, Network, OutPoint, TxOut, address::NetworkUnchecked};
use futures::TryStreamExt;
use sqlx::{PgConnection, PgPool, Row, postgres::PgRow};
use tracing::instrument;

use crate::chain::{self, AddressUtxo, BlockHeader, ChainRepositoryError, SpentTxo, Txo};
use crate::wakeup::Wakeup;

#[derive(Debug)]
pub struct ChainRepository {
    network: Network,
    pub(super) pool: Arc<PgPool>,
    pub(super) watched: Wakeup,
}

impl ChainRepository {
    pub fn new(pool: Arc<PgPool>, network: Network) -> Self {
        Self {
            network,
            pool,
            watched: Wakeup::new(),
        }
    }

    #[instrument(level = "trace", skip(self))]
    async fn add_utxos(
        &self,
        tx: &mut PgConnection,
        tx_outputs: &[AddressUtxo],
    ) -> Result<(), ChainRepositoryError> {
        let tx_ids: Vec<_> = tx_outputs
            .iter()
            .map(|u| u.utxo.outpoint.txid.to_string())
            .collect();
        let output_indices: Vec<_> = tx_outputs
            .iter()
            .map(|u| u.utxo.outpoint.vout.cast_signed())
            .collect();
        let addresses: Vec<_> = tx_outputs.iter().map(|u| u.address.to_string()).collect();
        let amounts: Vec<_> = tx_outputs
            .iter()
            .map(|u| u.utxo.tx_out.value.to_sat().cast_signed())
            .collect();
        sqlx::query(
            r#"INSERT INTO brz_ssp_chain_tx_outputs (
                   tx_id
               ,   output_index
               ,   address
               ,   amount)
               SELECT t.tx_id, t.output_index, t.address, t.amount
               FROM UNNEST(
                   $1::text[]
               ,   $2::int[]
               ,   $3::text[]
               ,   $4::bigint[]
               ) AS t(tx_id, output_index, address, amount)
               ON CONFLICT DO NOTHING"#,
        )
        .bind(&tx_ids)
        .bind(&output_indices)
        .bind(&addresses)
        .bind(&amounts)
        .execute(tx)
        .await?;

        Ok(())
    }

    async fn first_block(
        &self,
        query: &'static str,
    ) -> Result<Option<BlockHeader>, ChainRepositoryError> {
        let row: Option<(String, i64)> = sqlx::query_as(query).fetch_optional(&*self.pool).await?;
        row.map(|(hash, height)| {
            Ok(BlockHeader {
                hash: hash.parse()?,
                height: height.cast_unsigned(),
            })
        })
        .transpose()
    }

    fn map_txo(address: &Address, row: &PgRow) -> Result<Txo, ChainRepositoryError> {
        let tx_id: String = row.try_get("tx_id")?;
        let output_index: i32 = row.try_get("output_index")?;
        let amount: i64 = row.try_get("amount")?;
        let height: i64 = row.try_get("height")?;
        Ok(Txo {
            block_height: height.cast_unsigned(),
            outpoint: OutPoint::new(tx_id.parse()?, output_index.cast_unsigned()),
            tx_out: TxOut {
                value: Amount::from_sat(amount.cast_unsigned()),
                script_pubkey: address.script_pubkey(),
            },
        })
    }

    async fn mark_watched_outputs_spent(
        &self,
        tx: &mut PgConnection,
        block: &BlockHeader,
        txos: &[SpentTxo],
    ) -> Result<(), ChainRepositoryError> {
        let tx_ids: Vec<_> = txos.iter().map(|u| u.outpoint.txid.to_string()).collect();
        let output_indices: Vec<_> = txos.iter().map(|u| u.outpoint.vout.cast_signed()).collect();
        let spending_tx_ids: Vec<_> = txos.iter().map(|u| u.spending_tx.to_string()).collect();
        sqlx::query(
            r#"INSERT INTO brz_ssp_chain_spent_outputs (
                   tx_id
               ,   output_index
               ,   spending_tx_id
               ,   block_hash
               ,   height)
               SELECT i.tx_id
               ,      i.output_index
               ,      i.spending_tx_id
               ,      $4
               ,      $5
               FROM UNNEST(
                   $1::text[]
               ,   $2::int[]
               ,   $3::text[]
               ) AS i (
                   tx_id
               ,   output_index
               ,   spending_tx_id)
               INNER JOIN brz_ssp_chain_watch_outputs w
                   ON w.tx_id = i.tx_id AND w.output_index = i.output_index
               ON CONFLICT DO NOTHING"#,
        )
        .bind(&tx_ids)
        .bind(&output_indices)
        .bind(&spending_tx_ids)
        .bind(block.hash.to_string())
        .bind(block.height.cast_signed())
        .execute(tx)
        .await?;

        Ok(())
    }

    #[instrument(level = "trace", skip(self))]
    async fn mark_spent(
        &self,
        tx: &mut PgConnection,
        txos: &[SpentTxo],
    ) -> Result<(), ChainRepositoryError> {
        let tx_ids: Vec<_> = txos.iter().map(|u| u.outpoint.txid.to_string()).collect();
        let tx_output_indices: Vec<_> =
            txos.iter().map(|u| u.outpoint.vout.cast_signed()).collect();
        let spending_tx_ids: Vec<_> = txos.iter().map(|u| u.spending_tx.to_string()).collect();
        let spending_tx_input_indices: Vec<_> = txos
            .iter()
            .map(|u| u.spending_input_index.cast_signed())
            .collect();

        sqlx::query(
            r#"INSERT INTO brz_ssp_chain_tx_inputs (
                   tx_id
               ,   output_index
               ,   spending_tx_id
               ,   spending_input_index)
               SELECT i.tx_id
               ,      i.output_index
               ,      i.spending_tx_id
               ,      i.spending_input_index
               FROM UNNEST(
                   $1::text[]
               ,   $2::int[]
               ,   $3::text[]
               ,   $4::int[]
               ) AS i (
                   tx_id
               ,   output_index
               ,   spending_tx_id
               ,   spending_input_index)
               INNER JOIN brz_ssp_chain_tx_outputs o
                   ON i.tx_id = o.tx_id AND i.output_index = o.output_index
               ON CONFLICT DO NOTHING"#,
        )
        .bind(&tx_ids)
        .bind(&tx_output_indices)
        .bind(&spending_tx_ids)
        .bind(&spending_tx_input_indices)
        .execute(tx)
        .await?;

        Ok(())
    }
}

#[async_trait::async_trait]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
impl chain::ChainRepository for ChainRepository {
    #[instrument(level = "trace", skip(self))]
    async fn add_block(
        &self,
        block: &BlockHeader,
        tx_outputs: &[AddressUtxo],
        tx_inputs: &[SpentTxo],
    ) -> Result<(), ChainRepositoryError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            r#"INSERT INTO brz_ssp_chain_blocks (block_hash, height)
               VALUES ($1, $2)
               ON CONFLICT DO NOTHING"#,
        )
        .bind(block.hash.to_string())
        .bind(block.height.cast_signed())
        .execute(&mut *tx)
        .await?;

        self.add_utxos(&mut tx, tx_outputs).await?;

        self.mark_spent(&mut tx, tx_inputs).await?;
        self.mark_watched_outputs_spent(&mut tx, block, tx_inputs)
            .await?;

        let txns: Vec<_> = tx_outputs
            .iter()
            .map(|o| o.utxo.outpoint.txid.to_string())
            .chain(tx_inputs.iter().map(|i| i.spending_tx.to_string()))
            .collect();
        let block_hashes: Vec<_> = txns.iter().map(|_| block.hash.to_string()).collect();
        sqlx::query(
            r#"INSERT INTO brz_ssp_chain_tx_blocks
               SELECT i.tx_id
               ,      i.block_hash
               FROM UNNEST(
                   $1::text[]
               ,   $2::text[]
               ) AS i (
                   tx_id
               ,   block_hash)
               WHERE EXISTS (SELECT 1
                             FROM brz_ssp_chain_tx_outputs o
                             WHERE o.tx_id = i.tx_id)
                   OR EXISTS (SELECT 1
                              FROM brz_ssp_chain_tx_inputs ti
                              WHERE ti.spending_tx_id = i.tx_id)
               ON CONFLICT DO NOTHING"#,
        )
        .bind(&txns)
        .bind(&block_hashes)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    #[instrument(level = "trace", skip(self))]
    async fn add_watch_addresses(&self, addresses: &[Address]) -> Result<(), ChainRepositoryError> {
        let addresses: Vec<String> = addresses.iter().map(ToString::to_string).collect();
        sqlx::query(
            r#"INSERT INTO brz_ssp_chain_watch_addresses (address)
               SELECT * FROM UNNEST($1::text[])
               ON CONFLICT DO NOTHING"#,
        )
        .bind(&addresses)
        .execute(&*self.pool)
        .await?;

        Ok(())
    }

    #[instrument(level = "trace", skip(self))]
    async fn filter_watch_addresses(
        &self,
        addresses: &[Address],
    ) -> Result<Vec<Address>, ChainRepositoryError> {
        let addresses: Vec<String> = addresses.iter().map(ToString::to_string).collect();
        let mut rows = sqlx::query(
            r#"SELECT address
               FROM brz_ssp_chain_watch_addresses
               WHERE address = ANY($1)"#,
        )
        .bind(addresses)
        .fetch(&*self.pool);

        let mut result: Vec<Address> = Vec::new();
        while let Some(row) = rows.try_next().await? {
            let address: String = row.try_get("address")?;
            let address = address
                .parse::<Address<NetworkUnchecked>>()?
                .require_network(self.network)?;
            result.push(address);
        }
        Ok(result)
    }

    #[instrument(level = "trace", skip(self))]
    async fn get_block_hashes(&self, height: u64) -> Result<Vec<BlockHash>, ChainRepositoryError> {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT block_hash FROM brz_ssp_chain_blocks WHERE height = $1")
                .bind(height.cast_signed())
                .fetch_all(&*self.pool)
                .await?;
        rows.into_iter().map(|(hash,)| Ok(hash.parse()?)).collect()
    }

    #[instrument(level = "trace", skip(self))]
    async fn get_tip(&self) -> Result<Option<BlockHeader>, ChainRepositoryError> {
        self.first_block(
            "SELECT block_hash, height FROM brz_ssp_chain_blocks ORDER BY height DESC LIMIT 1",
        )
        .await
    }

    #[instrument(level = "trace", skip(self))]
    async fn get_base(&self) -> Result<Option<BlockHeader>, ChainRepositoryError> {
        self.first_block(
            "SELECT block_hash, height FROM brz_ssp_chain_blocks ORDER BY height LIMIT 1",
        )
        .await
    }

    #[instrument(level = "trace", skip(self))]
    async fn get_txos_for_addresses(
        &self,
        addresses: &[Address],
    ) -> Result<Vec<Txo>, ChainRepositoryError> {
        let by_text: HashMap<String, &Address> = addresses
            .iter()
            .map(|address| (address.to_string(), address))
            .collect();
        let texts: Vec<&str> = by_text.keys().map(String::as_str).collect();
        let mut rows = sqlx::query(
            r#"SELECT o.address
            ,         o.tx_id
            ,         o.output_index
            ,         o.amount
            ,         b.height
            FROM brz_ssp_chain_tx_outputs o
            INNER JOIN brz_ssp_chain_tx_blocks tb ON tb.tx_id = o.tx_id
            INNER JOIN brz_ssp_chain_blocks b ON tb.block_hash = b.block_hash
            WHERE o.address = ANY($1)
            ORDER BY b.height, o.tx_id, o.output_index"#,
        )
        .bind(&texts)
        .fetch(&*self.pool);

        let mut result: Vec<Txo> = Vec::new();
        while let Some(row) = rows.try_next().await? {
            let address: String = row.try_get("address")?;
            if let Some(address) = by_text.get(&address) {
                result.push(Self::map_txo(address, &row)?);
            }
        }
        Ok(result)
    }

    #[instrument(level = "trace", skip(self))]
    async fn get_spenders(
        &self,
        outpoints: &[OutPoint],
    ) -> Result<Vec<chain::Spender>, ChainRepositoryError> {
        if outpoints.is_empty() {
            return Ok(Vec::new());
        }
        let tx_ids: Vec<String> = outpoints.iter().map(|o| o.txid.to_string()).collect();
        let output_indices: Vec<i32> = outpoints.iter().map(|o| o.vout.cast_signed()).collect();

        let rows: Vec<(String, i32, String, i64)> = sqlx::query_as(
            r#"SELECT i.tx_id
               ,      i.output_index
               ,      i.spending_tx_id
               ,      b.height
               FROM brz_ssp_chain_tx_inputs i
               INNER JOIN brz_ssp_chain_tx_blocks tb ON tb.tx_id = i.spending_tx_id
               INNER JOIN brz_ssp_chain_blocks b ON b.block_hash = tb.block_hash
               INNER JOIN UNNEST($1::text[], $2::int[]) AS o(tx_id, output_index)
                   ON o.tx_id = i.tx_id AND o.output_index = i.output_index"#,
        )
        .bind(&tx_ids)
        .bind(&output_indices)
        .fetch_all(&*self.pool)
        .await?;

        rows.into_iter()
            .map(|(tx_id, output_index, spending_tx_id, height)| {
                Ok(chain::Spender {
                    outpoint: OutPoint::new(tx_id.parse()?, output_index.cast_unsigned()),
                    txid: spending_tx_id.parse()?,
                    block_height: height.cast_unsigned(),
                })
            })
            .collect()
    }

    #[instrument(level = "trace", skip(self))]
    async fn undo_block(&self, hash: BlockHash) -> Result<(), ChainRepositoryError> {
        sqlx::query("DELETE FROM brz_ssp_chain_blocks WHERE block_hash = $1")
            .bind(hash.to_string())
            .execute(&*self.pool)
            .await?;
        Ok(())
    }
}

impl From<bitcoin::address::ParseError> for ChainRepositoryError {
    fn from(value: bitcoin::address::ParseError) -> Self {
        ChainRepositoryError::General(Box::new(value))
    }
}

impl From<bitcoin::hashes::hex::HexToArrayError> for ChainRepositoryError {
    fn from(value: bitcoin::hashes::hex::HexToArrayError) -> Self {
        ChainRepositoryError::General(Box::new(value))
    }
}

impl From<sqlx::Error> for ChainRepositoryError {
    fn from(value: sqlx::Error) -> Self {
        ChainRepositoryError::General(Box::new(value))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use bitcoin::Txid;
    use bitcoin::hashes::Hash;
    use bitcoin::secp256k1::{Secp256k1, SecretKey};

    use super::*;
    use crate::chain::{ChainRepository as _, Spender, WatchedOutput};
    use crate::postgresql::testing::TestDatabase;

    /// An output, its address and its value in sats.
    type Paid<'a> = (OutPoint, &'a Address, u64);

    /// An output, the transaction with that output as an input, and the index
    /// of that input.
    type Spend = (OutPoint, Txid, u32);

    struct Fixture {
        chain: ChainRepository,
        _database: TestDatabase,
    }

    impl Fixture {
        async fn new() -> Self {
            let database = TestDatabase::new().await;
            Self {
                chain: ChainRepository::new(Arc::clone(&database.pool), Network::Regtest),
                _database: database,
            }
        }

        async fn add_block(&self, block: &BlockHeader, outputs: &[Paid<'_>], spends: &[Spend]) {
            let outputs: Vec<AddressUtxo> = outputs
                .iter()
                .map(|(outpoint, address, value)| AddressUtxo {
                    address: (*address).clone(),
                    utxo: Txo {
                        block_height: block.height,
                        outpoint: *outpoint,
                        tx_out: TxOut {
                            value: Amount::from_sat(*value),
                            script_pubkey: address.script_pubkey(),
                        },
                    },
                })
                .collect();
            let spends: Vec<SpentTxo> = spends
                .iter()
                .map(|(outpoint, spending_tx, spending_input_index)| SpentTxo {
                    outpoint: *outpoint,
                    spending_tx: *spending_tx,
                    spending_input_index: *spending_input_index,
                })
                .collect();
            self.chain
                .add_block(block, &outputs, &spends)
                .await
                .unwrap();
        }

        async fn outputs(&self, addresses: &[&Address]) -> Vec<(OutPoint, Address, u64, u64)> {
            let addresses: Vec<Address> = addresses.iter().copied().cloned().collect();
            let txos = self.chain.get_txos_for_addresses(&addresses).await.unwrap();
            let mut outputs: Vec<_> = txos
                .into_iter()
                .map(|txo| {
                    let script = &txo.tx_out.script_pubkey;
                    (
                        txo.outpoint,
                        Address::from_script(script, Network::Regtest).unwrap(),
                        txo.tx_out.value.to_sat(),
                        txo.block_height,
                    )
                })
                .collect();
            outputs.sort();
            outputs
        }

        async fn spenders(&self, outpoints: &[OutPoint]) -> Vec<Spender> {
            let mut spenders = self.chain.get_spenders(outpoints).await.unwrap();
            spenders.sort_by_key(|spender| spender.outpoint);
            spenders
        }

        async fn hashes(&self, height: u64) -> Vec<BlockHash> {
            let mut hashes = self.chain.get_block_hashes(height).await.unwrap();
            hashes.sort();
            hashes
        }

        async fn watched_among(&self, addresses: &[&Address]) -> HashSet<Address> {
            let addresses: Vec<Address> = addresses.iter().copied().cloned().collect();
            let watched = self.chain.filter_watch_addresses(&addresses).await.unwrap();
            watched.into_iter().collect()
        }
    }

    fn address(byte: u8) -> Address {
        let secp = Secp256k1::new();
        let key = SecretKey::from_slice(&[byte; 32]).unwrap();
        let (internal_key, _) = key.public_key(&secp).x_only_public_key();
        Address::p2tr(&secp, internal_key, None, Network::Regtest)
    }

    fn block(byte: u8, height: u64) -> BlockHeader {
        BlockHeader {
            hash: BlockHash::from_byte_array([byte; 32]),
            height,
        }
    }

    fn txid(byte: u8) -> Txid {
        Txid::from_byte_array([byte; 32])
    }

    fn outpoint(byte: u8, vout: u32) -> OutPoint {
        OutPoint::new(txid(byte), vout)
    }

    fn spender(outpoint: OutPoint, txid: Txid, block_height: u64) -> Spender {
        Spender {
            outpoint,
            txid,
            block_height,
        }
    }

    #[tokio::test]
    async fn a_chain_record_without_blocks_has_no_tip_and_no_first_block() {
        let fixture = Fixture::new().await;

        assert_eq!(fixture.chain.get_tip().await.unwrap(), None);
        assert_eq!(fixture.chain.get_base().await.unwrap(), None);
    }

    #[tokio::test]
    async fn the_tip_is_the_highest_block_and_the_first_block_the_lowest() {
        let fixture = Fixture::new().await;
        let (lowest, middle, highest) = (block(1, 100), block(2, 101), block(3, 102));
        for block in [&middle, &highest, &lowest] {
            fixture.add_block(block, &[], &[]).await;
        }

        assert_eq!(fixture.chain.get_tip().await.unwrap(), Some(highest));
        assert_eq!(fixture.chain.get_base().await.unwrap(), Some(lowest));
    }

    #[tokio::test]
    async fn the_blocks_at_a_height_are_all_returned() {
        let fixture = Fixture::new().await;
        let (alone, one, other) = (block(1, 100), block(2, 101), block(3, 101));
        for block in [&alone, &one, &other] {
            fixture.add_block(block, &[], &[]).await;
        }

        assert_eq!(fixture.hashes(100).await, [alone.hash]);
        assert_eq!(fixture.hashes(101).await, [one.hash, other.hash]);
        assert!(fixture.hashes(102).await.is_empty());
    }

    #[tokio::test]
    async fn a_block_added_again_stores_nothing_twice() {
        let fixture = Fixture::new().await;
        let address = address(1);
        let output = outpoint(1, 0);
        let block = block(1, 100);
        let paid = (output, &address, 5_000);
        let spend = (output, txid(2), 0);
        let watched = WatchedOutput {
            outpoint: output,
            input: None,
        };
        let mut db = fixture.chain.pool.acquire().await.unwrap();
        fixture
            .chain
            .watch_outputs(&mut db, &[watched])
            .await
            .unwrap();

        fixture
            .add_block(&block, &[paid, paid], &[spend, spend])
            .await;
        fixture.add_block(&block, &[paid], &[spend]).await;

        assert_eq!(fixture.hashes(100).await, [block.hash]);
        assert_eq!(
            fixture.outputs(&[&address]).await,
            [(output, address.clone(), 5_000, 100)]
        );
        assert_eq!(
            fixture.spenders(&[output]).await,
            [spender(output, txid(2), 100)]
        );
        let (spent_watched_outputs, _) = fixture.chain.spent_and_unchecked_outputs().await.unwrap();
        assert_eq!(spent_watched_outputs, [output]);
    }

    #[tokio::test]
    async fn the_block_below_is_the_tip_once_the_tip_is_undone() {
        let fixture = Fixture::new().await;
        let (first, below, tip) = (block(1, 100), block(2, 101), block(3, 102));
        for block in [&first, &below, &tip] {
            fixture.add_block(block, &[], &[]).await;
        }

        fixture.chain.undo_block(tip.hash).await.unwrap();

        assert_eq!(fixture.chain.get_tip().await.unwrap(), Some(below));
        assert_eq!(fixture.chain.get_base().await.unwrap(), Some(first));
        assert!(fixture.hashes(102).await.is_empty());
    }

    #[tokio::test]
    async fn undoing_one_of_two_blocks_at_a_height_keeps_the_other() {
        let fixture = Fixture::new().await;
        let (kept, undone) = (block(1, 100), block(2, 100));
        for block in [&kept, &undone] {
            fixture.add_block(block, &[], &[]).await;
        }

        fixture.chain.undo_block(undone.hash).await.unwrap();

        assert_eq!(fixture.hashes(100).await, [kept.hash]);
    }

    #[tokio::test]
    async fn undoing_a_block_the_record_does_not_have_changes_nothing() {
        let fixture = Fixture::new().await;
        let address = address(1);
        let output = outpoint(1, 0);
        let stored = block(1, 100);
        fixture
            .add_block(
                &stored,
                &[(output, &address, 5_000)],
                &[(output, txid(2), 0)],
            )
            .await;

        fixture.chain.undo_block(block(9, 100).hash).await.unwrap();

        assert_eq!(fixture.hashes(100).await, [stored.hash]);
        assert_eq!(
            fixture.outputs(&[&address]).await,
            [(output, address.clone(), 5_000, 100)]
        );
        assert_eq!(
            fixture.spenders(&[output]).await,
            [spender(output, txid(2), 100)]
        );
    }

    #[tokio::test]
    async fn the_outputs_at_several_addresses_are_read_each_with_its_own_address_and_height() {
        let fixture = Fixture::new().await;
        let (first, second) = (address(1), address(2));
        fixture
            .add_block(
                &block(1, 100),
                &[
                    (outpoint(1, 0), &first, 5_000),
                    (outpoint(1, 1), &second, 6_000),
                ],
                &[],
            )
            .await;
        fixture
            .add_block(
                &block(2, 101),
                &[
                    (outpoint(2, 0), &second, 7_000),
                    (outpoint(2, 1), &first, 8_000),
                ],
                &[],
            )
            .await;

        assert_eq!(
            fixture.outputs(&[&first, &second]).await,
            [
                (outpoint(1, 0), first.clone(), 5_000, 100),
                (outpoint(1, 1), second.clone(), 6_000, 100),
                (outpoint(2, 0), second.clone(), 7_000, 101),
                (outpoint(2, 1), first.clone(), 8_000, 101),
            ]
        );
    }

    #[tokio::test]
    async fn only_the_outputs_at_the_given_addresses_are_read() {
        let fixture = Fixture::new().await;
        let (given, other) = (address(1), address(2));
        fixture
            .add_block(
                &block(1, 100),
                &[
                    (outpoint(1, 0), &given, 5_000),
                    (outpoint(1, 1), &other, 6_000),
                ],
                &[],
            )
            .await;

        assert!(fixture.outputs(&[]).await.is_empty());
        assert_eq!(
            fixture.outputs(&[&given]).await,
            [(outpoint(1, 0), given.clone(), 5_000, 100)]
        );
    }

    #[tokio::test]
    async fn an_address_without_outputs_gets_none_whether_it_is_watched_or_not() {
        let fixture = Fixture::new().await;
        let (paid, watched, not_watched) = (address(1), address(2), address(3));
        fixture
            .chain
            .add_watch_addresses(&[paid.clone(), watched.clone()])
            .await
            .unwrap();
        fixture
            .add_block(&block(1, 100), &[(outpoint(1, 0), &paid, 5_000)], &[])
            .await;

        assert_eq!(
            fixture.outputs(&[&paid, &watched, &not_watched]).await,
            [(outpoint(1, 0), paid.clone(), 5_000, 100)]
        );
    }

    #[tokio::test]
    async fn an_address_given_twice_gets_each_of_its_outputs_once() {
        let fixture = Fixture::new().await;
        let address = address(1);
        fixture
            .add_block(
                &block(1, 100),
                &[
                    (outpoint(1, 0), &address, 5_000),
                    (outpoint(1, 1), &address, 6_000),
                ],
                &[],
            )
            .await;

        assert_eq!(
            fixture.outputs(&[&address, &address]).await,
            [
                (outpoint(1, 0), address.clone(), 5_000, 100),
                (outpoint(1, 1), address.clone(), 6_000, 100),
            ]
        );
    }

    #[tokio::test]
    async fn the_outputs_in_an_undone_block_are_no_longer_read() {
        let fixture = Fixture::new().await;
        let address = address(1);
        let (kept, undone) = (block(1, 100), block(2, 101));
        fixture
            .add_block(&kept, &[(outpoint(1, 0), &address, 5_000)], &[])
            .await;
        fixture
            .add_block(&undone, &[(outpoint(2, 0), &address, 6_000)], &[])
            .await;

        fixture.chain.undo_block(undone.hash).await.unwrap();

        assert_eq!(
            fixture.outputs(&[&address]).await,
            [(outpoint(1, 0), address.clone(), 5_000, 100)]
        );
    }

    #[tokio::test]
    async fn an_output_is_read_again_once_another_block_contains_its_transaction() {
        let fixture = Fixture::new().await;
        let address = address(1);
        let paid = (outpoint(1, 0), &address, 5_000);
        let undone = block(1, 100);
        fixture.add_block(&undone, &[paid], &[]).await;
        fixture.chain.undo_block(undone.hash).await.unwrap();

        fixture.add_block(&block(2, 103), &[paid], &[]).await;

        assert_eq!(
            fixture.outputs(&[&address]).await,
            [(outpoint(1, 0), address.clone(), 5_000, 103)]
        );
    }

    #[tokio::test]
    async fn the_spenders_of_several_outputs_are_read_each_with_the_height_of_its_block() {
        let fixture = Fixture::new().await;
        let address = address(1);
        let (first, second) = (outpoint(1, 0), outpoint(1, 1));
        fixture
            .add_block(
                &block(1, 100),
                &[(first, &address, 5_000), (second, &address, 6_000)],
                &[],
            )
            .await;
        fixture
            .add_block(&block(2, 101), &[], &[(first, txid(2), 0)])
            .await;
        fixture
            .add_block(&block(3, 102), &[], &[(second, txid(3), 4)])
            .await;

        assert_eq!(
            fixture.spenders(&[first, second]).await,
            [spender(first, txid(2), 101), spender(second, txid(3), 102)]
        );
    }

    #[tokio::test]
    async fn only_the_spent_among_the_given_outputs_have_a_spender() {
        let fixture = Fixture::new().await;
        let address = address(1);
        let (spent, unspent, not_given) = (outpoint(1, 0), outpoint(1, 1), outpoint(1, 2));
        let not_stored = outpoint(9, 0);
        fixture
            .add_block(
                &block(1, 100),
                &[
                    (spent, &address, 5_000),
                    (unspent, &address, 6_000),
                    (not_given, &address, 7_000),
                ],
                &[],
            )
            .await;
        fixture
            .add_block(
                &block(2, 101),
                &[],
                &[(spent, txid(2), 0), (not_given, txid(3), 0)],
            )
            .await;

        assert!(fixture.spenders(&[]).await.is_empty());
        assert!(fixture.spenders(&[unspent, not_stored]).await.is_empty());
        assert_eq!(
            fixture.spenders(&[spent, unspent, not_stored]).await,
            [spender(spent, txid(2), 101)]
        );
    }

    #[tokio::test]
    async fn the_spender_of_an_outpoint_is_returned_once_for_each_time_the_outpoint_is_given() {
        let fixture = Fixture::new().await;
        let address = address(1);
        let output = outpoint(1, 0);
        fixture
            .add_block(&block(1, 100), &[(output, &address, 5_000)], &[])
            .await;
        fixture
            .add_block(&block(2, 101), &[], &[(output, txid(2), 0)])
            .await;

        let spenders = fixture.spenders(&[output, output]).await;

        let once = spender(output, txid(2), 101);
        assert_eq!(spenders, [once, once]);
    }

    #[tokio::test]
    async fn the_spend_of_an_output_the_record_does_not_have_is_not_stored() {
        let fixture = Fixture::new().await;
        let not_stored = outpoint(1, 0);

        fixture
            .add_block(&block(1, 100), &[], &[(not_stored, txid(2), 0)])
            .await;

        assert!(fixture.spenders(&[not_stored]).await.is_empty());
    }

    #[tokio::test]
    async fn an_output_spent_in_the_block_that_contains_it_has_its_spender() {
        let fixture = Fixture::new().await;
        let address = address(1);
        let output = outpoint(1, 0);

        fixture
            .add_block(
                &block(1, 100),
                &[(output, &address, 5_000)],
                &[(output, txid(2), 0)],
            )
            .await;

        assert_eq!(
            fixture.spenders(&[output]).await,
            [spender(output, txid(2), 100)]
        );
    }

    #[tokio::test]
    async fn a_transaction_is_the_spender_of_each_output_among_its_inputs() {
        let fixture = Fixture::new().await;
        let address = address(1);
        let (first, second) = (outpoint(1, 0), outpoint(2, 0));
        fixture
            .add_block(
                &block(1, 100),
                &[(first, &address, 5_000), (second, &address, 6_000)],
                &[],
            )
            .await;

        fixture
            .add_block(
                &block(2, 101),
                &[],
                &[(first, txid(3), 0), (second, txid(3), 1)],
            )
            .await;

        assert_eq!(
            fixture.spenders(&[first, second]).await,
            [spender(first, txid(3), 101), spender(second, txid(3), 101)]
        );
    }

    #[tokio::test]
    async fn the_output_and_the_spend_of_one_transaction_are_both_stored() {
        let fixture = Fixture::new().await;
        let address = address(1);
        let (spent, change) = (outpoint(1, 0), outpoint(2, 0));
        fixture
            .add_block(&block(1, 100), &[(spent, &address, 5_000)], &[])
            .await;

        fixture
            .add_block(
                &block(2, 101),
                &[(change, &address, 4_000)],
                &[(spent, change.txid, 0)],
            )
            .await;

        assert_eq!(
            fixture.spenders(&[spent]).await,
            [spender(spent, change.txid, 101)]
        );
        assert_eq!(
            fixture.outputs(&[&address]).await,
            [
                (spent, address.clone(), 5_000, 100),
                (change, address.clone(), 4_000, 101),
            ]
        );
    }

    #[tokio::test]
    async fn the_spend_in_an_undone_block_is_no_longer_read() {
        let fixture = Fixture::new().await;
        let address = address(1);
        let output = outpoint(1, 0);
        let undone = block(2, 101);
        fixture
            .add_block(&block(1, 100), &[(output, &address, 5_000)], &[])
            .await;
        fixture
            .add_block(&undone, &[], &[(output, txid(2), 0)])
            .await;

        fixture.chain.undo_block(undone.hash).await.unwrap();

        assert!(fixture.spenders(&[output]).await.is_empty());
        assert_eq!(
            fixture.outputs(&[&address]).await,
            [(output, address.clone(), 5_000, 100)]
        );
    }

    #[tokio::test]
    async fn a_spend_is_read_again_once_another_block_contains_its_transaction() {
        let fixture = Fixture::new().await;
        let address = address(1);
        let output = outpoint(1, 0);
        let spend = (output, txid(2), 0);
        let undone = block(2, 101);
        fixture
            .add_block(&block(1, 100), &[(output, &address, 5_000)], &[])
            .await;
        fixture.add_block(&undone, &[], &[spend]).await;
        fixture.chain.undo_block(undone.hash).await.unwrap();

        fixture.add_block(&block(3, 104), &[], &[spend]).await;

        assert_eq!(
            fixture.spenders(&[output]).await,
            [spender(output, txid(2), 104)]
        );
    }

    #[tokio::test]
    async fn the_spender_is_the_transaction_that_confirmed_after_another_ones_block_was_undone() {
        let fixture = Fixture::new().await;
        let address = address(1);
        let output = outpoint(1, 0);
        let undone = block(2, 101);
        fixture
            .add_block(&block(1, 100), &[(output, &address, 5_000)], &[])
            .await;
        fixture
            .add_block(&undone, &[], &[(output, txid(2), 0)])
            .await;
        fixture.chain.undo_block(undone.hash).await.unwrap();

        fixture
            .add_block(&block(3, 101), &[], &[(output, txid(3), 0)])
            .await;

        assert_eq!(
            fixture.spenders(&[output]).await,
            [spender(output, txid(3), 101)]
        );
    }

    #[tokio::test]
    async fn the_chain_record_watches_the_addresses_it_is_given() {
        let fixture = Fixture::new().await;
        let (first, second, third) = (address(1), address(2), address(3));
        let all = [&first, &second, &third];

        fixture.chain.add_watch_addresses(&[]).await.unwrap();
        assert!(fixture.watched_among(&all).await.is_empty());

        fixture
            .chain
            .add_watch_addresses(std::slice::from_ref(&first))
            .await
            .unwrap();
        assert_eq!(
            fixture.watched_among(&all).await,
            HashSet::from([first.clone()])
        );

        fixture
            .chain
            .add_watch_addresses(&[second.clone(), third.clone()])
            .await
            .unwrap();
        assert_eq!(
            fixture.watched_among(&all).await,
            HashSet::from([first.clone(), second.clone(), third.clone()])
        );
    }

    #[tokio::test]
    async fn only_the_watched_among_the_given_addresses_are_returned() {
        let fixture = Fixture::new().await;
        let (watched, not_given, not_watched) = (address(1), address(2), address(3));
        fixture
            .chain
            .add_watch_addresses(&[watched.clone(), not_given])
            .await
            .unwrap();

        assert!(fixture.watched_among(&[]).await.is_empty());
        assert!(fixture.watched_among(&[&not_watched]).await.is_empty());
        assert_eq!(
            fixture.watched_among(&[&watched, &not_watched]).await,
            HashSet::from([watched.clone()])
        );
    }

    #[tokio::test]
    async fn an_address_watched_again_is_watched_once() {
        let fixture = Fixture::new().await;
        let address = address(1);
        let twice = [address.clone(), address.clone()];

        fixture.chain.add_watch_addresses(&twice).await.unwrap();
        fixture
            .chain
            .add_watch_addresses(std::slice::from_ref(&address))
            .await
            .unwrap();

        let watched = fixture.chain.filter_watch_addresses(&twice).await.unwrap();
        assert_eq!(watched, [address]);
    }
}
