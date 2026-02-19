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
   * Run all pending migrations, or up to a specific version.
   * @param {number|null} targetVersion - Target version to migrate to (default: latest)
   */
  migrate(targetVersion = null) {
    const currentVersion = this._getCurrentVersion();
    targetVersion = targetVersion ?? this.migrations.length;

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
        name: "Add lnurl_description column to payment_metadata",
        sql: `ALTER TABLE payment_metadata ADD COLUMN lnurl_description TEXT`,
      },
      {
        name: "Flatten payment details",
        sql: [
          `ALTER TABLE payments ADD COLUMN withdraw_tx_id TEXT`,
          `ALTER TABLE payments ADD COLUMN deposit_tx_id TEXT`,
          `ALTER TABLE payments ADD COLUMN spark INTEGER`,
          `CREATE TABLE payment_details_lightning (
              payment_id TEXT PRIMARY KEY,
              invoice TEXT NOT NULL,
              payment_hash TEXT NOT NULL,
              destination_pubkey TEXT NOT NULL,
              description TEXT,
              preimage TEXT,
              FOREIGN KEY (payment_id) REFERENCES payments(id) ON DELETE CASCADE
            )`,
          `INSERT INTO payment_details_lightning (payment_id, invoice, payment_hash, destination_pubkey, description, preimage)
            SELECT id, json_extract(details, '$.Lightning.invoice'), json_extract(details, '$.Lightning.payment_hash'), 
                json_extract(details, '$.Lightning.destination_pubkey'), json_extract(details, '$.Lightning.description'), 
                json_extract(details, '$.Lightning.preimage') 
            FROM payments WHERE json_extract(details, '$.Lightning.invoice') IS NOT NULL`,
          `UPDATE payments SET withdraw_tx_id = json_extract(details, '$.Withdraw.tx_id')
            WHERE json_extract(details, '$.Withdraw.tx_id') IS NOT NULL`,
          `UPDATE payments SET deposit_tx_id = json_extract(details, '$.Deposit.tx_id')
            WHERE json_extract(details, '$.Deposit.tx_id') IS NOT NULL`,
          `ALTER TABLE payments DROP COLUMN details`,
          `CREATE INDEX idx_payment_details_lightning_invoice ON payment_details_lightning(invoice)`,
        ],
      },
      {
        name: "Create payment_details_token table",
        sql: [
          `CREATE TABLE IF NOT EXISTS payment_details_token (
              payment_id TEXT PRIMARY KEY,
              metadata TEXT,
              tx_hash TEXT,
              FOREIGN KEY (payment_id) REFERENCES payments(id) ON DELETE CASCADE
            )`,
        ],
      },
      {
        name: "Change payments amount and fees from INTEGER to TEXT",
        sql: [
          `CREATE TABLE payments_new (
                        id TEXT PRIMARY KEY,
                        payment_type TEXT NOT NULL,
                        status TEXT NOT NULL,
                        amount TEXT NOT NULL,
                        fees TEXT NOT NULL,
                        timestamp INTEGER NOT NULL,
                        method TEXT,
                        withdraw_tx_id TEXT,
                        deposit_tx_id TEXT,
                        spark INTEGER
                    )`,
          `INSERT INTO payments_new (id, payment_type, status, amount, fees, timestamp, method, withdraw_tx_id, deposit_tx_id, spark)
           SELECT id, payment_type, status, CAST(amount AS TEXT), CAST(fees AS TEXT), timestamp, method, withdraw_tx_id, deposit_tx_id, spark
           FROM payments`,
          `DROP TABLE payments`,
          `ALTER TABLE payments_new RENAME TO payments`,
          `CREATE INDEX IF NOT EXISTS idx_payments_timestamp ON payments(timestamp DESC)`,
        ],
      },
      {
        name: "Add spark invoice details",
        sql: [
          `CREATE TABLE payment_details_spark (
              payment_id TEXT NOT NULL PRIMARY KEY,
              invoice_details TEXT NOT NULL,
              FOREIGN KEY (payment_id) REFERENCES payments(id) ON DELETE CASCADE
            )`,
          `ALTER TABLE payment_details_token ADD COLUMN invoice_details TEXT`,
        ],
      },
      {
        name: "Add lnurl_withdraw_info column to payment_metadata",
        sql: `ALTER TABLE payment_metadata ADD COLUMN lnurl_withdraw_info TEXT`,
      },
      {
        name: "Create sync tables",
        sql: [
          // sync_revision: tracks the last committed revision (from server-acknowledged
          // or server-received records). Does NOT include pending outgoing revisions.
          `CREATE TABLE sync_revision (
            revision INTEGER NOT NULL DEFAULT 0
          )`,
          `INSERT INTO sync_revision (revision) VALUES (0)`,
          `CREATE TABLE sync_outgoing (
            record_type TEXT NOT NULL,
            data_id TEXT NOT NULL,
            schema_version TEXT NOT NULL,
            commit_time INTEGER NOT NULL,
            updated_fields_json TEXT NOT NULL,
            revision INTEGER NOT NULL
          )`,
          `CREATE INDEX idx_sync_outgoing_data_id_record_type ON sync_outgoing(record_type, data_id)`,
          `CREATE TABLE sync_state (
            record_type TEXT NOT NULL,
            data_id TEXT NOT NULL,
            schema_version TEXT NOT NULL,
            commit_time INTEGER NOT NULL,
            data TEXT NOT NULL,
            revision INTEGER NOT NULL,
            PRIMARY KEY (record_type, data_id)
          )`,
          `CREATE TABLE sync_incoming (
            record_type TEXT NOT NULL,
            data_id TEXT NOT NULL,
            schema_version TEXT NOT NULL,
            commit_time INTEGER NOT NULL,
            data TEXT NOT NULL,
            revision INTEGER NOT NULL,
            PRIMARY KEY (record_type, data_id, revision)
          )`,
          `CREATE INDEX idx_sync_incoming_revision ON sync_incoming(revision)`,
        ],
      },
      {
        name: "Add htlc details to payment_details_spark",
        sql: [
          `ALTER TABLE payment_details_spark RENAME TO tmp_payment_details_spark`,
          `CREATE TABLE payment_details_spark (
            payment_id TEXT NOT NULL PRIMARY KEY,
            invoice_details TEXT,
            htlc_details TEXT,
            FOREIGN KEY (payment_id) REFERENCES payments(id) ON DELETE CASCADE
          )`,
          `INSERT INTO payment_details_spark (payment_id, invoice_details)
            SELECT payment_id, invoice_details FROM tmp_payment_details_spark`,
          `DROP TABLE tmp_payment_details_spark`,
        ],
      },
      {
        name: "Create lnurl_receive_metadata table",
        sql: `CREATE TABLE lnurl_receive_metadata (
                payment_hash TEXT NOT NULL PRIMARY KEY,
                nostr_zap_request TEXT,
                nostr_zap_receipt TEXT,
                sender_comment TEXT
            )`,
      },
      {
        // Delete all unclaimed deposits to clear old claim_error JSON format.
        // Deposits will be recovered on next sync.
        name: "Clear unclaimed deposits for claim_error format change",
        sql: `DELETE FROM unclaimed_deposits`,
      },
      {
        // Clear all sync tables due to BreezSigner signature change.
        // This forces users to sync from scratch to the sync server.
        // Also delete the sync_initial_complete flag to force re-populating
        // all payment metadata for outgoing sync using the new key.
        name: "Clear sync tables for BreezSigner backward compatibility",
        sql: [
          `DELETE FROM sync_outgoing`,
          `DELETE FROM sync_incoming`,
          `DELETE FROM sync_state`,
          `UPDATE sync_revision SET revision = 0`,
          `DELETE FROM settings WHERE key = 'sync_initial_complete'`
        ]
      },
      {
        name: "Add token conversion info to payment_metadata",
        sql: `ALTER TABLE payment_metadata ADD COLUMN token_conversion_info TEXT`,
      },
      {
        name: "Add parent payment id to payment_metadata",
        sql: `ALTER TABLE payment_metadata ADD COLUMN parent_payment_id TEXT`
      },
      {
        name: "Add conversion info to payment_metadata",
        sql: [
          `ALTER TABLE payment_metadata DROP COLUMN token_conversion_info`,
          `ALTER TABLE payment_metadata ADD COLUMN conversion_info TEXT`]
      },
      {
        name: "Add tx_type column to payment_details_token",
        sql: [
          // Add tx_type column with a default value of 'transfer'.
          // Delete sync cache to trigger token re-sync which will update all records with correct tx_type.
          // Note: This intentionally couples to the CachedSyncInfo schema at migration time.
          `ALTER TABLE payment_details_token ADD COLUMN tx_type TEXT NOT NULL DEFAULT 'transfer'`,
          `UPDATE settings
           SET value = json_set(value, '$.last_synced_final_token_payment_id', NULL)
           WHERE key = 'sync_offset' AND json_valid(value) AND json_type(value, '$.last_synced_final_token_payment_id') IS NOT NULL`,
        ],
      },
      {
        name: "Clear sync tables to force re-sync",
        sql: [
          `DELETE FROM sync_outgoing`,
          `DELETE FROM sync_incoming`,
          `DELETE FROM sync_state`,
          `UPDATE sync_revision SET revision = 0`,
          `DELETE FROM settings WHERE key = 'sync_initial_complete'`
        ]
      },
      {
        name: "Add htlc_status and htlc_expiry_time to lightning payments",
        sql: [
          `ALTER TABLE payment_details_lightning ADD COLUMN htlc_status TEXT NOT NULL DEFAULT 'waitingForPreimage'`,
          `ALTER TABLE payment_details_lightning ADD COLUMN htlc_expiry_time INTEGER NOT NULL DEFAULT 0`,
        ]
      },
      {
        name: "Backfill htlc_status for existing Lightning payments",
        sql: [
          `UPDATE payment_details_lightning
           SET htlc_status = CASE
                   WHEN (SELECT status FROM payments WHERE id = payment_id) = 'completed' THEN 'preimageShared'
                   WHEN (SELECT status FROM payments WHERE id = payment_id) = 'pending' THEN 'waitingForPreimage'
                   ELSE 'returned'
               END`,
          `UPDATE settings
           SET value = json_set(value, '$.offset', 0)
           WHERE key = 'sync_offset' AND json_valid(value)`,
        ]
      },
    ];
  }
}

module.exports = { MigrationManager };
