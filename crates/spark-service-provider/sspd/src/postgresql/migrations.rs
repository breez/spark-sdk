use deadpool_postgres::{GenericClient, Pool};

use super::DatabaseError;

/// sspd's schema changes, oldest first. sspd records the position of each change
/// it applies, so a new change goes at the end.
const MIGRATIONS: &[&str] = &[
    include_str!("migrations/20260323020404_initial.up.sql"),
    include_str!("migrations/20260326020404_onchain_wallet_meta.up.sql"),
    include_str!("migrations/20260405020404_deposit_trees.up.sql"),
    include_str!("migrations/20260417020404_leaf_id_map.up.sql"),
    include_str!("migrations/20260418020404_swaps.up.sql"),
    include_str!("migrations/20260419020404_lightning.up.sql"),
    include_str!("migrations/20260501221006_coop_exit.up.sql"),
    include_str!("migrations/20260502224529_lightning_receive_transfer_idx.up.sql"),
    include_str!("migrations/20260504113116_static_deposit.up.sql"),
    include_str!("migrations/20260505131026_static_deposit_spend_context.up.sql"),
    include_str!("migrations/20260506221620_instant_static_deposit.up.sql"),
    include_str!("migrations/20260906221150_onchain_wallet_addresses.up.sql"),
    include_str!("migrations/20260906221151_onchain_wallet_spends.up.sql"),
    include_str!("migrations/20260908143000_incoming_leaves.up.sql"),
    include_str!("migrations/20261002120000_static_deposit_signed_spend.up.sql"),
    include_str!("migrations/20261016090000_table_domains.up.sql"),
    include_str!("migrations/20261016092000_integer_output_indexes.up.sql"),
    include_str!("migrations/20261016094000_uuid_transfer_ids.up.sql"),
    include_str!("migrations/20261016100000_chain_watch_outputs.up.sql"),
    include_str!("migrations/20261016110000_leaf_store.up.sql"),
    include_str!("migrations/20261016120000_pool_leaf_store.up.sql"),
    include_str!("migrations/20261016130000_unilateral_exit_cpfp_children.up.sql"),
    include_str!("migrations/20261016140000_static_deposit_addresses.up.sql"),
];

/// Applies the schema changes that the database has no record of, in one
/// transaction. Two sspd processes that start together apply them in turn.
pub async fn migrate(pool: &Pool) -> Result<(), DatabaseError> {
    let mut client = pool.get().await?;
    let tx = client.transaction().await?;
    tx.execute(
        "SELECT pg_advisory_xact_lock(hashtextextended('brz_ssp_schema_migrations', 0))",
        &[],
    )
    .await?;
    refuse_sqlx_record(&tx).await?;
    tx.batch_execute(
        "CREATE TABLE IF NOT EXISTS brz_ssp_schema_migrations (
            version INTEGER PRIMARY KEY,
            applied_at TIMESTAMPTZ DEFAULT NOW()
        )",
    )
    .await?;
    let applied: i32 = tx
        .query_one(
            "SELECT COALESCE(MAX(version), 0) FROM brz_ssp_schema_migrations",
            &[],
        )
        .await?
        .try_get(0)?;
    for (version, migration) in (1..)
        .zip(MIGRATIONS)
        .filter(|(version, _)| *version > applied)
    {
        tx.batch_execute(migration)
            .await
            .map_err(|source| DatabaseError::Migration { version, source })?;
        tx.execute(
            "INSERT INTO brz_ssp_schema_migrations (version) VALUES ($1)",
            &[&version],
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// An sspd built with sqlx recorded its schema changes in `_sqlx_migrations`. This
/// sspd does not read that record and would apply every change again.
async fn refuse_sqlx_record(db: &impl GenericClient) -> Result<(), DatabaseError> {
    let migrated_by_sqlx: bool = db
        .query_one(
            "SELECT to_regclass('_sqlx_migrations') IS NOT NULL
                AND to_regclass('brz_ssp_schema_migrations') IS NULL",
            &[],
        )
        .await?
        .try_get(0)?;
    if migrated_by_sqlx {
        return Err(DatabaseError::MigratedBySqlx);
    }
    Ok(())
}
