/**
 * Database Migration Manager for Breez SDK PostgreSQL Tree Store
 *
 * Uses a tree_schema_migrations table + pg_advisory_xact_lock to safely run
 * migrations from concurrent processes.
 */

const { TreeStoreError } = require("./errors.cjs");

/**
 * Advisory lock ID for tree store migrations.
 * Uses a different lock ID from the storage migrations to avoid contention.
 * Derived from ASCII bytes of "TREE" (0x54524545).
 */
const MIGRATION_LOCK_ID = "1414743365"; // 0x54524545 as decimal string

class TreeStoreMigrationManager {
  constructor(logger = null) {
    this.logger = logger;
  }

  /**
   * Run all pending migrations inside a single transaction with an advisory lock.
   * @param {import('pg').Pool} pool
   */
  async migrate(pool) {
    const client = await pool.connect();
    try {
      await client.query("BEGIN");

      // Transaction-level advisory lock — automatically released on COMMIT/ROLLBACK
      await client.query(`SELECT pg_advisory_xact_lock(${MIGRATION_LOCK_ID})`);

      // Create the migrations tracking table if needed
      await client.query(`
        CREATE TABLE IF NOT EXISTS tree_schema_migrations (
          version INTEGER PRIMARY KEY,
          applied_at TIMESTAMPTZ DEFAULT NOW()
        )
      `);

      // Get current version
      const versionResult = await client.query(
        "SELECT COALESCE(MAX(version), 0) AS version FROM tree_schema_migrations"
      );
      const currentVersion = versionResult.rows[0].version;

      const migrations = this._getMigrations();

      if (currentVersion >= migrations.length) {
        this._log("info", `Tree store database is up to date (version ${currentVersion})`);
        await client.query("COMMIT");
        return;
      }

      this._log(
        "info",
        `Migrating tree store database from version ${currentVersion} to ${migrations.length}`
      );

      for (let i = currentVersion; i < migrations.length; i++) {
        const migration = migrations[i];
        const version = i + 1;
        this._log("debug", `Running tree store migration ${version}: ${migration.name}`);

        for (const sql of migration.sql) {
          await client.query(sql);
        }

        await client.query(
          "INSERT INTO tree_schema_migrations (version) VALUES ($1)",
          [version]
        );
      }

      await client.query("COMMIT");
      this._log("info", "Tree store database migration completed successfully");
    } catch (error) {
      await client.query("ROLLBACK").catch(() => {});
      throw new TreeStoreError(
        `Tree store migration failed: ${error.message}`,
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
      console.error(`[TreeStoreMigrationManager] ${message}`);
    }
  }

  /**
   * Migrations matching the Rust PostgresTreeStore schema exactly.
   */
  _getMigrations() {
    return [
      {
        name: "Create tree store tables",
        sql: [
          `CREATE TABLE IF NOT EXISTS tree_reservations (
            id TEXT PRIMARY KEY,
            purpose TEXT NOT NULL,
            pending_change_amount BIGINT NOT NULL DEFAULT 0,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
          )`,

          `CREATE TABLE IF NOT EXISTS tree_leaves (
            id TEXT PRIMARY KEY,
            status TEXT NOT NULL,
            is_missing_from_operators BOOLEAN NOT NULL DEFAULT FALSE,
            reservation_id TEXT REFERENCES tree_reservations(id) ON DELETE SET NULL,
            data JSONB NOT NULL,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            added_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
          )`,

          `CREATE TABLE IF NOT EXISTS tree_spent_leaves (
            leaf_id TEXT PRIMARY KEY,
            spent_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
          )`,

          `CREATE INDEX IF NOT EXISTS idx_tree_leaves_available ON tree_leaves(status, is_missing_from_operators)
            WHERE status = 'Available' AND is_missing_from_operators = FALSE`,

          `CREATE INDEX IF NOT EXISTS idx_tree_leaves_reservation ON tree_leaves(reservation_id)
            WHERE reservation_id IS NOT NULL`,

          `CREATE INDEX IF NOT EXISTS idx_tree_leaves_added_at ON tree_leaves(added_at)`,
        ],
      },
      {
        name: "Add swap status tracking",
        sql: [
          `CREATE TABLE IF NOT EXISTS tree_swap_status (
            id INTEGER PRIMARY KEY DEFAULT 1 CHECK (id = 1),
            last_completed_at TIMESTAMPTZ
          )`,
          `INSERT INTO tree_swap_status (id) VALUES (1) ON CONFLICT DO NOTHING`,
        ],
      },
    ];
  }
}

module.exports = { TreeStoreMigrationManager };
