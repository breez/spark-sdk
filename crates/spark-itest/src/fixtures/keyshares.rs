use std::collections::HashMap;

use anyhow::Result;
use tokio_postgres::NoTls;
use tracing::warn;

/// Must match `dkg.min_available_keys` in `docker/so.config.yaml`. An operator
/// runs DKG whenever its own pool is not strictly above this, so a restored pool
/// has to clear it to keep the DKG task idle.
pub const MIN_AVAILABLE_KEYS: usize = 100;

/// The DKG refill level while capturing the snapshot, so a restored cluster has
/// keyshares to spare for a test's restocks and wallet-side trees.
pub const CAPTURE_KEYS_PER_COORDINATOR: usize = 12_000;

async fn connect(connection_string: &str) -> Result<tokio_postgres::Client> {
    let (client, connection) = tokio_postgres::connect(connection_string, NoTls).await?;
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            warn!("Keyshare database connection error: {e}");
        }
    });
    Ok(client)
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
