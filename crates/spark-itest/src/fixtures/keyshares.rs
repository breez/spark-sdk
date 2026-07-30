//! Pre-generated signing keyshares for the local operator cluster.
//!
//! Operators fill their keyshare pool by running DKG: every operator
//! coordinates its own round, each round involves all three operators, and an
//! operator serializes its rounds behind one lock. Several clusters running at
//! once outrun a CI runner, and a round that stalls is not retried until the
//! operator's own task timeout expires, so a cluster can sit without usable
//! keyshares for minutes.
//!
//! The cluster is deterministic (fixed operator keys, identifiers, indices and
//! threshold), so keyshares captured from one real DKG stay valid for every
//! later cluster. Each operator database is seeded from
//! `keyshares/operator-<index>.tsv` before the operator boots, which leaves
//! every coordinator's pool above `dkg.min_available_keys` and keeps the DKG
//! task idle.
//!
//! Regenerate the files with `make capture-itest-keyshares` after changing the
//! pinned operator version, the operator count or the threshold.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use tokio_postgres::NoTls;
use tracing::warn;

use crate::fixtures::spark_so::NUM_OPERATORS;

/// The committed seed for each operator, in PostgreSQL `COPY` text format.
/// Embedded so a cluster cannot start against a missing or half-written file,
/// and so cargo rebuilds the fixtures after a capture.
const SEEDS: [&str; NUM_OPERATORS] = [
    include_str!("../../keyshares/operator-0.tsv"),
    include_str!("../../keyshares/operator-1.tsv"),
    include_str!("../../keyshares/operator-2.tsv"),
];

/// Columns carried by a seed file. Naming them explicitly keeps the seed
/// loadable against a schema that gained unrelated columns, and turns an
/// incompatible schema change into a failed `COPY` rather than a wrong row.
const SEED_COLUMNS: &str = "id, create_time, update_time, status, secret_share, secret_version, \
                            public_shares, public_key, min_signers, coordinator_index";

/// Must match `dkg.min_available_keys` in `docker/so.config.yaml`. An operator
/// runs DKG whenever its own pool is not strictly above this, so a seeded pool
/// has to clear it to keep the DKG task idle.
pub const MIN_AVAILABLE_KEYS: usize = 100;

fn seed_path(operator_index: usize) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("keyshares")
        .join(format!("operator-{operator_index}.tsv"))
}

async fn connect(connection_string: &str) -> Result<tokio_postgres::Client> {
    let (client, connection) = tokio_postgres::connect(connection_string, NoTls).await?;
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            warn!("Keyshare database connection error: {e}");
        }
    });
    Ok(client)
}

/// Load the committed seed into a freshly migrated operator database. Returns
/// the number of keyshares written.
///
/// Runs after the migrations container has exited and before the operator
/// starts, so the operator's first DKG check already sees a full pool.
pub async fn load_seed(connection_string: &str, operator_index: usize) -> Result<u64> {
    let seed = SEEDS
        .get(operator_index)
        .with_context(|| format!("no committed keyshare seed for operator {operator_index}"))?;
    if seed.trim().is_empty() {
        bail!(
            "keyshare seed for operator {operator_index} is empty; run `make capture-itest-keyshares`"
        );
    }

    let client = connect(connection_string).await?;
    let sink = client
        .copy_in(&format!(
            "COPY signing_keyshares ({SEED_COLUMNS}) FROM STDIN"
        ))
        .await
        .with_context(|| format!("failed to start keyshare COPY for operator {operator_index}"))?;
    futures::pin_mut!(sink);
    sink.send(Bytes::from_static(seed.as_bytes()))
        .await
        .with_context(|| format!("failed to send keyshare seed for operator {operator_index}"))?;
    let rows = sink
        .finish()
        .await
        .with_context(|| format!("failed to load keyshare seed for operator {operator_index}"))?;

    // The captured timestamps are as old as the seed file. The table holds
    // nothing but the seed at this point, so date them to this cluster.
    client
        .execute(
            "UPDATE signing_keyshares SET create_time = now(), update_time = now()",
            &[],
        )
        .await?;

    Ok(rows)
}

