/**
 * Database Migration Manager for Breez SDK PostgreSQL Storage
 *
 * Uses a brz_schema_migrations table + pg_advisory_xact_lock to safely run
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
   *
   * @param {import('pg').Pool} pool
   * @param {Buffer|Uint8Array} identity - 33-byte secp256k1 compressed pubkey
   *   identifying the tenant. Used to backfill `user_id` columns in the
   *   multi-tenant migration so that pre-existing single-tenant data remains
   *   readable.
   */
  async migrate(pool, identity) {
    const client = await pool.connect();
    try {
      await client.query("BEGIN");

      // Transaction-level advisory lock — automatically released on COMMIT/ROLLBACK
      await client.query(`SELECT pg_advisory_xact_lock(${MIGRATION_LOCK_ID})`);

      await this._applySchemaRenames(client);

      // Create the migrations tracking table if needed
      await client.query(`
        CREATE TABLE IF NOT EXISTS brz_schema_migrations (
          version INTEGER PRIMARY KEY,
          applied_at TIMESTAMPTZ DEFAULT NOW()
        )
      `);

      // Get current version
      const versionResult = await client.query(
        "SELECT COALESCE(MAX(version), 0) AS version FROM brz_schema_migrations"
      );
      const currentVersion = versionResult.rows[0].version;

      const migrations = this._getMigrations(identity);

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
          "INSERT INTO brz_schema_migrations (version) VALUES ($1)",
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

  /**
   * Pre-prefix rename. Canary-gated on the legacy `schema_migrations` table.
   * @param {import('pg').PoolClient} client
   */
  async _applySchemaRenames(client) {
    const canary = await client.query(
      `SELECT EXISTS (
         SELECT 1 FROM information_schema.tables
         WHERE table_schema = current_schema()
           AND table_name = 'schema_migrations'
       ) AS exists`
    );
    if (!canary.rows[0].exists) {
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
      await client.query(`ALTER TABLE IF EXISTS ${oldName} RENAME TO ${newName}`);
    }

    const indexRenames = [
      ["idx_payments_user_timestamp", "brz_idx_payments_user_timestamp"],
      ["idx_payments_user_payment_type", "brz_idx_payments_user_payment_type"],
      ["idx_payments_user_status", "brz_idx_payments_user_status"],
      ["idx_payment_metadata_user_parent", "brz_idx_payment_metadata_user_parent"],
      [
        "idx_payment_details_lightning_user_invoice",
        "brz_idx_payment_details_lightning_user_invoice",
      ],
      [
        "idx_payment_details_lightning_user_payment_hash",
        "brz_idx_payment_details_lightning_user_payment_hash",
      ],
      [
        "idx_sync_outgoing_user_record_type_data_id",
        "brz_idx_sync_outgoing_user_record_type_data_id",
      ],
      ["idx_sync_incoming_user_revision", "brz_idx_sync_incoming_user_revision"],
      // Pre-multi-tenant indexes (still present on version < 16 DBs).
      ["idx_payments_timestamp", "brz_idx_payments_timestamp"],
      ["idx_payments_payment_type", "brz_idx_payments_payment_type"],
      ["idx_payments_status", "brz_idx_payments_status"],
      ["idx_payment_metadata_parent", "brz_idx_payment_metadata_parent"],
      [
        "idx_payment_details_lightning_invoice",
        "brz_idx_payment_details_lightning_invoice",
      ],
      [
        "idx_payment_details_lightning_payment_hash",
        "brz_idx_payment_details_lightning_payment_hash",
      ],
      [
        "idx_sync_outgoing_data_id_record_type",
        "brz_idx_sync_outgoing_data_id_record_type",
      ],
      ["idx_sync_incoming_revision", "brz_idx_sync_incoming_revision"],
    ];
    for (const [oldName, newName] of indexRenames) {
      await client.query(`ALTER INDEX IF EXISTS ${oldName} RENAME TO ${newName}`);
    }

    const constraintRenames = [
      ["brz_payments", "payments_pkey", "brz_payments_pkey"],
      ["brz_settings", "settings_pkey", "brz_settings_pkey"],
      ["brz_unclaimed_deposits", "unclaimed_deposits_pkey", "brz_unclaimed_deposits_pkey"],
      ["brz_payment_metadata", "payment_metadata_pkey", "brz_payment_metadata_pkey"],
      [
        "brz_payment_details_lightning",
        "payment_details_lightning_pkey",
        "brz_payment_details_lightning_pkey",
      ],
      [
        "brz_payment_details_token",
        "payment_details_token_pkey",
        "brz_payment_details_token_pkey",
      ],
      [
        "brz_payment_details_spark",
        "payment_details_spark_pkey",
        "brz_payment_details_spark_pkey",
      ],
      [
        "brz_lnurl_receive_metadata",
        "lnurl_receive_metadata_pkey",
        "brz_lnurl_receive_metadata_pkey",
      ],
      ["brz_sync_revision", "sync_revision_pkey", "brz_sync_revision_pkey"],
      ["brz_sync_state", "sync_state_pkey", "brz_sync_state_pkey"],
      ["brz_sync_incoming", "sync_incoming_pkey", "brz_sync_incoming_pkey"],
      ["brz_contacts", "contacts_pkey", "brz_contacts_pkey"],
    ];
    for (const [table, oldName, newName] of constraintRenames) {
      await client.query(
        `DO $$ BEGIN
           IF EXISTS (
             SELECT 1 FROM information_schema.table_constraints
             WHERE table_schema = current_schema()
               AND table_name = '${table}'
               AND constraint_name = '${oldName}'
           ) THEN
             ALTER TABLE ${table} RENAME CONSTRAINT ${oldName} TO ${newName};
           END IF;
         END $$`
      );
    }

    await client.query(
      `ALTER TABLE IF EXISTS schema_migrations RENAME TO brz_schema_migrations`
    );
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
   *
   * @param {Buffer|Uint8Array} identity - 33-byte tenant identity. Inlined as
   *   a hex BYTEA literal in the multi-tenant scoping migration. Safe because
   *   the bytes come from a typed secp256k1 pubkey (character set
   *   `[0-9a-f]{66}` after hex encoding) — not user-controlled input.
   */
  _getMigrations(identity) {
    const idHex = Buffer.from(identity).toString("hex");
    const idLit = `'\\x${idHex}'::bytea`;

    // Helper for the per-table backfill: ADD COLUMN nullable -> UPDATE -> SET
    // NOT NULL + drop/recreate PK. Returns an array of statements.
    const scopeTable = (table, pkCols) => [
      `ALTER TABLE ${table} ADD COLUMN user_id BYTEA`,
      `UPDATE ${table} SET user_id = ${idLit}`,
      `ALTER TABLE ${table}
         ALTER COLUMN user_id SET NOT NULL,
         DROP CONSTRAINT IF EXISTS ${table}_pkey,
         ADD PRIMARY KEY (user_id, ${pkCols})`,
    ];

    return [
      {
        name: "Create all tables at final schema",
        sql: [
          // -- Core tables --
          `CREATE TABLE IF NOT EXISTS brz_payments (
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

          `CREATE TABLE IF NOT EXISTS brz_settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
          )`,

          `CREATE TABLE IF NOT EXISTS brz_unclaimed_deposits (
            txid TEXT NOT NULL,
            vout INTEGER NOT NULL,
            amount_sats BIGINT,
            claim_error JSONB,
            refund_tx TEXT,
            refund_tx_id TEXT,
            PRIMARY KEY (txid, vout)
          )`,

          `CREATE TABLE IF NOT EXISTS brz_payment_metadata (
            payment_id TEXT PRIMARY KEY,
            parent_payment_id TEXT,
            lnurl_pay_info JSONB,
            lnurl_withdraw_info JSONB,
            lnurl_description TEXT,
            conversion_info JSONB
          )`,

          `CREATE TABLE IF NOT EXISTS brz_payment_details_lightning (
            payment_id TEXT PRIMARY KEY,
            invoice TEXT NOT NULL,
            payment_hash TEXT NOT NULL,
            destination_pubkey TEXT NOT NULL,
            description TEXT,
            preimage TEXT,
            htlc_status TEXT NOT NULL,
            htlc_expiry_time BIGINT NOT NULL
          )`,

          `CREATE TABLE IF NOT EXISTS brz_payment_details_token (
            payment_id TEXT PRIMARY KEY,
            metadata JSONB NOT NULL,
            tx_hash TEXT NOT NULL,
            tx_type TEXT NOT NULL,
            invoice_details JSONB
          )`,

          `CREATE TABLE IF NOT EXISTS brz_payment_details_spark (
            payment_id TEXT PRIMARY KEY,
            invoice_details JSONB,
            htlc_details JSONB
          )`,

          `CREATE TABLE IF NOT EXISTS brz_lnurl_receive_metadata (
            payment_hash TEXT PRIMARY KEY,
            nostr_zap_request TEXT,
            nostr_zap_receipt TEXT,
            sender_comment TEXT,
            preimage TEXT
          )`,

          // -- Sync tables --
          `CREATE TABLE IF NOT EXISTS brz_sync_revision (
            id INTEGER PRIMARY KEY DEFAULT 1,
            revision BIGINT NOT NULL DEFAULT 0,
            CHECK (id = 1)
          )`,
          `INSERT INTO brz_sync_revision (id, revision) VALUES (1, 0) ON CONFLICT (id) DO NOTHING`,

          `CREATE TABLE IF NOT EXISTS brz_sync_outgoing (
            record_type TEXT NOT NULL,
            data_id TEXT NOT NULL,
            schema_version TEXT NOT NULL,
            commit_time BIGINT NOT NULL,
            updated_fields_json JSONB NOT NULL,
            revision BIGINT NOT NULL
          )`,

          `CREATE TABLE IF NOT EXISTS brz_sync_state (
            record_type TEXT NOT NULL,
            data_id TEXT NOT NULL,
            schema_version TEXT NOT NULL,
            commit_time BIGINT NOT NULL,
            data JSONB NOT NULL,
            revision BIGINT NOT NULL,
            PRIMARY KEY (record_type, data_id)
          )`,

          `CREATE TABLE IF NOT EXISTS brz_sync_incoming (
            record_type TEXT NOT NULL,
            data_id TEXT NOT NULL,
            schema_version TEXT NOT NULL,
            commit_time BIGINT NOT NULL,
            data JSONB NOT NULL,
            revision BIGINT NOT NULL,
            PRIMARY KEY (record_type, data_id, revision)
          )`,

          // -- Indexes --
          `CREATE INDEX IF NOT EXISTS brz_idx_payments_timestamp ON brz_payments(timestamp)`,
          `CREATE INDEX IF NOT EXISTS brz_idx_payments_payment_type ON brz_payments(payment_type)`,
          `CREATE INDEX IF NOT EXISTS brz_idx_payments_status ON brz_payments(status)`,
          `CREATE INDEX IF NOT EXISTS brz_idx_payment_details_lightning_invoice ON brz_payment_details_lightning(invoice)`,
          `CREATE INDEX IF NOT EXISTS brz_idx_payment_details_lightning_payment_hash ON brz_payment_details_lightning(payment_hash)`,
          `CREATE INDEX IF NOT EXISTS brz_idx_payment_metadata_parent ON brz_payment_metadata(parent_payment_id)`,
          `CREATE INDEX IF NOT EXISTS brz_idx_sync_outgoing_data_id_record_type ON brz_sync_outgoing(record_type, data_id)`,
          `CREATE INDEX IF NOT EXISTS brz_idx_sync_incoming_revision ON brz_sync_incoming(revision)`,
        ],
      },
      {
        name: "Create brz_contacts table",
        sql: [
          `CREATE TABLE IF NOT EXISTS brz_contacts (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            payment_identifier TEXT NOT NULL,
            created_at BIGINT NOT NULL,
            updated_at BIGINT NOT NULL
          )`,
        ],
      },
      {
        name: "Drop preimage column from brz_lnurl_receive_metadata",
        sql: [
          `ALTER TABLE brz_lnurl_receive_metadata DROP COLUMN IF EXISTS preimage`,
        ],
      },
      {
        name: "Clear cached lightning address for CachedLightningAddress format change",
        sql: [
          `DELETE FROM brz_settings WHERE key = 'lightning_address'`,
        ],
      },
      {
        name: "Add is_mature to brz_unclaimed_deposits",
        sql: [
          `ALTER TABLE brz_unclaimed_deposits ADD COLUMN is_mature BOOLEAN NOT NULL DEFAULT TRUE`,
        ],
      },
      {
        name: "Add conversion_status to brz_payment_metadata",
        sql: [
          `ALTER TABLE brz_payment_metadata ADD COLUMN IF NOT EXISTS conversion_status TEXT`,
        ],
      },
      {
        name: "Multi-tenant scoping: add user_id and rewrite primary keys",
        sql: [
          // Per-user tables
          ...scopeTable("brz_payments", "id"),
          `DROP INDEX IF EXISTS brz_idx_payments_timestamp`,
          `DROP INDEX IF EXISTS brz_idx_payments_payment_type`,
          `DROP INDEX IF EXISTS brz_idx_payments_status`,
          `CREATE INDEX brz_idx_payments_user_timestamp ON brz_payments(user_id, timestamp)`,
          `CREATE INDEX brz_idx_payments_user_payment_type ON brz_payments(user_id, payment_type)`,
          `CREATE INDEX brz_idx_payments_user_status ON brz_payments(user_id, status)`,

          ...scopeTable("brz_payment_metadata", "payment_id"),
          `DROP INDEX IF EXISTS brz_idx_payment_metadata_parent`,
          `CREATE INDEX brz_idx_payment_metadata_user_parent
             ON brz_payment_metadata(user_id, parent_payment_id)`,

          ...scopeTable("brz_payment_details_lightning", "payment_id"),
          `DROP INDEX IF EXISTS brz_idx_payment_details_lightning_invoice`,
          `DROP INDEX IF EXISTS brz_idx_payment_details_lightning_payment_hash`,
          `CREATE INDEX brz_idx_payment_details_lightning_user_invoice
             ON brz_payment_details_lightning(user_id, invoice)`,
          `CREATE INDEX brz_idx_payment_details_lightning_user_payment_hash
             ON brz_payment_details_lightning(user_id, payment_hash)`,

          ...scopeTable("brz_payment_details_token", "payment_id"),
          ...scopeTable("brz_payment_details_spark", "payment_id"),
          ...scopeTable("brz_lnurl_receive_metadata", "payment_hash"),
          ...scopeTable("brz_unclaimed_deposits", "txid, vout"),
          ...scopeTable("brz_contacts", "id"),
          ...scopeTable("brz_settings", "key"),

          // brz_sync_revision: drop the singleton id (CASCADE removes PK + CHECK),
          // then re-key by user_id so each tenant has its own revision row.
          `ALTER TABLE brz_sync_revision DROP COLUMN id CASCADE`,
          `ALTER TABLE brz_sync_revision ADD COLUMN user_id BYTEA`,
          `UPDATE brz_sync_revision SET user_id = ${idLit}`,
          `ALTER TABLE brz_sync_revision
             ALTER COLUMN user_id SET NOT NULL,
             ADD PRIMARY KEY (user_id)`,

          // brz_sync_outgoing has no PK, only an index — just add user_id and rewrite the index.
          `ALTER TABLE brz_sync_outgoing ADD COLUMN user_id BYTEA`,
          `UPDATE brz_sync_outgoing SET user_id = ${idLit}`,
          `ALTER TABLE brz_sync_outgoing ALTER COLUMN user_id SET NOT NULL`,
          `DROP INDEX IF EXISTS brz_idx_sync_outgoing_data_id_record_type`,
          `CREATE INDEX brz_idx_sync_outgoing_user_record_type_data_id
             ON brz_sync_outgoing(user_id, record_type, data_id)`,

          ...scopeTable("brz_sync_state", "record_type, data_id"),

          ...scopeTable("brz_sync_incoming", "record_type, data_id, revision"),
          `DROP INDEX IF EXISTS brz_idx_sync_incoming_revision`,
          `CREATE INDEX brz_idx_sync_incoming_user_revision
             ON brz_sync_incoming(user_id, revision)`,
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
              user_id BYTEA NOT NULL,
              payment_id TEXT NOT NULL,
              tx_id TEXT NOT NULL,
              vout BIGINT NOT NULL,
              PRIMARY KEY (user_id, payment_id)
          )`,
          `ALTER TABLE brz_payments DROP COLUMN IF EXISTS deposit_tx_id`,
          `UPDATE brz_settings
           SET value = jsonb_set(value::jsonb, '{offset}', '0')::text
           WHERE key = 'sync_offset' AND value IS NOT NULL`,
        ],
      },
      {
        name: "Backfill conversion_info type discriminator",
        sql: [
          `UPDATE brz_payment_metadata SET conversion_info = conversion_info::jsonb || '{"type": "amm"}'::jsonb WHERE conversion_info IS NOT NULL AND conversion_info::jsonb->>'type' IS NULL`,
        ],
      },
    ];
  }
}

module.exports = { PostgresMigrationManager };
