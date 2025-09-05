use diesel::sqlite::Sqlite;
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};
pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("../../migrations/sqlite");

pub fn has_migrations(
    connection: &mut impl MigrationHarness<Sqlite>,
) -> Result<bool, anyhow::Error> {
    Ok(connection.has_pending_migration(MIGRATIONS)?)
}

pub fn run_migrations(connection: &mut impl MigrationHarness<Sqlite>) -> Result<(), anyhow::Error> {
    connection.run_pending_migrations(MIGRATIONS)?;

    Ok(())
}
