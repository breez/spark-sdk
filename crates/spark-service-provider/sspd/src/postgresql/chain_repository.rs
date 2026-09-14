#![allow(clippy::needless_raw_string_hashes)]

use std::sync::Arc;

use bitcoin::{Address, Amount, BlockHash, Network, OutPoint, TxOut, address::NetworkUnchecked};
use futures::TryStreamExt;
use sqlx::{PgConnection, PgPool, Row, postgres::PgRow};
use tracing::instrument;

use crate::chain::{self, AddressUtxo, BlockHeader, ChainRepositoryError, SpentTxo, Txo};

#[derive(Debug)]
pub struct ChainRepository {
    network: Network,
    pool: Arc<PgPool>,
}

impl ChainRepository {
    pub fn new(pool: Arc<PgPool>, network: Network) -> Self {
        Self { network, pool }
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
            .map(|u| i64::from(u.utxo.outpoint.vout))
            .collect();
        let addresses: Vec<_> = tx_outputs.iter().map(|u| u.address.to_string()).collect();
        let amounts: Vec<_> = tx_outputs
            .iter()
            .map(|u| u.utxo.tx_out.value.to_sat().cast_signed())
            .collect();
        sqlx::query(
            r#"INSERT INTO brz_ssp_tx_outputs (
                   tx_id
               ,   output_index
               ,   address
               ,   amount)
               SELECT t.tx_id, t.output_index, t.address, t.amount
               FROM UNNEST(
                   $1::text[]
               ,   $2::bigint[]
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
        let output_index: i64 = row.try_get("output_index")?;
        let amount: i64 = row.try_get("amount")?;
        let height: i64 = row.try_get("height")?;
        Ok(Txo {
            block_height: height.cast_unsigned(),
            outpoint: OutPoint::new(tx_id.parse()?, u32::try_from(output_index).unwrap_or(0)),
            tx_out: TxOut {
                value: Amount::from_sat(amount.cast_unsigned()),
                script_pubkey: address.script_pubkey(),
            },
        })
    }

    #[instrument(level = "trace", skip(self))]
    async fn mark_spent(
        &self,
        tx: &mut PgConnection,
        txos: &[SpentTxo],
    ) -> Result<(), ChainRepositoryError> {
        let tx_ids: Vec<_> = txos.iter().map(|u| u.outpoint.txid.to_string()).collect();
        let tx_output_indices: Vec<_> = txos.iter().map(|u| i64::from(u.outpoint.vout)).collect();
        let spending_tx_ids: Vec<_> = txos.iter().map(|u| u.spending_tx.to_string()).collect();
        let spending_tx_input_indices: Vec<_> = txos
            .iter()
            .map(|u| i64::from(u.spending_input_index))
            .collect();

        sqlx::query(
            r#"INSERT INTO brz_ssp_tx_inputs (
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
               ,   $2::bigint[]
               ,   $3::text[]
               ,   $4::bigint[]
               ) AS i (
                   tx_id
               ,   output_index
               ,   spending_tx_id
               ,   spending_input_index)
               INNER JOIN brz_ssp_tx_outputs o
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
            r#"INSERT INTO brz_ssp_blocks (block_hash, height)
               VALUES ($1, $2)
               ON CONFLICT DO NOTHING"#,
        )
        .bind(block.hash.to_string())
        .bind(block.height.cast_signed())
        .execute(&mut *tx)
        .await?;

        self.add_utxos(&mut tx, tx_outputs).await?;

        self.mark_spent(&mut tx, tx_inputs).await?;

        let txns: Vec<_> = tx_outputs
            .iter()
            .map(|o| o.utxo.outpoint.txid.to_string())
            .chain(tx_inputs.iter().map(|i| i.spending_tx.to_string()))
            .collect();
        let block_hashes: Vec<_> = txns.iter().map(|_| block.hash.to_string()).collect();
        sqlx::query(
            r#"INSERT INTO brz_ssp_tx_blocks
               SELECT i.tx_id
               ,      i.block_hash
               FROM UNNEST(
                   $1::text[]
               ,   $2::text[]
               ) AS i (
                   tx_id
               ,   block_hash)
               WHERE EXISTS (SELECT 1
                             FROM brz_ssp_tx_outputs o
                             WHERE o.tx_id = i.tx_id)
                   OR EXISTS (SELECT 1
                              FROM brz_ssp_tx_inputs ti
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
    async fn add_watch_address(&self, address: &Address) -> Result<(), ChainRepositoryError> {
        sqlx::query(
            r#"INSERT INTO brz_ssp_watch_addresses (address)
               VALUES ($1)
               ON CONFLICT DO NOTHING"#,
        )
        .bind(address.to_string())
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
               FROM brz_ssp_watch_addresses
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
            sqlx::query_as("SELECT block_hash FROM brz_ssp_blocks WHERE height = $1")
                .bind(height.cast_signed())
                .fetch_all(&*self.pool)
                .await?;
        rows.into_iter().map(|(hash,)| Ok(hash.parse()?)).collect()
    }

    #[instrument(level = "trace", skip(self))]
    async fn get_tip(&self) -> Result<Option<BlockHeader>, ChainRepositoryError> {
        self.first_block(
            "SELECT block_hash, height FROM brz_ssp_blocks ORDER BY height DESC LIMIT 1",
        )
        .await
    }

    #[instrument(level = "trace", skip(self))]
    async fn get_base(&self) -> Result<Option<BlockHeader>, ChainRepositoryError> {
        self.first_block("SELECT block_hash, height FROM brz_ssp_blocks ORDER BY height LIMIT 1")
            .await
    }

    #[instrument(level = "trace", skip(self))]
    async fn get_txos_for_address(
        &self,
        address: &Address,
    ) -> Result<Vec<Txo>, ChainRepositoryError> {
        let mut rows = sqlx::query(
            r#"SELECT o.tx_id
            ,         o.output_index
            ,         o.amount
            ,         b.height
            FROM brz_ssp_tx_outputs o
            INNER JOIN brz_ssp_tx_blocks tb ON tb.tx_id = o.tx_id
            INNER JOIN brz_ssp_blocks b ON tb.block_hash = b.block_hash
            WHERE o.address = $1
            ORDER BY b.height, o.tx_id, o.output_index"#,
        )
        .bind(address.to_string())
        .fetch(&*self.pool);

        let mut result: Vec<Txo> = Vec::new();
        while let Some(row) = rows.try_next().await? {
            let txo = Self::map_txo(address, &row)?;
            result.push(txo);
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
        let output_indices: Vec<i64> = outpoints.iter().map(|o| i64::from(o.vout)).collect();

        let rows: Vec<(String, i64, String, i64)> = sqlx::query_as(
            r#"SELECT i.tx_id
               ,      i.output_index
               ,      i.spending_tx_id
               ,      b.height
               FROM brz_ssp_tx_inputs i
               INNER JOIN brz_ssp_tx_blocks tb ON tb.tx_id = i.spending_tx_id
               INNER JOIN brz_ssp_blocks b ON b.block_hash = tb.block_hash
               INNER JOIN UNNEST($1::text[], $2::bigint[]) AS o(tx_id, output_index)
                   ON o.tx_id = i.tx_id AND o.output_index = i.output_index"#,
        )
        .bind(&tx_ids)
        .bind(&output_indices)
        .fetch_all(&*self.pool)
        .await?;

        rows.into_iter()
            .map(|(tx_id, output_index, spending_tx_id, height)| {
                Ok(chain::Spender {
                    outpoint: OutPoint::new(
                        tx_id.parse()?,
                        u32::try_from(output_index).unwrap_or(0),
                    ),
                    txid: spending_tx_id.parse()?,
                    block_height: height.cast_unsigned(),
                })
            })
            .collect()
    }

    #[instrument(level = "trace", skip(self))]
    async fn undo_block(&self, hash: BlockHash) -> Result<(), ChainRepositoryError> {
        sqlx::query("DELETE FROM brz_ssp_blocks WHERE block_hash = $1")
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
