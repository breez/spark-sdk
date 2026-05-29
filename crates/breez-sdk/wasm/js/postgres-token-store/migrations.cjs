/**
 * Database Migration Manager for Breez SDK PostgreSQL Token Store
 *
 * Uses a brz_token_schema_migrations table + pg_advisory_xact_lock to safely run
 * migrations from concurrent processes.
 */

const { TokenStoreError } = require("./errors.cjs");

/**
 * Advisory lock ID for token store migrations.
 * Uses a different lock ID from the storage/tree store migrations to avoid contention.
 * Derived from ASCII bytes of "TOKN" (0x544F4B4E).
 */
const MIGRATION_LOCK_ID = "1414022990"; // 0x544F4B4E as decimal string

class TokenStoreMigrationManager {
  constructor(logger = null) {
    this.logger = logger;
  }

  /**
   * Run all pending migrations inside a single transaction with an advisory lock.
   * @param {import('pg').Pool} pool
   * @param {Buffer|Uint8Array} identity - 33-byte secp256k1 compressed pubkey
   *   identifying the tenant. Used to backfill `user_id` columns in the
   *   multi-tenant scoping migration. Required.
   */
  async migrate(pool, identity) {
    if (!identity || identity.length !== 33) {
      throw new TokenStoreError(
        "tenant identity (33-byte secp256k1 pubkey) is required"
      );
    }
    const client = await pool.connect();
    try {
      await client.query("BEGIN");

      // Transaction-level advisory lock — automatically released on COMMIT/ROLLBACK
      await client.query(`SELECT pg_advisory_xact_lock(${MIGRATION_LOCK_ID})`);

      await this._applySchemaRenames(client);

      // Create the migrations tracking table if needed
      await client.query(`
        CREATE TABLE IF NOT EXISTS brz_token_schema_migrations (
          version INTEGER PRIMARY KEY,
          applied_at TIMESTAMPTZ DEFAULT NOW()
        )
      `);

      // Get current version
      const versionResult = await client.query(
        "SELECT COALESCE(MAX(version), 0) AS version FROM brz_token_schema_migrations"
      );
      const currentVersion = versionResult.rows[0].version;

      const migrations = this._getMigrations(identity);

      if (currentVersion >= migrations.length) {
        this._log("info", `Token store database is up to date (version ${currentVersion})`);
        await client.query("COMMIT");
        return;
      }

      this._log(
        "info",
        `Migrating token store database from version ${currentVersion} to ${migrations.length}`
      );

      for (let i = currentVersion; i < migrations.length; i++) {
        const migration = migrations[i];
        const version = i + 1;
        this._log("debug", `Running token store migration ${version}: ${migration.name}`);

        for (const sql of migration.sql) {
          await client.query(sql);
        }

        await client.query(
          "INSERT INTO brz_token_schema_migrations (version) VALUES ($1)",
          [version]
        );
      }

      await client.query("COMMIT");
      this._log("info", "Token store database migration completed successfully");
    } catch (error) {
      await client.query("ROLLBACK").catch(() => {});
      throw new TokenStoreError(
        `Token store migration failed: ${error.message}`,
        error
      );
    } finally {
      client.release();
    }
  }

