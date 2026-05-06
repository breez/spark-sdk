/**
 * Database Migration Manager for Breez SDK PostgreSQL Token Store
 *
 * Uses a token_schema_migrations table + pg_advisory_xact_lock to safely run
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

      // Create the migrations tracking table if needed
      await client.query(`
        CREATE TABLE IF NOT EXISTS token_schema_migrations (
          version INTEGER PRIMARY KEY,
          applied_at TIMESTAMPTZ DEFAULT NOW()
        )
      `);

      // Get current version
      const versionResult = await client.query(
        "SELECT COALESCE(MAX(version), 0) AS version FROM token_schema_migrations"
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
          "INSERT INTO token_schema_migrations (version) VALUES ($1)",
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
          `CREATE TABLE IF NOT EXISTS token_metadata (
            identifier TEXT PRIMARY KEY,
            issuer_public_key TEXT NOT NULL,
            name TEXT NOT NULL,
            ticker TEXT NOT NULL,
            decimals INTEGER NOT NULL,
            max_supply TEXT NOT NULL,
            is_freezable BOOLEAN NOT NULL,
            creation_entity_public_key TEXT
          )`,

          `CREATE INDEX IF NOT EXISTS idx_token_metadata_issuer_pk
            ON token_metadata (issuer_public_key)`,

          `CREATE TABLE IF NOT EXISTS token_reservations (
            id TEXT PRIMARY KEY,
            purpose TEXT NOT NULL,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
          )`,

          `CREATE TABLE IF NOT EXISTS token_outputs (
            id TEXT PRIMARY KEY,
            token_identifier TEXT NOT NULL REFERENCES token_metadata(identifier),
            owner_public_key TEXT NOT NULL,
            revocation_commitment TEXT NOT NULL,
            withdraw_bond_sats BIGINT NOT NULL,
            withdraw_relative_block_locktime BIGINT NOT NULL,
            token_public_key TEXT,
            token_amount TEXT NOT NULL,
            prev_tx_hash TEXT NOT NULL,
            prev_tx_vout INTEGER NOT NULL,
            reservation_id TEXT REFERENCES token_reservations(id) ON DELETE SET NULL,
            added_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
          )`,

          `CREATE INDEX IF NOT EXISTS idx_token_outputs_identifier
            ON token_outputs (token_identifier)`,

          `CREATE INDEX IF NOT EXISTS idx_token_outputs_reservation
            ON token_outputs (reservation_id) WHERE reservation_id IS NOT NULL`,

          `CREATE TABLE IF NOT EXISTS token_spent_outputs (
            output_id TEXT PRIMARY KEY,
            spent_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
          )`,

          `CREATE TABLE IF NOT EXISTS token_swap_status (
            id INTEGER PRIMARY KEY DEFAULT 1 CHECK (id = 1),
            last_completed_at TIMESTAMPTZ
          )`,

          `INSERT INTO token_swap_status (id) VALUES (1) ON CONFLICT DO NOTHING`,
        ],
      },
      {
        // Mirrors Rust migration 2 in spark-postgres/src/token_store.rs.
        // Adds user_id to every token-store table (including token_metadata —
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
          `ALTER TABLE token_outputs
             DROP CONSTRAINT IF EXISTS token_outputs_reservation_id_fkey`,
          `ALTER TABLE token_outputs
             DROP CONSTRAINT IF EXISTS token_outputs_token_identifier_fkey`,

          // token_metadata: per-tenant scoping (privacy — see header).
          `ALTER TABLE token_metadata ADD COLUMN user_id BYTEA`,
          `UPDATE token_metadata SET user_id = ${idLit}`,
          `ALTER TABLE token_metadata
             ALTER COLUMN user_id SET NOT NULL,
             DROP CONSTRAINT IF EXISTS token_metadata_pkey,
             ADD PRIMARY KEY (user_id, identifier)`,
          `DROP INDEX IF EXISTS idx_token_metadata_issuer_pk`,
          `CREATE INDEX idx_token_metadata_user_issuer_pk
             ON token_metadata (user_id, issuer_public_key)`,

          // token_reservations: scope by user_id.
          `ALTER TABLE token_reservations ADD COLUMN user_id BYTEA`,
          `UPDATE token_reservations SET user_id = ${idLit}`,
          `ALTER TABLE token_reservations
             ALTER COLUMN user_id SET NOT NULL,
             DROP CONSTRAINT IF EXISTS token_reservations_pkey,
             ADD PRIMARY KEY (user_id, id)`,

          // token_outputs: scope by user_id, rekey, re-add composite FKs.
          `ALTER TABLE token_outputs ADD COLUMN user_id BYTEA`,
          `UPDATE token_outputs SET user_id = ${idLit}`,
          `ALTER TABLE token_outputs
             ALTER COLUMN user_id SET NOT NULL,
             DROP CONSTRAINT IF EXISTS token_outputs_pkey,
             ADD PRIMARY KEY (user_id, id),
             ADD FOREIGN KEY (user_id, token_identifier)
                REFERENCES token_metadata(user_id, identifier),
             ADD FOREIGN KEY (user_id, reservation_id)
                REFERENCES token_reservations(user_id, id)`,
          `DROP INDEX IF EXISTS idx_token_outputs_identifier`,
          `DROP INDEX IF EXISTS idx_token_outputs_reservation`,
          `CREATE INDEX idx_token_outputs_user_identifier
             ON token_outputs (user_id, token_identifier)`,
          `CREATE INDEX idx_token_outputs_user_reservation
             ON token_outputs (user_id, reservation_id)
             WHERE reservation_id IS NOT NULL`,

          // token_spent_outputs: scope by user_id.
          `ALTER TABLE token_spent_outputs ADD COLUMN user_id BYTEA`,
          `UPDATE token_spent_outputs SET user_id = ${idLit}`,
          `ALTER TABLE token_spent_outputs
             ALTER COLUMN user_id SET NOT NULL,
             DROP CONSTRAINT IF EXISTS token_spent_outputs_pkey,
             ADD PRIMARY KEY (user_id, output_id)`,

          // token_swap_status: drop the singleton id, rekey by user_id.
          `ALTER TABLE token_swap_status DROP COLUMN id CASCADE`,
          `ALTER TABLE token_swap_status ADD COLUMN user_id BYTEA`,
          `UPDATE token_swap_status SET user_id = ${idLit}`,
          `ALTER TABLE token_swap_status
             ALTER COLUMN user_id SET NOT NULL,
             ADD PRIMARY KEY (user_id)`,
        ],
      },
    ];
  }
}

module.exports = { TokenStoreMigrationManager };
