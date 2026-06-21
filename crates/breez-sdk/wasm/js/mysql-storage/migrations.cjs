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
 * Uses a brz_schema_migrations table + GET_LOCK to safely run migrations from
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
        await this._applySchemaRenames(conn);

        await conn.query("START TRANSACTION");

        await conn.query(`
          CREATE TABLE IF NOT EXISTS brz_schema_migrations (
            version INT PRIMARY KEY,
            applied_at DATETIME(6) NOT NULL DEFAULT (UTC_TIMESTAMP(6))
          )
        `);

        const [versionRows] = await conn.query(
          "SELECT COALESCE(MAX(version), 0) AS version FROM brz_schema_migrations"
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
            "INSERT INTO brz_schema_migrations (version, applied_at) VALUES (?, UTC_TIMESTAMP(6))",
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
   * Pre-prefix rename. Canary-gated on the legacy `schema_migrations`
   * table. MySQL PKs are always named `PRIMARY` and the core schema has
   * no FKs, so only tables and indexes need renaming.
   * @param {import('mysql2/promise').PoolConnection} conn
   */
  async _applySchemaRenames(conn) {
    if (!(await _mysqlTableExists(conn, "schema_migrations"))) {
      return;
    }

    const tableRenames = [
      ["payments", "brz_payments"],
      ["settings", "brz_settings"],
      ["unclaimed_deposits", "brz_unclaimed_deposits"],
      ["payment_metadata", "brz_payment_metadata"],
      ["payment_details_lightning", "brz_payment_details_lightning"],
      ["payment_details_token", "brz_payment_details_token"],
      ["payment_details_spark", "brz_payment_details_spark"],
      ["lnurl_receive_metadata", "brz_lnurl_receive_metadata"],
      ["sync_revision", "brz_sync_revision"],
      ["sync_outgoing", "brz_sync_outgoing"],
      ["sync_state", "brz_sync_state"],
      ["sync_incoming", "brz_sync_incoming"],
      ["contacts", "brz_contacts"],
    ];
    for (const [oldName, newName] of tableRenames) {
      if (
        (await _mysqlTableExists(conn, oldName)) &&
        !(await _mysqlTableExists(conn, newName))
      ) {
        await conn.query(`RENAME TABLE \`${oldName}\` TO \`${newName}\``);
      }
    }

    const indexRenames = [
      ["brz_payments", "idx_payments_user_timestamp", "brz_idx_payments_user_timestamp"],
      ["brz_payments", "idx_payments_user_payment_type", "brz_idx_payments_user_payment_type"],
      ["brz_payments", "idx_payments_user_status", "brz_idx_payments_user_status"],
      [
        "brz_payment_metadata",
        "idx_payment_metadata_user_parent",
        "brz_idx_payment_metadata_user_parent",
      ],
      [
        "brz_payment_details_lightning",
        "idx_payment_details_lightning_user_invoice",
        "brz_idx_payment_details_lightning_user_invoice",
      ],
      [
        "brz_payment_details_lightning",
        "idx_payment_details_lightning_user_payment_hash",
        "brz_idx_payment_details_lightning_user_payment_hash",
      ],
      [
        "brz_sync_outgoing",
        "idx_sync_outgoing_user_record_type_data_id",
        "brz_idx_sync_outgoing_user_record_type_data_id",
      ],
      [
        "brz_sync_incoming",
        "idx_sync_incoming_user_revision",
        "brz_idx_sync_incoming_user_revision",
      ],
      // Pre-multi-tenant indexes (still present on version < 16 DBs).
      ["brz_payments", "idx_payments_timestamp", "brz_idx_payments_timestamp"],
      ["brz_payments", "idx_payments_payment_type", "brz_idx_payments_payment_type"],
      ["brz_payments", "idx_payments_status", "brz_idx_payments_status"],
      [
        "brz_payment_metadata",
        "idx_payment_metadata_parent",
        "brz_idx_payment_metadata_parent",
      ],
      [
        "brz_payment_details_lightning",
        "idx_payment_details_lightning_invoice",
        "brz_idx_payment_details_lightning_invoice",
      ],
      [
        "brz_payment_details_lightning",
        "idx_payment_details_lightning_payment_hash",
        "brz_idx_payment_details_lightning_payment_hash",
      ],
      [
        "brz_sync_outgoing",
        "idx_sync_outgoing_data_id_record_type",
        "brz_idx_sync_outgoing_data_id_record_type",
      ],
      [
        "brz_sync_incoming",
        "idx_sync_incoming_revision",
        "brz_idx_sync_incoming_revision",
      ],
    ];
    for (const [table, oldName, newName] of indexRenames) {
      if (
        (await _mysqlIndexExists(conn, table, oldName)) &&
        !(await _mysqlIndexExists(conn, table, newName))
      ) {
        await conn.query(
          `ALTER TABLE \`${table}\` RENAME INDEX \`${oldName}\` TO \`${newName}\``
        );
      }
    }

    if (
      (await _mysqlTableExists(conn, "schema_migrations")) &&
      !(await _mysqlTableExists(conn, "brz_schema_migrations"))
    ) {
      await conn.query(
        "RENAME TABLE `schema_migrations` TO `brz_schema_migrations`"
      );
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
    // names (e.g. `brz_settings` uses `key`) need the caller to backtick `pkCols`.
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
          `CREATE TABLE IF NOT EXISTS brz_payments (
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

          `CREATE TABLE IF NOT EXISTS brz_settings (
            \`key\` VARCHAR(255) NOT NULL PRIMARY KEY,
            value LONGTEXT NOT NULL
          )`,

          `CREATE TABLE IF NOT EXISTS brz_unclaimed_deposits (
            txid VARCHAR(255) NOT NULL,
            vout INT NOT NULL,
            amount_sats BIGINT NULL,
            claim_error JSON NULL,
            refund_tx LONGTEXT NULL,
            refund_tx_id VARCHAR(255) NULL,
            PRIMARY KEY (txid, vout)
          )`,

          `CREATE TABLE IF NOT EXISTS brz_payment_metadata (
            payment_id VARCHAR(255) NOT NULL PRIMARY KEY,
            parent_payment_id VARCHAR(255) NULL,
            lnurl_pay_info JSON NULL,
            lnurl_withdraw_info JSON NULL,
            lnurl_description LONGTEXT NULL,
            conversion_info JSON NULL
          )`,

          `CREATE TABLE IF NOT EXISTS brz_payment_details_lightning (
            payment_id VARCHAR(255) NOT NULL PRIMARY KEY,
            invoice LONGTEXT NOT NULL,
            payment_hash VARCHAR(255) NOT NULL,
            destination_pubkey VARCHAR(255) NOT NULL,
            description LONGTEXT NULL,
            preimage VARCHAR(255) NULL,
            htlc_status VARCHAR(64) NOT NULL,
            htlc_expiry_time BIGINT NOT NULL
          )`,

          `CREATE TABLE IF NOT EXISTS brz_payment_details_token (
            payment_id VARCHAR(255) NOT NULL PRIMARY KEY,
            metadata JSON NOT NULL,
            tx_hash VARCHAR(255) NOT NULL,
            tx_type VARCHAR(64) NOT NULL,
            invoice_details JSON NULL
          )`,

          `CREATE TABLE IF NOT EXISTS brz_payment_details_spark (
            payment_id VARCHAR(255) NOT NULL PRIMARY KEY,
            invoice_details JSON NULL,
            htlc_details JSON NULL
          )`,

          `CREATE TABLE IF NOT EXISTS brz_lnurl_receive_metadata (
            payment_hash VARCHAR(255) NOT NULL PRIMARY KEY,
            nostr_zap_request LONGTEXT NULL,
            nostr_zap_receipt LONGTEXT NULL,
            sender_comment LONGTEXT NULL
          )`,

          // -- Sync tables --
          `CREATE TABLE IF NOT EXISTS brz_sync_revision (
            id INT NOT NULL PRIMARY KEY DEFAULT 1,
            revision BIGINT NOT NULL DEFAULT 0,
            CHECK (id = 1)
          )`,
          `INSERT INTO brz_sync_revision (id, revision) VALUES (1, 0)
            ON DUPLICATE KEY UPDATE id = id`,

          `CREATE TABLE IF NOT EXISTS brz_sync_outgoing (
            record_type VARCHAR(255) NOT NULL,
            data_id VARCHAR(255) NOT NULL,
            schema_version VARCHAR(64) NOT NULL,
            commit_time BIGINT NOT NULL,
            updated_fields_json JSON NOT NULL,
            revision BIGINT NOT NULL
          )`,

          `CREATE TABLE IF NOT EXISTS brz_sync_state (
            record_type VARCHAR(255) NOT NULL,
            data_id VARCHAR(255) NOT NULL,
            schema_version VARCHAR(64) NOT NULL,
            commit_time BIGINT NOT NULL,
            data JSON NOT NULL,
            revision BIGINT NOT NULL,
            PRIMARY KEY (record_type, data_id)
          )`,

          `CREATE TABLE IF NOT EXISTS brz_sync_incoming (
            record_type VARCHAR(255) NOT NULL,
            data_id VARCHAR(255) NOT NULL,
            schema_version VARCHAR(64) NOT NULL,
            commit_time BIGINT NOT NULL,
            data JSON NOT NULL,
            revision BIGINT NOT NULL,
            PRIMARY KEY (record_type, data_id, revision)
          )`,

          // -- Indexes --
          `CREATE INDEX brz_idx_payments_timestamp ON brz_payments(timestamp)`,
          `CREATE INDEX brz_idx_payments_payment_type ON brz_payments(payment_type)`,
          `CREATE INDEX brz_idx_payments_status ON brz_payments(status)`,
          `CREATE INDEX brz_idx_payment_details_lightning_invoice
            ON brz_payment_details_lightning(invoice(255))`,
          `CREATE INDEX brz_idx_payment_details_lightning_payment_hash
            ON brz_payment_details_lightning(payment_hash)`,
          `CREATE INDEX brz_idx_payment_metadata_parent ON brz_payment_metadata(parent_payment_id)`,
          `CREATE INDEX brz_idx_sync_outgoing_data_id_record_type
            ON brz_sync_outgoing(record_type, data_id)`,
          `CREATE INDEX brz_idx_sync_incoming_revision ON brz_sync_incoming(revision)`,
        ],
      },
      {
        name: "Create brz_contacts table",
        sql: [
          `CREATE TABLE IF NOT EXISTS brz_contacts (
            id VARCHAR(255) NOT NULL PRIMARY KEY,
            name VARCHAR(255) NOT NULL,
            payment_identifier VARCHAR(255) NOT NULL,
            created_at BIGINT NOT NULL,
            updated_at BIGINT NOT NULL
          )`,
        ],
      },
      {
        name: "Add is_mature to brz_unclaimed_deposits",
        sql: [
          `ALTER TABLE brz_unclaimed_deposits ADD COLUMN is_mature TINYINT(1) NOT NULL DEFAULT 1`,
        ],
      },
      {
        name: "Add conversion_status to brz_payment_metadata",
        sql: [
          `ALTER TABLE brz_payment_metadata ADD COLUMN conversion_status VARCHAR(64) NULL`,
        ],
      },
      {
        name: "Multi-tenant scoping: add user_id and rewrite primary keys",
        sql: [
          // Per-user tables.
          ...scopeTable("brz_payments", "id"),
          `DROP INDEX brz_idx_payments_timestamp ON brz_payments`,
          `DROP INDEX brz_idx_payments_payment_type ON brz_payments`,
          `DROP INDEX brz_idx_payments_status ON brz_payments`,
          `CREATE INDEX brz_idx_payments_user_timestamp ON brz_payments(user_id, timestamp)`,
          `CREATE INDEX brz_idx_payments_user_payment_type ON brz_payments(user_id, payment_type)`,
          `CREATE INDEX brz_idx_payments_user_status ON brz_payments(user_id, status)`,

          ...scopeTable("brz_payment_metadata", "payment_id"),
          `DROP INDEX brz_idx_payment_metadata_parent ON brz_payment_metadata`,
          `CREATE INDEX brz_idx_payment_metadata_user_parent
             ON brz_payment_metadata(user_id, parent_payment_id)`,

          ...scopeTable("brz_payment_details_lightning", "payment_id"),
          `DROP INDEX brz_idx_payment_details_lightning_invoice ON brz_payment_details_lightning`,
          `DROP INDEX brz_idx_payment_details_lightning_payment_hash ON brz_payment_details_lightning`,
          `CREATE INDEX brz_idx_payment_details_lightning_user_invoice
             ON brz_payment_details_lightning(user_id, invoice(255))`,
          `CREATE INDEX brz_idx_payment_details_lightning_user_payment_hash
             ON brz_payment_details_lightning(user_id, payment_hash)`,

          ...scopeTable("brz_payment_details_token", "payment_id"),
          ...scopeTable("brz_payment_details_spark", "payment_id"),
          ...scopeTable("brz_lnurl_receive_metadata", "payment_hash"),
          ...scopeTable("brz_unclaimed_deposits", "txid, vout"),
          ...scopeTable("brz_contacts", "id"),
          ...scopeTable("brz_settings", "`key`"),

          // brz_sync_revision was a singleton (PK id=1, CHECK id=1). Drop the PK
          // and the id column, then re-key by user_id.
          { op: "dropPrimaryKey", table: "brz_sync_revision" },
          `ALTER TABLE brz_sync_revision DROP COLUMN id`,
          `ALTER TABLE brz_sync_revision ADD COLUMN user_id VARBINARY(33) NULL`,
          `UPDATE brz_sync_revision SET user_id = ${idLit} WHERE user_id IS NULL`,
          `ALTER TABLE brz_sync_revision MODIFY COLUMN user_id VARBINARY(33) NOT NULL`,
          `ALTER TABLE brz_sync_revision ADD PRIMARY KEY (user_id)`,

          // brz_sync_outgoing has no PK, only an index — just add user_id and
          // rewrite the index.
          `ALTER TABLE brz_sync_outgoing ADD COLUMN user_id VARBINARY(33) NULL`,
          `UPDATE brz_sync_outgoing SET user_id = ${idLit} WHERE user_id IS NULL`,
          `ALTER TABLE brz_sync_outgoing MODIFY COLUMN user_id VARBINARY(33) NOT NULL`,
          `DROP INDEX brz_idx_sync_outgoing_data_id_record_type ON brz_sync_outgoing`,
          `CREATE INDEX brz_idx_sync_outgoing_user_record_type_data_id
             ON brz_sync_outgoing(user_id, record_type, data_id)`,

          ...scopeTable("brz_sync_state", "record_type, data_id"),

          ...scopeTable("brz_sync_incoming", "record_type, data_id, revision"),
          `DROP INDEX brz_idx_sync_incoming_revision ON brz_sync_incoming`,
          `CREATE INDEX brz_idx_sync_incoming_user_revision
             ON brz_sync_incoming(user_id, revision)`,
        ],
      },
      {
        // Pin the migration-tracking table's `applied_at` default to UTC.
        // The migration manager already passes `UTC_TIMESTAMP(6)` explicitly
        // on INSERT, but aligning the default keeps `SHOW CREATE TABLE`
        // output consistent with the token-store / tree-store migrations
        // table and avoids future mistakes if a callsite omits the column.
        name: "Pin schema-migrations applied_at default to UTC",
        sql: [
          `ALTER TABLE brz_schema_migrations MODIFY COLUMN applied_at DATETIME(6) NOT NULL DEFAULT (UTC_TIMESTAMP(6))`,
        ],
      },
      {
        // Move deposit details into their own table so vout can be NOT NULL and
        // the schema matches brz_payment_details_lightning / _token / _spark. We
        // can't safely backfill the new table from the dropped deposit_tx_id
        // column: we never stored the original SSP output_index, and vout=0 is a
        // valid output index, so defaulting would silently mislabel. Drop the
        // column and leave the brz_payments row in place. The read path sees an
        // unjoined deposit row as `details: None` until the resync re-fetches the
        // SSP user_request and the upsert inserts the new details row.
        name: "Move deposit details into brz_payment_details_deposit table",
        sql: [
          `CREATE TABLE IF NOT EXISTS brz_payment_details_deposit (
              user_id VARBINARY(33) NOT NULL,
              payment_id VARCHAR(255) NOT NULL,
              tx_id VARCHAR(255) NOT NULL,
              vout INT UNSIGNED NOT NULL,
              PRIMARY KEY (user_id, payment_id)
          )`,
          `ALTER TABLE brz_payments DROP COLUMN deposit_tx_id`,
          `UPDATE brz_settings
           SET value = JSON_SET(value, '$.offset', 0)
           WHERE \`key\` = 'sync_offset' AND value IS NOT NULL`,
        ],
      },
      {
        // Backfill the conversion_info `type` discriminator for the
        // ConversionInfo enum refactor. All existing rows are AMM.
        // Mirrors the Rust mysql migration of the same name.
        name: "Backfill conversion_info type discriminator",
        sql: [
          `UPDATE brz_payment_metadata SET conversion_info = JSON_SET(conversion_info, '$.type', 'amm') WHERE conversion_info IS NOT NULL AND JSON_EXTRACT(conversion_info, '$.type') IS NULL`,
        ],
      },
    ];
  }
}

async function _mysqlTableExists(conn, tableName) {
  const [rows] = await conn.query(
    `SELECT COUNT(*) AS c FROM information_schema.tables
     WHERE table_schema = DATABASE() AND table_name = ?`,
    [tableName]
  );
  return Number(rows[0].c) > 0;
}

async function _mysqlIndexExists(conn, tableName, indexName) {
  const [rows] = await conn.query(
    `SELECT COUNT(*) AS c FROM information_schema.statistics
     WHERE table_schema = DATABASE() AND table_name = ? AND index_name = ?`,
    [tableName, indexName]
  );
  return Number(rows[0].c) > 0;
}

module.exports = { MysqlMigrationManager };