  /**
   * Pre-prefix rename. Canary-gated on the legacy `token_schema_migrations`
   * table.
   * @param {import('pg').PoolClient} client
   */
  async _applySchemaRenames(client) {
    const canary = await client.query(
      `SELECT EXISTS (
         SELECT 1 FROM information_schema.tables
         WHERE table_schema = current_schema()
           AND table_name = 'token_schema_migrations'
       ) AS exists`
    );
    if (!canary.rows[0].exists) {
      return;
    }

    const tableRenames = [
      ["token_metadata", "brz_token_metadata"],
      ["token_reservations", "brz_token_reservations"],
      ["token_outputs", "brz_token_outputs"],
      ["token_spent_outputs", "brz_token_spent_outputs"],
      ["token_swap_status", "brz_token_swap_status"],
    ];
    for (const [oldName, newName] of tableRenames) {
      await client.query(`ALTER TABLE IF EXISTS ${oldName} RENAME TO ${newName}`);
    }

    const indexRenames = [
      ["idx_token_metadata_user_issuer_pk", "brz_idx_token_metadata_user_issuer_pk"],
      ["idx_token_outputs_user_identifier", "brz_idx_token_outputs_user_identifier"],
      ["idx_token_outputs_user_reservation", "brz_idx_token_outputs_user_reservation"],
      // Pre-multi-tenant indexes (dropped by the multi-tenant migration).
      ["idx_token_metadata_issuer_pk", "brz_idx_token_metadata_issuer_pk"],
      ["idx_token_outputs_identifier", "brz_idx_token_outputs_identifier"],
      ["idx_token_outputs_reservation", "brz_idx_token_outputs_reservation"],
    ];
    for (const [oldName, newName] of indexRenames) {
      await client.query(`ALTER INDEX IF EXISTS ${oldName} RENAME TO ${newName}`);
    }

    const constraintRenames = [
      ["brz_token_metadata", "token_metadata_pkey", "brz_token_metadata_pkey"],
      ["brz_token_reservations", "token_reservations_pkey", "brz_token_reservations_pkey"],
      ["brz_token_outputs", "token_outputs_pkey", "brz_token_outputs_pkey"],
      [
        "brz_token_outputs",
        "token_outputs_user_id_token_identifier_fkey",
        "brz_token_outputs_user_id_token_identifier_fkey",
      ],
      [
        "brz_token_outputs",
        "token_outputs_user_id_reservation_id_fkey",
        "brz_token_outputs_user_id_reservation_id_fkey",
      ],
      // Pre-multi-tenant FKs (single-column). Rename so the post-tenant
      // migration's `DROP CONSTRAINT IF EXISTS brz_*_fkey` finds them.
      [
        "brz_token_outputs",
        "token_outputs_token_identifier_fkey",
        "brz_token_outputs_token_identifier_fkey",
      ],
      [
        "brz_token_outputs",
        "token_outputs_reservation_id_fkey",
        "brz_token_outputs_reservation_id_fkey",
      ],
      ["brz_token_spent_outputs", "token_spent_outputs_pkey", "brz_token_spent_outputs_pkey"],
      ["brz_token_swap_status", "token_swap_status_pkey", "brz_token_swap_status_pkey"],
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
      `ALTER TABLE IF EXISTS token_schema_migrations RENAME TO brz_token_schema_migrations`
    );
  }

  _log(level, message) {
    if (this.logger && typeof this.logger.log === "function") {
      this.logger.log({ line: message, level });
    } else if (level === "error") {
      console.error(`[TokenStoreMigrationManager] ${message}`);
    }
  }

