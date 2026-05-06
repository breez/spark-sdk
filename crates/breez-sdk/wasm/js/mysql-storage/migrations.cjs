/**
 * Database Migration Manager for Breez SDK MySQL Storage.
 *
 * Mirrors `postgres-storage/migrations.cjs` with SQL adapted for MySQL 8.0+:
 * - JSONB → JSON
 * - TIMESTAMPTZ NOT NULL DEFAULT NOW() → DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6)
 * - TEXT PRIMARY KEY → VARCHAR(255) PRIMARY KEY
 * - BOOLEAN → TINYINT(1)
 * - ON CONFLICT DO NOTHING → INSERT … ON DUPLICATE KEY UPDATE <pk> = <pk>
 *   (avoid INSERT IGNORE: it silently swallows non-PK errors too)
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

/**
 * Runs a single migration step. Plain strings are run as-is; tagged objects
 * (`{ op: 'dropPrimaryKey', table }`) are guarded against partial-apply replay
 * by checking `information_schema` first. MySQL DDL implicitly commits, so
 * if the migration crashes between two DDL statements the version row never
 * gets recorded — and on retry, an unguarded DROP PRIMARY KEY would fail
 * (`ER_CANT_DROP_FIELD_OR_KEY`) because the PK is already gone.
 */
async function runMigrationStep(conn, step) {
  if (typeof step === "string") {
    await conn.query(step);
    return;
  }
  if (step.op === "dropPrimaryKey") {
    const [rows] = await conn.query(
      `SELECT COUNT(*) AS c FROM information_schema.table_constraints
       WHERE table_schema = DATABASE()
         AND table_name = ?
         AND constraint_type = 'PRIMARY KEY'`,
      [step.table]
    );
    if (rows[0].c > 0) {
      await conn.query(`ALTER TABLE \`${step.table}\` DROP PRIMARY KEY`);
    }
    return;
  }
  throw new Error(`Unknown migration step op: ${JSON.stringify(step)}`);
}

class MysqlMigrationManager {
  constructor(logger = null) {
    this.logger = logger;
  }

  /**
   * Run all pending migrations. Holds a session-scoped GET_LOCK for the full
   * sequence so concurrent processes serialize.
   *
   * @param {import('mysql2/promise').Pool} pool
   * @param {Buffer|Uint8Array} identity - 33-byte secp256k1 compressed pubkey
   *   identifying the tenant. Used to backfill `user_id` columns in the
   *   multi-tenant migration so that pre-existing single-tenant data remains
   *   readable.
   */
  async migrate(pool, identity) {
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

        const migrations = this._getMigrations(identity);

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

          for (const step of migration.sql) {
            await runMigrationStep(conn, step);
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

  /**
   * @param {Buffer|Uint8Array} identity - 33-byte tenant identity. Inlined as
   *   an `UNHEX('...')` literal in the multi-tenant scoping migration. Safe
   *   because the bytes come from a typed secp256k1 pubkey (character set
   *   `[0-9a-f]{66}` after hex encoding) — not user-controlled input.
   */
  _getMigrations(identity) {
    const idHex = Buffer.from(identity).toString("hex");
    const idLit = `UNHEX('${idHex}')`;

    // Per-table backfill: ADD COLUMN nullable -> UPDATE -> SET NOT NULL +
    // drop/recreate PK. Returns an array of statements. Tables with backticked
    // names (e.g. `settings` uses `key`) need the caller to backtick `pkCols`.
    const scopeTable = (table, pkCols) => [
      `ALTER TABLE \`${table}\` ADD COLUMN user_id VARBINARY(33) NULL`,
      `UPDATE \`${table}\` SET user_id = ${idLit} WHERE user_id IS NULL`,
      `ALTER TABLE \`${table}\` MODIFY COLUMN user_id VARBINARY(33) NOT NULL`,
      `ALTER TABLE \`${table}\` DROP PRIMARY KEY, ADD PRIMARY KEY (user_id, ${pkCols})`,
    ];

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
          `INSERT INTO sync_revision (id, revision) VALUES (1, 0)
            ON DUPLICATE KEY UPDATE id = id`,

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
      {
        name: "Multi-tenant scoping: add user_id and rewrite primary keys",
        sql: [
          // Per-user tables.
          ...scopeTable("payments", "id"),
          `DROP INDEX idx_payments_timestamp ON payments`,
          `DROP INDEX idx_payments_payment_type ON payments`,
          `DROP INDEX idx_payments_status ON payments`,
          `CREATE INDEX idx_payments_user_timestamp ON payments(user_id, timestamp)`,
          `CREATE INDEX idx_payments_user_payment_type ON payments(user_id, payment_type)`,
          `CREATE INDEX idx_payments_user_status ON payments(user_id, status)`,

          ...scopeTable("payment_metadata", "payment_id"),
          `DROP INDEX idx_payment_metadata_parent ON payment_metadata`,
          `CREATE INDEX idx_payment_metadata_user_parent
             ON payment_metadata(user_id, parent_payment_id)`,

          ...scopeTable("payment_details_lightning", "payment_id"),
          `DROP INDEX idx_payment_details_lightning_invoice ON payment_details_lightning`,
          `DROP INDEX idx_payment_details_lightning_payment_hash ON payment_details_lightning`,
          `CREATE INDEX idx_payment_details_lightning_user_invoice
             ON payment_details_lightning(user_id, invoice(255))`,
          `CREATE INDEX idx_payment_details_lightning_user_payment_hash
             ON payment_details_lightning(user_id, payment_hash)`,

          ...scopeTable("payment_details_token", "payment_id"),
          ...scopeTable("payment_details_spark", "payment_id"),
          ...scopeTable("lnurl_receive_metadata", "payment_hash"),
          ...scopeTable("unclaimed_deposits", "txid, vout"),
          ...scopeTable("contacts", "id"),
          ...scopeTable("settings", "`key`"),

          // sync_revision was a singleton (PK id=1, CHECK id=1). Drop the PK
          // and the id column, then re-key by user_id.
          { op: "dropPrimaryKey", table: "sync_revision" },
          `ALTER TABLE sync_revision DROP COLUMN id`,
          `ALTER TABLE sync_revision ADD COLUMN user_id VARBINARY(33) NULL`,
          `UPDATE sync_revision SET user_id = ${idLit} WHERE user_id IS NULL`,
          `ALTER TABLE sync_revision MODIFY COLUMN user_id VARBINARY(33) NOT NULL`,
          `ALTER TABLE sync_revision ADD PRIMARY KEY (user_id)`,

          // sync_outgoing has no PK, only an index — just add user_id and
          // rewrite the index.
          `ALTER TABLE sync_outgoing ADD COLUMN user_id VARBINARY(33) NULL`,
          `UPDATE sync_outgoing SET user_id = ${idLit} WHERE user_id IS NULL`,
          `ALTER TABLE sync_outgoing MODIFY COLUMN user_id VARBINARY(33) NOT NULL`,
          `DROP INDEX idx_sync_outgoing_data_id_record_type ON sync_outgoing`,
          `CREATE INDEX idx_sync_outgoing_user_record_type_data_id
             ON sync_outgoing(user_id, record_type, data_id)`,

          ...scopeTable("sync_state", "record_type, data_id"),

          ...scopeTable("sync_incoming", "record_type, data_id, revision"),
          `DROP INDEX idx_sync_incoming_revision ON sync_incoming`,
          `CREATE INDEX idx_sync_incoming_user_revision
             ON sync_incoming(user_id, revision)`,
        ],
      },
    ];
  }
}

module.exports = { MysqlMigrationManager };
