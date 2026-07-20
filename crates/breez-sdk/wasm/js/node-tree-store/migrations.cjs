/**
 * Database Migration Manager for the Breez SDK Node.js SQLite Tree Store.
 *
 * The store shares the wallet's main SQLite database file, so its tables carry
 * the `brz_` prefix to stay clear of the main storage's. Schema mirrors the Rust
 * `spark-sqlite` tree store (crates/spark-sqlite/src/lib.rs).
 */

// TreeStoreError arrives as a parameter to avoid a circular require.
class TreeStoreMigrationManager {
  constructor(db, TreeStoreError, logger = null) {
    this.db = db;
    this.TreeStoreError = TreeStoreError;
    this.logger = logger;
    this.migrations = this._getMigrations();
  }

  /**
   * Run all pending migrations, or up to a specific version.
   * @param {number|null} targetVersion - Target version (default: latest)
   */
  migrate(targetVersion = null) {
    this._ensureMigrationsTable();
    const currentVersion = this._getCurrentVersion();
    targetVersion = targetVersion ?? this.migrations.length;

    if (currentVersion >= targetVersion) {
      this._log("info", `Tree store is up to date (version ${currentVersion})`);
      return;
    }

    this._log(
      "info",
      `Migrating tree store from version ${currentVersion} to ${targetVersion}`
    );

    try {
      const transaction = this.db.transaction(() => {
        for (let i = currentVersion; i < targetVersion; i++) {
          const migration = this.migrations[i];
          this._log("debug", `Running migration ${i + 1}: ${migration.name}`);

          if (Array.isArray(migration.sql)) {
            migration.sql.forEach((sql) => this.db.exec(sql));
          } else {
            this.db.exec(migration.sql);
          }
          this._recordVersion(i + 1);
        }
      });

      transaction();
      this._log("info", `Tree store migration completed successfully`);
    } catch (error) {
      throw new this.TreeStoreError(
        `Migration failed at version ${currentVersion}: ${error.message}`,
        error
      );
    }
  }

  /**
   * Create the schema-version ledger if it does not exist. Tracked in a table,
   * not `PRAGMA user_version`, so the store can share the main storage's file.
   */
  _ensureMigrationsTable() {
    this.db.exec(
      "CREATE TABLE IF NOT EXISTS brz_tree_schema_migrations (version INTEGER PRIMARY KEY)"
    );
  }

  /**
   * Get current schema version (the highest recorded migration).
   */
  _getCurrentVersion() {
    try {
      const row = this.db
        .prepare(
          "SELECT COALESCE(MAX(version), 0) AS version FROM brz_tree_schema_migrations"
        )
        .get();
      return row.version || 0;
    } catch (error) {
      this._log(
        "warn",
        `Failed to get tree store version, assuming 0: ${error.message}`
      );
      return 0;
    }
  }

  /**
   * Record a migration as applied.
   */
  _recordVersion(version) {
    this.db
      .prepare("INSERT INTO brz_tree_schema_migrations (version) VALUES (?)")
      .run(version);
  }

  _log(level, message) {
    if (this.logger && typeof this.logger.log === "function") {
      this.logger.log({
        line: message,
        level: level,
      });
    } else if (level === "error") {
      console.error(`[TreeStoreMigrationManager] ${message}`);
    }
  }

  /**
   * Define all database migrations.
   *
   * Two-table model: `brz_tree_leaves` is the spendable pool (with reservation,
   * missing-from-operators, and timestamp metadata); `brz_tree_ancestors` holds
   * the intermediate exit-chain nodes a leaf walks through, carrying no pool
   * metadata. `brz_tree_reservations`, `brz_tree_spent`, and `brz_tree_swap_status`
   * support the reservation, spent-marker, and swap-guard logic.
   */
  _getMigrations() {
    return [
      {
        name: "Create tree store tables",
        sql: [
          `CREATE TABLE IF NOT EXISTS brz_tree_reservations (
                        id                    TEXT PRIMARY KEY,
                        purpose               TEXT NOT NULL,
                        pending_change_amount INTEGER NOT NULL DEFAULT 0,
                        created_at            INTEGER NOT NULL
                    )`,
          `CREATE TABLE IF NOT EXISTS brz_tree_leaves (
                        id                        TEXT PRIMARY KEY,
                        parent_node_id            TEXT,
                        status                    TEXT NOT NULL,
                        value                     INTEGER NOT NULL DEFAULT 0,
                        verifying_public_key      TEXT NOT NULL DEFAULT '',
                        signing_public_key        TEXT NOT NULL DEFAULT '',
                        data                      TEXT NOT NULL,
                        is_missing_from_operators INTEGER NOT NULL DEFAULT 0,
                        reservation_id            TEXT,
                        added_at                  INTEGER
                    )`,
          `CREATE INDEX IF NOT EXISTS brz_idx_tree_leaves_parent ON brz_tree_leaves (parent_node_id)`,
          `CREATE INDEX IF NOT EXISTS brz_idx_tree_leaves_reservation ON brz_tree_leaves (reservation_id)`,
          `CREATE TABLE IF NOT EXISTS brz_tree_ancestors (
                        id                   TEXT PRIMARY KEY,
                        parent_node_id       TEXT,
                        status               TEXT NOT NULL,
                        value                INTEGER NOT NULL DEFAULT 0,
                        verifying_public_key TEXT NOT NULL DEFAULT '',
                        data                 TEXT NOT NULL
                    )`,
          `CREATE INDEX IF NOT EXISTS brz_idx_tree_ancestors_parent ON brz_tree_ancestors (parent_node_id)`,
          `CREATE TABLE IF NOT EXISTS brz_tree_spent (
                        id       TEXT PRIMARY KEY,
                        spent_at INTEGER NOT NULL
                    )`,
          `CREATE TABLE IF NOT EXISTS brz_tree_swap_status (
                        id                INTEGER PRIMARY KEY CHECK (id = 1),
                        last_completed_at INTEGER
                    )`,
          `INSERT OR IGNORE INTO brz_tree_swap_status (id, last_completed_at) VALUES (1, NULL)`,
        ],
      },
    ];
  }
}

module.exports = { TreeStoreMigrationManager };
