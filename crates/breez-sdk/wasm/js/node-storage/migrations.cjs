/**
 * Database Migration Manager for Breez SDK Node.js Storage
 */

// We'll receive StorageError as a parameter to avoid circular dependencies

class MigrationManager {
  constructor(db, StorageError, logger = null) {
    this.db = db;
    this.StorageError = StorageError;
    this.logger = logger;
    this.migrations = this._getMigrations();
  }

  /**
   * Run all pending migrations
   */
  migrate() {
    const currentVersion = this._getCurrentVersion();
    const targetVersion = this.migrations.length;

    if (currentVersion >= targetVersion) {
      this._log("info", `Database is up to date (version ${currentVersion})`);
      return;
    }

    this._log(
      "info",
      `Migrating database from version ${currentVersion} to ${targetVersion}`
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
        }
        this._setVersion(targetVersion);
      });

      transaction();
      this._log("info", `Database migration completed successfully`);
    } catch (error) {
      throw new this.StorageError(
        `Migration failed at version ${currentVersion}: ${error.message}`,
        error
      );
    }
  }

  /**
   * Get current database version
   */
  _getCurrentVersion() {
    try {
      const row = this.db.prepare("PRAGMA user_version").get();
      return row.user_version || 0;
    } catch (error) {
      this._log(
        "warn",
        `Failed to get database version, assuming 0: ${error.message}`
      );
      return 0;
    }
  }

  /**
   * Set database version
   */
  _setVersion(version) {
    this.db.pragma(`user_version = ${version}`);
  }

  _log(level, message) {
    if (this.logger && typeof this.logger.log === "function") {
      this.logger.log({
        line: message,
        level: level,
      });
    } else if (level === "error") {
      // Fallback to console.error for errors only
      console.error(`[MigrationManager] ${message}`);
    }
    // For info/debug/warn levels, only log if logger is provided
  }

  /**
   * Define all database migrations
   *
   * Each migration is an object with:
   * - name: Description of the migration
   * - sql: SQL statement(s) to execute (string or array of strings)
   */
  _getMigrations() {
    return [
      {
        name: "Create initial tables",
        sql: [
          `CREATE TABLE IF NOT EXISTS payments (
                        id TEXT PRIMARY KEY,
                        payment_type TEXT NOT NULL,
                        status TEXT NOT NULL,
                        amount INTEGER NOT NULL,
                        fees INTEGER NOT NULL,
                        timestamp INTEGER NOT NULL,
                        details TEXT,
                        method TEXT
                    )`,
          `CREATE TABLE IF NOT EXISTS settings (
                        key TEXT PRIMARY KEY,
                        value TEXT NOT NULL
                    )`,
          `CREATE INDEX IF NOT EXISTS idx_payments_timestamp ON payments(timestamp DESC)`,
        ],
      },
      {
        name: "Create unclaimed deposits table",
        sql: [
          `CREATE TABLE IF NOT EXISTS unclaimed_deposits (
                        txid TEXT NOT NULL,
                        vout INTEGER NOT NULL,
                        amount_sats INTEGER,
                        claim_error TEXT,
                        refund_tx TEXT,
                        refund_tx_id TEXT,
                        PRIMARY KEY (txid, vout)
                    )`,
          `CREATE INDEX IF NOT EXISTS idx_unclaimed_deposits_txid ON unclaimed_deposits(txid)`,
        ],
      },
      {
        name: "Create payment metadata table",
        sql: [
          `CREATE TABLE IF NOT EXISTS payment_metadata (
                        payment_id TEXT PRIMARY KEY,
                        lnurl_pay_info TEXT,
                        FOREIGN KEY (payment_id) REFERENCES payments(id) ON DELETE CASCADE
                    )`,
          `CREATE INDEX IF NOT EXISTS idx_payment_metadata_payment_id ON payment_metadata(payment_id)`,
        ],
      },
      {
        name: "Change payments amount and fees from INTEGER to TEXT",
        sql: [
          `
          CREATE TABLE IF NOT EXISTS deposit_refunds (
                        deposit_tx_id TEXT NOT NULL,
                        deposit_vout INTEGER NOT NULL,
                        refund_tx TEXT NOT NULL,
                        refund_tx_id TEXT NOT NULL,
                        PRIMARY KEY (deposit_tx_id, deposit_vout)
                    )`,
          `CREATE TABLE payments_new (
                        id TEXT PRIMARY KEY,
                        payment_type TEXT NOT NULL,
                        status TEXT NOT NULL,
                        amount TEXT NOT NULL,
                        fees TEXT NOT NULL,
                        timestamp INTEGER NOT NULL,
                        details TEXT,
                        method TEXT
                    )`,
          `INSERT INTO payments_new (id, payment_type, status, amount, fees, timestamp, details, method)
           SELECT id, payment_type, status, CAST(amount AS TEXT), CAST(fees AS TEXT), timestamp, details, method
           FROM payments`,
          `DROP TABLE payments`,
          `ALTER TABLE payments_new RENAME TO payments`,
        ],
      },
    ];
  }
}

module.exports = { MigrationManager };
