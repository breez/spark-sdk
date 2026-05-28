/**
 * Database Migration Manager for Breez SDK PostgreSQL Session Store.
 *
 * Uses a brz_session_schema_migrations table + pg_advisory_xact_lock to safely
 * run migrations from concurrent processes. Mirrors the schema produced by
 * the Rust `PostgresSessionStore`.
 */

const { SessionStoreError } = require("./errors.cjs");

/**
 * Advisory lock ID for session-store migrations.
 * Uses a different lock ID from the storage / tree store / token store
 * migrations to avoid contention. Derived from ASCII bytes of "SESN"
 * (0x5345534E).
 */
const MIGRATION_LOCK_ID = "1397245774"; // 0x5345534E as decimal string

class SessionStoreMigrationManager {
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

      await this._applySchemaRenames(client);

      await client.query(`
        CREATE TABLE IF NOT EXISTS brz_session_schema_migrations (
          version INTEGER PRIMARY KEY,
          applied_at TIMESTAMPTZ DEFAULT NOW()
        )
      `);

      const versionResult = await client.query(
        "SELECT COALESCE(MAX(version), 0) AS version FROM brz_session_schema_migrations"
      );
      const currentVersion = versionResult.rows[0].version;

      const migrations = this._getMigrations();

      if (currentVersion >= migrations.length) {
        this._log(
          "info",
          `Session store database is up to date (version ${currentVersion})`
        );
        await client.query("COMMIT");
        return;
      }

      this._log(
        "info",
        `Migrating session store database from version ${currentVersion} to ${migrations.length}`
      );

      for (let i = currentVersion; i < migrations.length; i++) {
        const migration = migrations[i];
        const version = i + 1;
        this._log(
          "debug",
          `Running session store migration ${version}: ${migration.name}`
        );

        for (const sql of migration.sql) {
          await client.query(sql);
        }

        await client.query(
          "INSERT INTO brz_session_schema_migrations (version) VALUES ($1)",
          [version]
        );
      }

      await client.query("COMMIT");
      this._log(
        "info",
        "Session store database migration completed successfully"
      );
    } catch (error) {
      await client.query("ROLLBACK").catch(() => {});
      throw new SessionStoreError(
        `Session store migration failed: ${error.message}`,
        error
      );
    } finally {
      client.release();
    }
  }

  /**
   * Pre-prefix rename. Canary-gated on the legacy `session_schema_migrations`
   * table.
   * @param {import('pg').PoolClient} client
   */
  async _applySchemaRenames(client) {
    const canary = await client.query(
      `SELECT EXISTS (
         SELECT 1 FROM information_schema.tables
         WHERE table_schema = current_schema()
           AND table_name = 'session_schema_migrations'
       ) AS exists`
    );
    if (!canary.rows[0].exists) {
      return;
    }

    // Rename data tables first, then their auto-named PK constraints, then
    // the migrations tracking table (which doubles as the rename canary).
    await client.query(`ALTER TABLE IF EXISTS sessions RENAME TO brz_sessions`);
    await client.query(
      `DO $$ BEGIN
         IF EXISTS (
           SELECT 1 FROM information_schema.table_constraints
           WHERE table_schema = current_schema()
             AND table_name = 'brz_sessions'
             AND constraint_name = 'sessions_pkey'
         ) THEN
           ALTER TABLE brz_sessions RENAME CONSTRAINT sessions_pkey TO brz_sessions_pkey;
         END IF;
       END $$`
    );
    await client.query(
      `ALTER TABLE IF EXISTS session_schema_migrations RENAME TO brz_session_schema_migrations`
    );
  }

  _log(level, message) {
    if (this.logger && typeof this.logger.log === "function") {
      this.logger.log({ line: message, level });
    } else if (level === "error") {
      console.error(`[SessionStoreMigrationManager] ${message}`);
    }
  }

  /**
   * Migrations matching the Rust PostgresSessionStore schema exactly.
   */
  _getMigrations() {
    return [
      {
        name: "Create brz_sessions table",
        sql: [
          `CREATE TABLE IF NOT EXISTS brz_sessions (
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

module.exports = { SessionStoreMigrationManager };
