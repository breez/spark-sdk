use diesel::sqlite::Sqlite;
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};
use tracing::error;
pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations/sqlite");

pub fn has_migrations(
    connection: &mut impl MigrationHarness<Sqlite>,
) -> Result<bool, anyhow::Error> {
    connection.has_pending_migration(MIGRATIONS).map_err(|e| {
        error!("failed to check for migrations: {}", e);
        anyhow::anyhow!("failed to check for migrations: {}", e)
    })
}

pub fn run_migrations(connection: &mut impl MigrationHarness<Sqlite>) -> Result<(), anyhow::Error> {
    connection.run_pending_migrations(MIGRATIONS).map_err(|e| {
        error!("failed to run migrations: {}", e);
        anyhow::anyhow!("failed to run migrations: {}", e)
    })?;

    Ok(())
}
