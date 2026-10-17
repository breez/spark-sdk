use std::collections::HashMap;

use bitcoin::{OutPoint, Txid};
use sqlx::postgres::PgRow;
use sqlx::{PgConnection, Row};

use super::ChainRepository;
use crate::chain::{
    ChainRepositoryError, ConfirmedOutput, OutputSpend, OutputStates, Spender, WatchedOutput,
};
use crate::wakeup::Wakeup;

fn ids(outpoints: &[OutPoint]) -> (Vec<String>, Vec<i32>) {
    outpoints
        .iter()
        .map(|outpoint| (outpoint.txid.to_string(), outpoint.vout.cast_signed()))
        .unzip()
}

fn outpoint(tx_id: &str, output_index: i32) -> Result<OutPoint, ChainRepositoryError> {
    Ok(OutPoint::new(tx_id.parse()?, output_index.cast_unsigned()))
}

const OUTPUT_STATES: &str = r"SELECT o.tx_id, o.output_index, spent.spending_tx_id, spent.height,
              queued.tx_id IS NOT NULL AS unchecked
       FROM UNNEST($1::text[], $2::int[]) AS o (tx_id, output_index)
       LEFT JOIN brz_ssp_chain_spent_outputs spent
         ON spent.tx_id = o.tx_id AND spent.output_index = o.output_index
       LEFT JOIN brz_ssp_chain_unchecked_outputs queued
         ON queued.tx_id = o.tx_id AND queued.output_index = o.output_index
       WHERE spent.tx_id IS NOT NULL OR queued.tx_id IS NOT NULL";

fn read_state(states: &mut OutputStates, row: &PgRow) -> Result<(), ChainRepositoryError> {
    let outpoint = outpoint(row.try_get("tx_id")?, row.try_get("output_index")?)?;
    let spender: Option<String> = row.try_get("spending_tx_id")?;
    let height: Option<i64> = row.try_get("height")?;
    if let (Some(spender), Some(height)) = (spender, height) {
        states.spent.insert(
            outpoint,
            Spender {
                outpoint,
                txid: spender.parse()?,
                block_height: height.cast_unsigned(),
            },
        );
    }
    if row.try_get("unchecked")? {
        states.unchecked.insert(outpoint);
    }
    Ok(())
}

