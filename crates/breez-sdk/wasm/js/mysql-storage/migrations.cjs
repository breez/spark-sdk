/**
 * Database Migration Manager for Breez SDK MySQL Storage.
 *
 * Mirrors `postgres-storage/migrations.cjs` with SQL adapted for MySQL 8.0+:
 * - JSONB → JSON
 * - TIMESTAMPTZ NOT NULL DEFAULT NOW() → DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6)
 * - TEXT PRIMARY KEY → VARCHAR(255) PRIMARY KEY
 * - BOOLEAN → TINYINT(1)
 * - ON CONFLICT DO NOTHING → INSERT IGNORE
 * - pg_advisory_xact_lock → GET_LOCK/RELEASE_LOCK
 * - reserved word `key` quoted with backticks
 *
 * Uses a schema_migrations table + GET_LOCK to safely run migrations from
 * concurrent processes. Unlike pg's transaction-scoped advisory locks, MySQL
 * named locks are session-scoped, so we explicitly RELEASE_LOCK after the
 * commit (or on error in finally).
 */

const { StorageError } = require("./errors.cjs");

/** Named lock used to serialize concurrent migration runs. */
const MIGRATION_LOCK_NAME = "breez_mysql_migration_lock";
/** Seconds to wait when acquiring the migration lock. */
const MIGRATION_LOCK_TIMEOUT = 60;

class MysqlMigrationManager {
  constructor(logger = null) {
    this.logger = logger;
  }

  /**
   * Run all pending migrations. Holds a session-scoped GET_LOCK for the full
   * sequence so concurrent processes serialize.
   * @param {import('mysql2/promise').Pool} pool
   */
  async migrate(pool) {
    const conn = await pool.getConnection();
    try {
      const [lockRows] = await conn.query(
        "SELECT GET_LOCK(?, ?) AS acquired",
        [MIGRATION_LOCK_NAME, MIGRATION_LOCK_TIMEOUT]
      );
      if (!lockRows || lockRows[0].acquired !== 1) {
        throw new StorageError(
          `Failed to acquire migration lock '${MIGRATION_LOCK_NAME}' within ${MIGRATION_LOCK_TIMEOUT}s`
        );
      }

      try {
        await conn.query("START TRANSACTION");

        await conn.query(`
          CREATE TABLE IF NOT EXISTS schema_migrations (
            version INT PRIMARY KEY,
            applied_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6)
          )
        `);

        const [versionRows] = await conn.query(
          "SELECT COALESCE(MAX(version), 0) AS version FROM schema_migrations"
        );
        const currentVersion = versionRows[0].version;

        const migrations = this._getMigrations();

        if (currentVersion >= migrations.length) {
          this._log("info", `Database is up to date (version ${currentVersion})`);
          await conn.query("COMMIT");
          return;
        }

        this._log(
          "info",
          `Migrating database from version ${currentVersion} to ${migrations.length}`
        );

        for (let i = currentVersion; i < migrations.length; i++) {
          const migration = migrations[i];
          const version = i + 1;
          this._log("debug", `Running migration ${version}: ${migration.name}`);

          for (const sql of migration.sql) {
            await conn.query(sql);
          }

          await conn.query(
            "INSERT INTO schema_migrations (version) VALUES (?)",
            [version]
          );
        }

        await conn.query("COMMIT");
        this._log("info", "Database migration completed successfully");
      } catch (error) {
        await conn.query("ROLLBACK").catch(() => {});
        throw new StorageError(
          `Migration failed: ${error.message}`,
          error
        );
      } finally {
        // Release the named lock regardless of outcome.
        await conn
          .query("SELECT RELEASE_LOCK(?)", [MIGRATION_LOCK_NAME])
          .catch(() => {});
      }
    } finally {
      conn.release();
    }
  }

  _log(level, message) {
    if (this.logger && typeof this.logger.log === "function") {
      this.logger.log({ line: message, level });
    } else if (level === "error") {
      // eslint-disable-next-line no-console
      console.error(`[MysqlMigrationManager] ${message}`);
    }
  }