  /**
   * Migrations matching the Rust PostgresTokenStore schema exactly.
   *
   * @param {Buffer|Uint8Array} identity - tenant identity inlined as a hex
   *   BYTEA literal in the multi-tenant scoping migration. Safe because the
   *   bytes come from a typed secp256k1 pubkey (`[0-9a-f]{66}` after hex
   *   encoding) — not user-controlled input.
   */
  _getMigrations(identity) {
    const idHex = Buffer.from(identity).toString("hex");
    const idLit = `'\\x${idHex}'::bytea`;

    return [
      {
        name: "Create token store tables with race condition protection",
        sql: [
          `CREATE TABLE IF NOT EXISTS brz_token_metadata (
            identifier TEXT PRIMARY KEY,
            issuer_public_key TEXT NOT NULL,
            name TEXT NOT NULL,
            ticker TEXT NOT NULL,
            decimals INTEGER NOT NULL,
            max_supply TEXT NOT NULL,
            is_freezable BOOLEAN NOT NULL,
            creation_entity_public_key TEXT
          )`,

          `CREATE INDEX IF NOT EXISTS brz_idx_token_metadata_issuer_pk
            ON brz_token_metadata (issuer_public_key)`,

          `CREATE TABLE IF NOT EXISTS brz_token_reservations (
            id TEXT PRIMARY KEY,
            purpose TEXT NOT NULL,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
          )`,

          `CREATE TABLE IF NOT EXISTS brz_token_outputs (
            id TEXT PRIMARY KEY,
            token_identifier TEXT NOT NULL REFERENCES brz_token_metadata(identifier),
            owner_public_key TEXT NOT NULL,
            revocation_commitment TEXT NOT NULL,
            withdraw_bond_sats BIGINT NOT NULL,
            withdraw_relative_block_locktime BIGINT NOT NULL,
            token_public_key TEXT,
            token_amount TEXT NOT NULL,
            prev_tx_hash TEXT NOT NULL,
            prev_tx_vout INTEGER NOT NULL,
            reservation_id TEXT REFERENCES brz_token_reservations(id) ON DELETE SET NULL,
            added_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
          )`,

          `CREATE INDEX IF NOT EXISTS brz_idx_token_outputs_identifier
            ON brz_token_outputs (token_identifier)`,

          `CREATE INDEX IF NOT EXISTS brz_idx_token_outputs_reservation
            ON brz_token_outputs (reservation_id) WHERE reservation_id IS NOT NULL`,

          `CREATE TABLE IF NOT EXISTS brz_token_spent_outputs (
            output_id TEXT PRIMARY KEY,
            spent_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
          )`,

          `CREATE TABLE IF NOT EXISTS brz_token_swap_status (
            id INTEGER PRIMARY KEY DEFAULT 1 CHECK (id = 1),
            last_completed_at TIMESTAMPTZ
          )`,

          `INSERT INTO brz_token_swap_status (id) VALUES (1) ON CONFLICT DO NOTHING`,
        ],
      },
      {
        // Mirrors Rust migration 2 in spark-postgres/src/token_store.rs.
        // Adds user_id to every token-store table (including brz_token_metadata —
        // per-tenant to avoid 0-balance leakage for tokens a tenant never
        // owned), backfills with the connecting tenant, and rewrites primary
        // keys / FKs / indexes to lead with user_id. Composite FKs use NO
        // ACTION because column-list SET NULL is PG15+ and a whole-row SET
        // NULL would null user_id (NOT NULL).
        name: "Multi-tenant scoping: add user_id and rewrite primary keys",
        sql: [
          // Drop dependent FKs FIRST so we can rebuild parent PKs they
          // reference. Inline `REFERENCES` clauses get auto-named
          // `<table>_<column>_fkey`.
          `ALTER TABLE brz_token_outputs
             DROP CONSTRAINT IF EXISTS brz_token_outputs_reservation_id_fkey`,
          `ALTER TABLE brz_token_outputs
             DROP CONSTRAINT IF EXISTS brz_token_outputs_token_identifier_fkey`,

          // brz_token_metadata: per-tenant scoping (privacy — see header).
          `ALTER TABLE brz_token_metadata ADD COLUMN user_id BYTEA`,
          `UPDATE brz_token_metadata SET user_id = ${idLit}`,
          `ALTER TABLE brz_token_metadata
             ALTER COLUMN user_id SET NOT NULL,
             DROP CONSTRAINT IF EXISTS brz_token_metadata_pkey,
             ADD PRIMARY KEY (user_id, identifier)`,
          `DROP INDEX IF EXISTS brz_idx_token_metadata_issuer_pk`,
          `CREATE INDEX brz_idx_token_metadata_user_issuer_pk
             ON brz_token_metadata (user_id, issuer_public_key)`,

          // brz_token_reservations: scope by user_id.
          `ALTER TABLE brz_token_reservations ADD COLUMN user_id BYTEA`,
          `UPDATE brz_token_reservations SET user_id = ${idLit}`,
          `ALTER TABLE brz_token_reservations
             ALTER COLUMN user_id SET NOT NULL,
             DROP CONSTRAINT IF EXISTS brz_token_reservations_pkey,
             ADD PRIMARY KEY (user_id, id)`,

          // brz_token_outputs: scope by user_id, rekey, re-add composite FKs.
          `ALTER TABLE brz_token_outputs ADD COLUMN user_id BYTEA`,
          `UPDATE brz_token_outputs SET user_id = ${idLit}`,
          `ALTER TABLE brz_token_outputs
             ALTER COLUMN user_id SET NOT NULL,
             DROP CONSTRAINT IF EXISTS brz_token_outputs_pkey,
             ADD PRIMARY KEY (user_id, id),
             ADD FOREIGN KEY (user_id, token_identifier)
                REFERENCES brz_token_metadata(user_id, identifier),
             ADD FOREIGN KEY (user_id, reservation_id)
                REFERENCES brz_token_reservations(user_id, id)`,
          `DROP INDEX IF EXISTS brz_idx_token_outputs_identifier`,
          `DROP INDEX IF EXISTS brz_idx_token_outputs_reservation`,
          `CREATE INDEX brz_idx_token_outputs_user_identifier
             ON brz_token_outputs (user_id, token_identifier)`,
          `CREATE INDEX brz_idx_token_outputs_user_reservation
             ON brz_token_outputs (user_id, reservation_id)
             WHERE reservation_id IS NOT NULL`,

          // brz_token_spent_outputs: scope by user_id.
          `ALTER TABLE brz_token_spent_outputs ADD COLUMN user_id BYTEA`,
          `UPDATE brz_token_spent_outputs SET user_id = ${idLit}`,
          `ALTER TABLE brz_token_spent_outputs
             ALTER COLUMN user_id SET NOT NULL,
             DROP CONSTRAINT IF EXISTS brz_token_spent_outputs_pkey,
             ADD PRIMARY KEY (user_id, output_id)`,

          // brz_token_swap_status: drop the singleton id, rekey by user_id.
          `ALTER TABLE brz_token_swap_status DROP COLUMN id CASCADE`,
          `ALTER TABLE brz_token_swap_status ADD COLUMN user_id BYTEA`,
          `UPDATE brz_token_swap_status SET user_id = ${idLit}`,
          `ALTER TABLE brz_token_swap_status
             ALTER COLUMN user_id SET NOT NULL,
             ADD PRIMARY KEY (user_id)`,
        ],
      },
      {
        // Mirrors Rust migration 3 in spark-postgres/src/token_store.rs.
        // Re-keys brz_token_spent_outputs by (prev_tx_hash, prev_tx_vout) instead
        // of the operator-issued output id. v3 FinalTokenOutput carries no id
        // field, so post-broadcast spent markers only have an outpoint to work
        // with. Existing output_id-keyed rows can't be backfilled (no outpoint
        // stored alongside them), so the table is wiped on upgrade — spent
        // markers are short-lived (5 minute cleanup window) so wiping is
        // equivalent to letting them age out.
        name: "Re-key spent outputs by (prev_tx_hash, prev_tx_vout)",
        sql: [
          `DROP TABLE IF EXISTS brz_token_spent_outputs`,
          `CREATE TABLE brz_token_spent_outputs (
             user_id BYTEA NOT NULL,
             prev_tx_hash TEXT NOT NULL,
             prev_tx_vout INTEGER NOT NULL,
             spent_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
             PRIMARY KEY (user_id, prev_tx_hash, prev_tx_vout)
           )`,
        ],
      },
      {
        // Re-key brz_token_outputs by (prev_tx_hash, prev_tx_vout) and drop the
        // legacy id column. id already held "{prev_tx_hash}:{vout}", so the
        // outpoint is the natural key. Dedup any duplicate-outpoint rows
        // (possible from pre-outpoint code) before adding the composite PK,
        // preferring rows that hold a reservation.
        name: "Re-key token outputs by (prev_tx_hash, prev_tx_vout), drop legacy id",
        sql: [
          `DELETE FROM brz_token_outputs WHERE ctid IN (
             SELECT ctid FROM (
               SELECT ctid, ROW_NUMBER() OVER (
                 PARTITION BY user_id, prev_tx_hash, prev_tx_vout
                 ORDER BY (reservation_id IS NULL) ASC, id ASC
               ) AS rn
               FROM brz_token_outputs
             ) t WHERE t.rn > 1
           )`,
          `ALTER TABLE brz_token_outputs
             DROP CONSTRAINT IF EXISTS brz_token_outputs_pkey,
             ADD PRIMARY KEY (user_id, prev_tx_hash, prev_tx_vout)`,
          `ALTER TABLE brz_token_outputs DROP COLUMN id`,
        ],
      },
    ];
  }
}

module.exports = { TokenStoreMigrationManager };