impl ChainRepository {
    /// Watches each of `outputs` that is not watched yet, and queues it for the
    /// spend check. It runs on the caller's connection, so that Postgres commits
    /// the new rows with the caller's change. After that commit, the caller wakes
    /// the spend check through [`Self::watched`].
    pub async fn watch_outputs(
        &self,
        db: &mut PgConnection,
        outputs: &[WatchedOutput],
    ) -> Result<(), ChainRepositoryError> {
        if outputs.is_empty() {
            return Ok(());
        }
        let (tx_ids, output_indices) = ids(&outputs.iter().map(|o| o.outpoint).collect::<Vec<_>>());
        let input_tx_ids: Vec<Option<String>> = outputs
            .iter()
            .map(|o| o.input.map(|input| input.txid.to_string()))
            .collect();
        let input_output_indices: Vec<Option<i32>> = outputs
            .iter()
            .map(|o| o.input.map(|input| input.vout.cast_signed()))
            .collect();
        // Postgres inserts the rows in sorted order, so two callers that insert
        // the same outputs take their locks in the same order and never deadlock.
        sqlx::query(
            r"WITH watched AS (
                   INSERT INTO brz_ssp_chain_watch_outputs
                       (tx_id, output_index, input_tx_id, input_output_index)
                   SELECT DISTINCT ON (tx_id, output_index)
                          tx_id, output_index, input_tx_id, input_output_index
                   FROM UNNEST($1::text[], $2::int[], $3::text[], $4::int[])
                        AS o (tx_id, output_index, input_tx_id, input_output_index)
                   ORDER BY tx_id, output_index, input_tx_id
                   ON CONFLICT (tx_id, output_index) DO NOTHING
                   RETURNING tx_id, output_index
               )
               INSERT INTO brz_ssp_chain_unchecked_outputs (tx_id, output_index)
               SELECT tx_id, output_index FROM watched",
        )
        .bind(&tx_ids)
        .bind(&output_indices)
        .bind(&input_tx_ids)
        .bind(&input_output_indices)
        .execute(db)
        .await?;
        Ok(())
    }

    pub fn watched(&self) -> Wakeup {
        self.watched.clone()
    }

    /// An outpoint in neither set of the result has no spend on chain, or is
    /// not watched.
    pub async fn output_states(
        &self,
        db: &mut PgConnection,
        outpoints: &[OutPoint],
    ) -> Result<OutputStates, ChainRepositoryError> {
        let mut states = OutputStates::default();
        if outpoints.is_empty() {
            return Ok(states);
        }
        let (tx_ids, output_indices) = ids(outpoints);
        let rows = sqlx::query(OUTPUT_STATES)
            .bind(&tx_ids)
            .bind(&output_indices)
            .fetch_all(db)
            .await?;
        for row in &rows {
            read_state(&mut states, row)?;
        }
        Ok(states)
    }

    pub async fn spent_and_unchecked_outputs(
        &self,
    ) -> Result<(Vec<OutPoint>, Vec<OutPoint>), ChainRepositoryError> {
        let rows: Vec<(String, i32, bool)> = sqlx::query_as(
            r"SELECT tx_id, output_index, TRUE FROM brz_ssp_chain_spent_outputs
             UNION ALL
               SELECT tx_id, output_index, FALSE FROM brz_ssp_chain_unchecked_outputs",
        )
        .fetch_all(&*self.pool)
        .await?;
        let (mut spent, mut unchecked) = (Vec::new(), Vec::new());
        for (tx_id, index, is_spent) in &rows {
            let outputs = if *is_spent {
                &mut spent
            } else {
                &mut unchecked
            };
            outputs.push(outpoint(tx_id, *index)?);
        }
        Ok((spent, unchecked))
    }

    /// The watched outputs that are on chain and unspent. An output is in the
    /// list when the record has its transaction as the confirmed spend of the
    /// input that [`Self::watch_outputs`] stored with it, the spend check has
    /// checked it, and the record has no spend of it.
    pub async fn confirmed_unspent_outputs(
        &self,
    ) -> Result<Vec<ConfirmedOutput>, ChainRepositoryError> {
        let rows: Vec<(String, i32, i64)> = sqlx::query_as(
            r"SELECT w.tx_id, w.output_index, confirmed.height
               FROM brz_ssp_chain_spent_outputs confirmed
               JOIN brz_ssp_chain_watch_outputs w
                 ON w.input_tx_id = confirmed.tx_id
                AND w.input_output_index = confirmed.output_index
                AND w.tx_id = confirmed.spending_tx_id
               WHERE NOT EXISTS (
                         SELECT 1 FROM brz_ssp_chain_spent_outputs spent
                         WHERE spent.tx_id = w.tx_id AND spent.output_index = w.output_index)
                 AND NOT EXISTS (
                         SELECT 1 FROM brz_ssp_chain_unchecked_outputs queued
                         WHERE queued.tx_id = w.tx_id AND queued.output_index = w.output_index)
               ORDER BY confirmed.height, w.tx_id, w.output_index",
        )
        .fetch_all(&*self.pool)
        .await?;
        rows.iter()
            .map(|(tx_id, index, height)| {
                Ok(ConfirmedOutput {
                    outpoint: outpoint(tx_id, *index)?,
                    height: height.cast_unsigned(),
                })
            })
            .collect()
    }

    /// Returns what [`Self::output_states`] returns for `outpoints`, and the
    /// height of each of `txids` that is in a block of the chain record.
    pub async fn output_states_and_tx_heights(
        &self,
        db: &mut PgConnection,
        outpoints: &[OutPoint],
        txids: &[Txid],
    ) -> Result<(OutputStates, HashMap<Txid, u64>), ChainRepositoryError> {
        let (tx_ids, output_indices) = ids(outpoints);
        let block_tx_ids: Vec<String> = txids.iter().map(ToString::to_string).collect();
        let rows = sqlx::query(&format!(
            r"SELECT states.*, NULL::bigint AS tx_height FROM ({OUTPUT_STATES}) states
             UNION ALL
               SELECT tb.tx_id, NULL, NULL, NULL, FALSE, b.height
               FROM brz_ssp_chain_tx_blocks tb
               JOIN brz_ssp_chain_blocks b ON b.block_hash = tb.block_hash
               WHERE tb.tx_id = ANY($3)"
        ))
        .bind(&tx_ids)
        .bind(&output_indices)
        .bind(&block_tx_ids)
        .fetch_all(db)
        .await?;
        let mut states = OutputStates::default();
        let mut heights = HashMap::new();
        for row in &rows {
            let tx_height: Option<i64> = row.try_get("tx_height")?;
            match tx_height {
                Some(height) => {
                    let tx_id: String = row.try_get("tx_id")?;
                    heights.insert(tx_id.parse()?, height.cast_unsigned());
                }
                None => read_state(&mut states, row)?,
            }
        }
        Ok((states, heights))
    }

    /// The height of each of `txids` that is in a block of the chain record.
    pub async fn tx_heights(
        &self,
        txids: &[Txid],
    ) -> Result<HashMap<Txid, u64>, ChainRepositoryError> {
        let tx_ids: Vec<String> = txids.iter().map(ToString::to_string).collect();
        let rows: Vec<(String, i64)> = sqlx::query_as(
            r"SELECT tb.tx_id, b.height
               FROM brz_ssp_chain_tx_blocks tb
               JOIN brz_ssp_chain_blocks b ON b.block_hash = tb.block_hash
               WHERE tb.tx_id = ANY($1)",
        )
        .bind(&tx_ids)
        .fetch_all(&*self.pool)
        .await?;
        rows.iter()
            .map(|(tx_id, height)| Ok((tx_id.parse()?, height.cast_unsigned())))
            .collect()
    }

    /// Up to `limit` queued outputs that the spend check can query bitcoind for
    /// now. It leaves out an output while the input of the output's transaction
    /// is queued, because the spend check first has to find out whether that
    /// transaction is confirmed.
    pub async fn next_unchecked_outputs(
        &self,
        limit: u32,
    ) -> Result<Vec<OutPoint>, ChainRepositoryError> {
        let rows: Vec<(String, i32)> = sqlx::query_as(
            r"SELECT queued.tx_id, queued.output_index
               FROM brz_ssp_chain_unchecked_outputs queued
               JOIN brz_ssp_chain_watch_outputs w
                 ON w.tx_id = queued.tx_id AND w.output_index = queued.output_index
               WHERE NOT EXISTS (
                   SELECT 1 FROM brz_ssp_chain_unchecked_outputs queued_input
                   WHERE queued_input.tx_id = w.input_tx_id
                     AND queued_input.output_index = w.input_output_index)
               ORDER BY queued.tx_id, queued.output_index
               LIMIT $1",
        )
        .bind(i64::from(limit))
        .fetch_all(&*self.pool)
        .await?;
        rows.iter()
            .map(|(tx_id, index)| outpoint(tx_id, *index))
            .collect()
    }

    /// Removes from the queue each output of an unconfirmed transaction and of
    /// its descendants: no block contains such an output, so no block contains
    /// a spend of it. It treats a transaction as unconfirmed when the spend
    /// check has checked the transaction's watched input and the record has no
    /// spend of that input by that transaction. Returns how many outputs it
    /// removed.
    pub async fn drop_unconfirmed_outputs(&self) -> Result<u64, ChainRepositoryError> {
        let dropped = sqlx::query(
            r"WITH RECURSIVE unconfirmed AS (
                   SELECT queued.tx_id, queued.output_index
                   FROM brz_ssp_chain_unchecked_outputs queued
                   JOIN brz_ssp_chain_watch_outputs w
                     ON w.tx_id = queued.tx_id AND w.output_index = queued.output_index
                   JOIN brz_ssp_chain_watch_outputs input
                     ON input.tx_id = w.input_tx_id AND input.output_index = w.input_output_index
                   WHERE NOT EXISTS (
                       SELECT 1 FROM brz_ssp_chain_unchecked_outputs queued_input
                       WHERE queued_input.tx_id = input.tx_id
                         AND queued_input.output_index = input.output_index)
                     AND NOT EXISTS (
                       SELECT 1 FROM brz_ssp_chain_spent_outputs spent
                       WHERE spent.tx_id = input.tx_id
                         AND spent.output_index = input.output_index
                         AND spent.spending_tx_id = queued.tx_id)
                 UNION
                   SELECT queued.tx_id, queued.output_index
                   FROM unconfirmed
                   JOIN brz_ssp_chain_watch_outputs w
                     ON w.input_tx_id = unconfirmed.tx_id
                    AND w.input_output_index = unconfirmed.output_index
                   JOIN brz_ssp_chain_unchecked_outputs queued
                     ON queued.tx_id = w.tx_id AND queued.output_index = w.output_index
               )
               DELETE FROM brz_ssp_chain_unchecked_outputs queued
               USING unconfirmed
               WHERE queued.tx_id = unconfirmed.tx_id
                 AND queued.output_index = unconfirmed.output_index",
        )
        .execute(&*self.pool)
        .await?
        .rows_affected();
        Ok(dropped)
    }

    /// Removes `checked` from the queue and records the confirmed spends among
    /// `spends`, which bitcoind reported for them. It records a spend with its
    /// block when the chain record has that block, and without a block when the
    /// block is older than the record. It skips a spend in any other block:
    /// bitcoind has dropped that block from its chain since the spend check
    /// queried bitcoind. Returns how many spends it recorded.
    pub async fn record_checked_outputs(
        &self,
        checked: &[OutPoint],
        spends: &[OutputSpend],
    ) -> Result<u64, ChainRepositoryError> {
        let confirmed: Vec<_> = spends
            .iter()
            .filter_map(|spend| Some((spend, spend.block.as_ref()?)))
            .collect();
        let (spent_ids, spent_indices) = ids(&confirmed
            .iter()
            .map(|(s, _)| s.outpoint)
            .collect::<Vec<_>>());
        let spenders: Vec<String> = confirmed
            .iter()
            .map(|(spend, _)| spend.spender.to_string())
            .collect();
        let blocks: Vec<String> = confirmed
            .iter()
            .map(|(_, block)| block.hash.to_string())
            .collect();
        let heights: Vec<i64> = confirmed
            .iter()
            .map(|(_, block)| block.height.cast_signed())
            .collect();
        let (checked_ids, checked_indices) = ids(checked);

        // Postgres records the spends and removes the outputs from the queue in
        // one statement, so it commits both or neither.
        let recorded: i64 = sqlx::query_scalar(
            r"WITH recorded AS (
                   INSERT INTO brz_ssp_chain_spent_outputs
                       (tx_id, output_index, spending_tx_id, block_hash, height)
                   SELECT spend.tx_id, spend.output_index, spend.spending_tx_id,
                          block.block_hash, spend.height
                   FROM UNNEST($1::text[], $2::int[], $3::text[], $4::text[], $5::bigint[])
                        AS spend (tx_id, output_index, spending_tx_id, block_hash, height)
                   LEFT JOIN brz_ssp_chain_blocks block ON block.block_hash = spend.block_hash
                   WHERE block.block_hash IS NOT NULL
                      OR spend.height < (SELECT MIN(height) FROM brz_ssp_chain_blocks)
                   ON CONFLICT (tx_id, output_index) DO NOTHING
                   RETURNING tx_id
               ), dequeued AS (
                   DELETE FROM brz_ssp_chain_unchecked_outputs queued
                   USING UNNEST($6::text[], $7::int[]) AS checked (tx_id, output_index)
                   WHERE queued.tx_id = checked.tx_id
                     AND queued.output_index = checked.output_index
               )
               SELECT COUNT(*) FROM recorded",
        )
        .bind(&spent_ids)
        .bind(&spent_indices)
        .bind(&spenders)
        .bind(&blocks)
        .bind(&heights)
        .bind(&checked_ids)
        .bind(&checked_indices)
        .fetch_one(&*self.pool)
        .await?;
        Ok(recorded.cast_unsigned())
    }
}
