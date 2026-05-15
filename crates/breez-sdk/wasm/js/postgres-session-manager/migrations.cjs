/**
 * Database Migration Manager for Breez SDK PostgreSQL Session Manager.
 *
 * Uses a session_schema_migrations table + pg_advisory_xact_lock to safely
 * run migrations from concurrent processes. Mirrors the schema produced by
 * the Rust `PostgresSessionManager`.
 */

const { SessionManagerError } = require("./errors.cjs");

/**
 * Advisory lock ID for session-manager migrations.
 * Uses a different lock ID from the storage / tree store / token store
 * migrations to avoid contention. Derived from ASCII bytes of "SESN"
 * (0x5345534E).
 */
const MIGRATION_LOCK_ID = "1397245774"; // 0x5345534E as decimal string

class SessionManagerMigrationManager {
  constructor(logger = null) {
    this.logger = logger;
  }

  /**
   * Run all pending migrations inside a single transaction with an advisory
   * lock.
   * @param {import('pg').Pool} pool
   */
  async migrate(pool) {
    const client = await pool.connect();
    try {
      await client.query("BEGIN");
      await client.query(`SELECT pg_advisory_xact_lock(${MIGRATION_LOCK_ID})`);

      await client.query(`
        CREATE TABLE IF NOT EXISTS session_schema_migrations (
          version INTEGER PRIMARY KEY,
          applied_at TIMESTAMPTZ DEFAULT NOW()
        )
      `);

      const versionResult = await client.query(
        "SELECT COALESCE(MAX(version), 0) AS version FROM session_schema_migrations"
      );
      const currentVersion = versionResult.rows[0].version;

      const migrations = this._getMigrations();

      if (currentVersion >= migrations.length) {
        this._log(
          "info",
          `Session manager database is up to date (version ${currentVersion})`
        );
        await client.query("COMMIT");
        return;
      }

      this._log(
        "info",
        `Migrating session manager database from version ${currentVersion} to ${migrations.length}`
      );

      for (let i = currentVersion; i < migrations.length; i++) {
        const migration = migrations[i];
        const version = i + 1;
        this._log(
          "debug",
          `Running session manager migration ${version}: ${migration.name}`
        );

        for (const sql of migration.sql) {
          await client.query(sql);
        }

        await client.query(
          "INSERT INTO session_schema_migrations (version) VALUES ($1)",
          [version]
        );
      }

      await client.query("COMMIT");
      this._log(
        "info",
        "Session manager database migration completed successfully"
      );
    } catch (error) {
      await client.query("ROLLBACK").catch(() => {});
      throw new SessionManagerError(
        `Session manager migration failed: ${error.message}`,
        error
      );
    } finally {
      client.release();
    }
  }

  _log(level, message) {
    if (this.logger && typeof this.logger.log === "function") {
      this.logger.log({ line: message, level });
    } else if (level === "error") {
      console.error(`[SessionManagerMigrationManager] ${message}`);
    }
  }

  /**
   * Migrations matching the Rust PostgresSessionManager schema exactly.
   */
  _getMigrations() {
    return [
      {
        name: "Create sessions table",
        sql: [
          `CREATE TABLE IF NOT EXISTS sessions (
            user_id BYTEA NOT NULL,
            service_identity_key BYTEA NOT NULL,
            token TEXT NOT NULL,
            expiration BIGINT NOT NULL,
            PRIMARY KEY (user_id, service_identity_key)
          )`,
        ],
      },
    ];
  }
}

module.exports = { SessionManagerMigrationManager };