/// Count AVAILABLE keyshares per `coordinator_index` in one operator database.
pub async fn available_per_coordinator(connection_string: &str) -> Result<HashMap<i64, usize>> {
    let client = connect(connection_string).await?;
    let rows = client
        .query(
            "SELECT coordinator_index::bigint, COUNT(*)::bigint FROM signing_keyshares
             WHERE status = 'AVAILABLE' GROUP BY coordinator_index",
            &[],
        )
        .await?;

    Ok(rows
        .iter()
        .map(|row| {
            let count: i64 = row.get(1);
            (row.get(0), usize::try_from(count).unwrap_or(0))
        })
        .collect())
}

/// Write one operator's AVAILABLE keyshares to its seed file, replacing the
/// committed copy. Returns the number of keyshares captured.
pub async fn capture_seed(connection_string: &str, operator_index: usize) -> Result<u64> {
    let counts = available_per_coordinator(connection_string).await?;
    for coordinator in 0..NUM_OPERATORS {
        let available = counts.get(&(coordinator as i64)).copied().unwrap_or(0);
        if available <= MIN_AVAILABLE_KEYS {
            bail!(
                "operator {operator_index} holds {available} keyshares for coordinator \
                 {coordinator}, need more than {MIN_AVAILABLE_KEYS}: capturing this would seed a \
                 pool that immediately triggers DKG"
            );
        }
    }

    let client = connect(connection_string).await?;
    let stream = client
        .copy_out(&format!(
            "COPY (SELECT {SEED_COLUMNS} FROM signing_keyshares WHERE status = 'AVAILABLE'
             ORDER BY coordinator_index, id) TO STDOUT"
        ))
        .await?;
    futures::pin_mut!(stream);

    let mut seed = Vec::new();
    let mut rows = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        rows += chunk.iter().filter(|b| **b == b'\n').count() as u64;
        seed.extend_from_slice(&chunk);
    }

    let path = seed_path(operator_index);
    std::fs::write(&path, &seed).with_context(|| format!("failed to write {}", path.display()))?;

    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three seeds must come from one capture: an operator only signs with
    /// a keyshare its peers also hold, so a set that disagrees across operators
    /// would break every flow that reserves keyshares.
    #[test]
    fn seeds_hold_the_same_keyshares() {
        let ids: Vec<Vec<&str>> = SEEDS
            .iter()
            .map(|seed| {
                seed.lines()
                    .map(|line| line.split('\t').next().unwrap_or_default())
                    .collect()
            })
            .collect();

        assert!(!ids[0].is_empty(), "seed is empty");
        for (index, operator_ids) in ids.iter().enumerate().skip(1) {
            assert_eq!(
                ids[0], *operator_ids,
                "operator {index}'s seed holds different keyshares than operator 0's"
            );
        }
    }

    /// Every coordinator's pool has to clear `dkg.min_available_keys`, or the
    /// operators run DKG anyway and the seed buys nothing.
    #[test]
    fn seeds_keep_every_coordinator_pool_above_the_dkg_threshold() {
        for (index, seed) in SEEDS.iter().enumerate() {
            let mut per_coordinator = HashMap::new();
            for line in seed.lines() {
                let columns: Vec<&str> = line.split('\t').collect();
                assert_eq!(
                    columns.len(),
                    SEED_COLUMNS.split(',').count(),
                    "operator {index}'s seed has a row with the wrong column count"
                );
                assert_eq!(columns[3], "AVAILABLE", "seeded keyshare is not AVAILABLE");
                *per_coordinator.entry(columns[9]).or_insert(0usize) += 1;
            }

            for coordinator in 0..NUM_OPERATORS {
                let available = per_coordinator
                    .get(coordinator.to_string().as_str())
                    .copied()
                    .unwrap_or(0);
                assert!(
                    available > MIN_AVAILABLE_KEYS,
                    "operator {index}'s seed holds {available} keyshares for coordinator \
                     {coordinator}, need more than {MIN_AVAILABLE_KEYS}"
                );
            }
        }
    }
}