  _getMigrations() {
    return [
      {
        name: "Create all tables at final schema",
        sql: [
          // -- Core tables --
          `CREATE TABLE IF NOT EXISTS payments (
            id VARCHAR(255) NOT NULL PRIMARY KEY,
            payment_type VARCHAR(64) NOT NULL,
            status VARCHAR(64) NOT NULL,
            amount VARCHAR(64) NOT NULL,
            fees VARCHAR(64) NOT NULL,
            timestamp BIGINT NOT NULL,
            method VARCHAR(64) NULL,
            withdraw_tx_id VARCHAR(255) NULL,
            deposit_tx_id VARCHAR(255) NULL,
            spark TINYINT(1) NULL
          )`,

          `CREATE TABLE IF NOT EXISTS settings (
            \`key\` VARCHAR(255) NOT NULL PRIMARY KEY,
            value LONGTEXT NOT NULL
          )`,

          `CREATE TABLE IF NOT EXISTS unclaimed_deposits (
            txid VARCHAR(255) NOT NULL,
            vout INT NOT NULL,
            amount_sats BIGINT NULL,
            claim_error JSON NULL,
            refund_tx LONGTEXT NULL,
            refund_tx_id VARCHAR(255) NULL,
            PRIMARY KEY (txid, vout)
          )`,

          `CREATE TABLE IF NOT EXISTS payment_metadata (
            payment_id VARCHAR(255) NOT NULL PRIMARY KEY,
            parent_payment_id VARCHAR(255) NULL,
            lnurl_pay_info JSON NULL,
            lnurl_withdraw_info JSON NULL,
            lnurl_description LONGTEXT NULL,
            conversion_info JSON NULL
          )`,

          `CREATE TABLE IF NOT EXISTS payment_details_lightning (
            payment_id VARCHAR(255) NOT NULL PRIMARY KEY,
            invoice LONGTEXT NOT NULL,
            payment_hash VARCHAR(255) NOT NULL,
            destination_pubkey VARCHAR(255) NOT NULL,
            description LONGTEXT NULL,
            preimage VARCHAR(255) NULL,
            htlc_status VARCHAR(64) NOT NULL,
            htlc_expiry_time BIGINT NOT NULL
          )`,

          `CREATE TABLE IF NOT EXISTS payment_details_token (
            payment_id VARCHAR(255) NOT NULL PRIMARY KEY,
            metadata JSON NOT NULL,
            tx_hash VARCHAR(255) NOT NULL,
            tx_type VARCHAR(64) NOT NULL,
            invoice_details JSON NULL
          )`,

          `CREATE TABLE IF NOT EXISTS payment_details_spark (
            payment_id VARCHAR(255) NOT NULL PRIMARY KEY,
            invoice_details JSON NULL,
            htlc_details JSON NULL
          )`,

          `CREATE TABLE IF NOT EXISTS lnurl_receive_metadata (
            payment_hash VARCHAR(255) NOT NULL PRIMARY KEY,
            nostr_zap_request LONGTEXT NULL,
            nostr_zap_receipt LONGTEXT NULL,
            sender_comment LONGTEXT NULL
          )`,

          // -- Sync tables --
          `CREATE TABLE IF NOT EXISTS sync_revision (
            id INT NOT NULL PRIMARY KEY DEFAULT 1,
            revision BIGINT NOT NULL DEFAULT 0,
            CHECK (id = 1)
          )`,
          `INSERT IGNORE INTO sync_revision (id, revision) VALUES (1, 0)`,

          `CREATE TABLE IF NOT EXISTS sync_outgoing (
            record_type VARCHAR(255) NOT NULL,
            data_id VARCHAR(255) NOT NULL,
            schema_version VARCHAR(64) NOT NULL,
            commit_time BIGINT NOT NULL,
            updated_fields_json JSON NOT NULL,
            revision BIGINT NOT NULL
          )`,

          `CREATE TABLE IF NOT EXISTS sync_state (
            record_type VARCHAR(255) NOT NULL,
            data_id VARCHAR(255) NOT NULL,
            schema_version VARCHAR(64) NOT NULL,
            commit_time BIGINT NOT NULL,
            data JSON NOT NULL,
            revision BIGINT NOT NULL,
            PRIMARY KEY (record_type, data_id)
          )`,

          `CREATE TABLE IF NOT EXISTS sync_incoming (
            record_type VARCHAR(255) NOT NULL,
            data_id VARCHAR(255) NOT NULL,
            schema_version VARCHAR(64) NOT NULL,
            commit_time BIGINT NOT NULL,
            data JSON NOT NULL,
            revision BIGINT NOT NULL,
            PRIMARY KEY (record_type, data_id, revision)
          )`,

          // -- Indexes --
          `CREATE INDEX idx_payments_timestamp ON payments(timestamp)`,
          `CREATE INDEX idx_payments_payment_type ON payments(payment_type)`,
          `CREATE INDEX idx_payments_status ON payments(status)`,
          `CREATE INDEX idx_payment_details_lightning_invoice
            ON payment_details_lightning(invoice(255))`,
          `CREATE INDEX idx_payment_details_lightning_payment_hash
            ON payment_details_lightning(payment_hash)`,
          `CREATE INDEX idx_payment_metadata_parent ON payment_metadata(parent_payment_id)`,
          `CREATE INDEX idx_sync_outgoing_data_id_record_type
            ON sync_outgoing(record_type, data_id)`,
          `CREATE INDEX idx_sync_incoming_revision ON sync_incoming(revision)`,
        ],
      },
      {
        name: "Create contacts table",
        sql: [
          `CREATE TABLE IF NOT EXISTS contacts (
            id VARCHAR(255) NOT NULL PRIMARY KEY,
            name VARCHAR(255) NOT NULL,
            payment_identifier VARCHAR(255) NOT NULL,
            created_at BIGINT NOT NULL,
            updated_at BIGINT NOT NULL
          )`,
        ],
      },
      {
        name: "Add is_mature to unclaimed_deposits",
        sql: [
          `ALTER TABLE unclaimed_deposits ADD COLUMN is_mature TINYINT(1) NOT NULL DEFAULT 1`,
        ],
      },
      {
        name: "Add conversion_status to payment_metadata",
        sql: [
          `ALTER TABLE payment_metadata ADD COLUMN conversion_status VARCHAR(64) NULL`,
        ],
      },
    ];
  }
}

module.exports = { MysqlMigrationManager };
