/**
 * Database Migration Manager for Breez SDK PostgreSQL Storage
 *
 * Uses a schema_migrations table + pg_advisory_xact_lock to safely run
 * migrations from concurrent processes.
 */

const { StorageError } = require("./errors.cjs");

/**
 * Advisory lock ID for migrations.
 * Derived from ASCII bytes of "MIGR" (0x4D49_4752).
 */
const MIGRATION_LOCK_ID = "1296388946"; // 0x4D494752 as decimal string

class PostgresMigrationManager {
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
        CREATE TABLE IF NOT EXISTS schema_migrations (
          version INTEGER PRIMARY KEY,
          applied_at TIMESTAMPTZ DEFAULT NOW()
        )
      `);

      // Get current version
      const versionResult = await client.query(
        "SELECT COALESCE(MAX(version), 0) AS version FROM schema_migrations"
      );
      const currentVersion = versionResult.rows[0].version;

      const migrations = this._getMigrations();

      if (currentVersion >= migrations.length) {
        this._log("info", `Database is up to date (version ${currentVersion})`);
        await client.query("COMMIT");
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
          await client.query(sql);
        }

        await client.query(
          "INSERT INTO schema_migrations (version) VALUES ($1)",
          [version]
        );
      }

      await client.query("COMMIT");
      this._log("info", "Database migration completed successfully");
    } catch (error) {
      await client.query("ROLLBACK").catch(() => {});
      throw new StorageError(
        `Migration failed: ${error.message}`,
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
      console.error(`[PostgresMigrationManager] ${message}`);
    }
  }

  /**
   * Single migration creating all tables at their final schema.
   * This mirrors the Rust-native PostgresStorage schema but uses camelCase
   * enum values (as produced by the WASM bridge).
   */
  _getMigrations() {
    return [
      {
        name: "Create all tables at final schema",
        sql: [
          // -- Core tables --
          `CREATE TABLE IF NOT EXISTS payments (
            id TEXT PRIMARY KEY,
            payment_type TEXT NOT NULL,
            status TEXT NOT NULL,
            amount TEXT NOT NULL,
            fees TEXT NOT NULL,
            timestamp BIGINT NOT NULL,
            method TEXT,
            withdraw_tx_id TEXT,
            deposit_tx_id TEXT,
            spark BOOLEAN
          )`,

          `CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
          )`,

          `CREATE TABLE IF NOT EXISTS unclaimed_deposits (
            txid TEXT NOT NULL,
            vout INTEGER NOT NULL,
            amount_sats BIGINT,
            claim_error JSONB,
            refund_tx TEXT,
            refund_tx_id TEXT,
            PRIMARY KEY (txid, vout)
          )`,

          `CREATE TABLE IF NOT EXISTS payment_metadata (
            payment_id TEXT PRIMARY KEY,
            parent_payment_id TEXT,
            lnurl_pay_info JSONB,
            lnurl_withdraw_info JSONB,
            lnurl_description TEXT,
            conversion_info JSONB
          )`,

          `CREATE TABLE IF NOT EXISTS payment_details_lightning (
            payment_id TEXT PRIMARY KEY,
            invoice TEXT NOT NULL,
            payment_hash TEXT NOT NULL,
            destination_pubkey TEXT NOT NULL,
            description TEXT,
            preimage TEXT,
            htlc_status TEXT NOT NULL,
            htlc_expiry_time BIGINT NOT NULL
          )`,

          `CREATE TABLE IF NOT EXISTS payment_details_token (
            payment_id TEXT PRIMARY KEY,
            metadata JSONB NOT NULL,
            tx_hash TEXT NOT NULL,
            tx_type TEXT NOT NULL,
            invoice_details JSONB
          )`,

          `CREATE TABLE IF NOT EXISTS payment_details_spark (
            payment_id TEXT PRIMARY KEY,
            invoice_details JSONB,
            htlc_details JSONB
          )`,

          `CREATE TABLE IF NOT EXISTS lnurl_receive_metadata (
            payment_hash TEXT PRIMARY KEY,
            nostr_zap_request TEXT,
            nostr_zap_receipt TEXT,
            sender_comment TEXT,
            preimage TEXT
          )`,

          // -- Sync tables --
          `CREATE TABLE IF NOT EXISTS sync_revision (
            id INTEGER PRIMARY KEY DEFAULT 1,
            revision BIGINT NOT NULL DEFAULT 0,
            CHECK (id = 1)
          )`,
          `INSERT INTO sync_revision (id, revision) VALUES (1, 0) ON CONFLICT (id) DO NOTHING`,

          `CREATE TABLE IF NOT EXISTS sync_outgoing (
            record_type TEXT NOT NULL,
            data_id TEXT NOT NULL,
            schema_version TEXT NOT NULL,
            commit_time BIGINT NOT NULL,
            updated_fields_json JSONB NOT NULL,
            revision BIGINT NOT NULL
          )`,

          `CREATE TABLE IF NOT EXISTS sync_state (
            record_type TEXT NOT NULL,
            data_id TEXT NOT NULL,
            schema_version TEXT NOT NULL,
            commit_time BIGINT NOT NULL,
            data JSONB NOT NULL,
            revision BIGINT NOT NULL,
            PRIMARY KEY (record_type, data_id)
          )`,

          `CREATE TABLE IF NOT EXISTS sync_incoming (
            record_type TEXT NOT NULL,
            data_id TEXT NOT NULL,
            schema_version TEXT NOT NULL,
            commit_time BIGINT NOT NULL,
            data JSONB NOT NULL,
            revision BIGINT NOT NULL,
            PRIMARY KEY (record_type, data_id, revision)
          )`,

          // -- Indexes --
          `CREATE INDEX IF NOT EXISTS idx_payments_timestamp ON payments(timestamp)`,
          `CREATE INDEX IF NOT EXISTS idx_payments_payment_type ON payments(payment_type)`,
          `CREATE INDEX IF NOT EXISTS idx_payments_status ON payments(status)`,
          `CREATE INDEX IF NOT EXISTS idx_payment_details_lightning_invoice ON payment_details_lightning(invoice)`,
          `CREATE INDEX IF NOT EXISTS idx_payment_details_lightning_payment_hash ON payment_details_lightning(payment_hash)`,
          `CREATE INDEX IF NOT EXISTS idx_payment_metadata_parent ON payment_metadata(parent_payment_id)`,
          `CREATE INDEX IF NOT EXISTS idx_sync_outgoing_data_id_record_type ON sync_outgoing(record_type, data_id)`,
          `CREATE INDEX IF NOT EXISTS idx_sync_incoming_revision ON sync_incoming(revision)`,
        ],
      },
      {
        name: "Create contacts table",
        sql: [
          `CREATE TABLE IF NOT EXISTS contacts (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            payment_identifier TEXT NOT NULL,
            created_at BIGINT NOT NULL,
            updated_at BIGINT NOT NULL
          )`,
        ],
      },
      {
        name: "Drop preimage column from lnurl_receive_metadata",
        sql: [
          `ALTER TABLE lnurl_receive_metadata DROP COLUMN IF EXISTS preimage`,
        ],
      },
    ];
  }
}

module.exports = { PostgresMigrationManager };
