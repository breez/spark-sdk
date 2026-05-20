/**
 * Database Migration Manager for Breez SDK PostgreSQL Tree Store
 *
 * Uses a brz_tree_schema_migrations table + pg_advisory_xact_lock to safely run
 * migrations from concurrent processes.
 */

const { TreeStoreError } = require("./errors.cjs");

/**
 * Advisory lock ID for tree store migrations.
 * Uses a different lock ID from the storage migrations to avoid contention.
 * Derived from ASCII bytes of "TREE" (0x54524545).
 */
const MIGRATION_LOCK_ID = "1414743365"; // 0x54524545 as decimal string

class TreeStoreMigrationManager {
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
      throw new TreeStoreError(
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
        CREATE TABLE IF NOT EXISTS brz_tree_schema_migrations (
          version INTEGER PRIMARY KEY,
          applied_at TIMESTAMPTZ DEFAULT NOW()
        )
      `);

      // Get current version
      const versionResult = await client.query(
        "SELECT COALESCE(MAX(version), 0) AS version FROM brz_tree_schema_migrations"
      );
      const currentVersion = versionResult.rows[0].version;

      const migrations = this._getMigrations(identity);

      if (currentVersion >= migrations.length) {
        this._log("info", `Tree store database is up to date (version ${currentVersion})`);
        await client.query("COMMIT");
        return;
      }

      this._log(
        "info",
        `Migrating tree store database from version ${currentVersion} to ${migrations.length}`
      );

      for (let i = currentVersion; i < migrations.length; i++) {
        const migration = migrations[i];
        const version = i + 1;
        this._log("debug", `Running tree store migration ${version}: ${migration.name}`);

        for (const sql of migration.sql) {
          await client.query(sql);
        }

        await client.query(
          "INSERT INTO brz_tree_schema_migrations (version) VALUES ($1)",
          [version]
        );
      }

      await client.query("COMMIT");
      this._log("info", "Tree store database migration completed successfully");
    } catch (error) {
      await client.query("ROLLBACK").catch(() => {});
      throw new TreeStoreError(
        `Tree store migration failed: ${error.message}`,
        error
      );
    } finally {
      client.release();
    }
  }

  /**
   * Pre-prefix rename. Canary-gated on the legacy `tree_schema_migrations`
   * table.
   * @param {import('pg').PoolClient} client
   */
  async _applySchemaRenames(client) {
    const canary = await client.query(
      `SELECT EXISTS (
         SELECT 1 FROM information_schema.tables
         WHERE table_schema = current_schema()
           AND table_name = 'tree_schema_migrations'
       ) AS exists`
    );
    if (!canary.rows[0].exists) {
      return;
    }

    const tableRenames = [
      ["tree_reservations", "brz_tree_reservations"],
      ["tree_leaves", "brz_tree_leaves"],
      ["tree_spent_leaves", "brz_tree_spent_leaves"],
      ["tree_swap_status", "brz_tree_swap_status"],
    ];
    for (const [oldName, newName] of tableRenames) {
      await client.query(
        `ALTER TABLE IF EXISTS ${oldName} RENAME TO ${newName}`
      );
    }

    const indexRenames = [
      ["idx_tree_leaves_user_available", "brz_idx_tree_leaves_user_available"],
      ["idx_tree_leaves_user_reservation", "brz_idx_tree_leaves_user_reservation"],
      ["idx_tree_leaves_user_added_at", "brz_idx_tree_leaves_user_added_at"],
      // Pre-multi-tenant indexes (dropped by the multi-tenant migration).
      ["idx_tree_leaves_available", "brz_idx_tree_leaves_available"],
      ["idx_tree_leaves_reservation", "brz_idx_tree_leaves_reservation"],
      ["idx_tree_leaves_added_at", "brz_idx_tree_leaves_added_at"],
    ];
    for (const [oldName, newName] of indexRenames) {
      await client.query(
        `ALTER INDEX IF EXISTS ${oldName} RENAME TO ${newName}`
      );
    }

    const constraintRenames = [
      ["brz_tree_reservations", "tree_reservations_pkey", "brz_tree_reservations_pkey"],
      ["brz_tree_leaves", "tree_leaves_pkey", "brz_tree_leaves_pkey"],
      [
        "brz_tree_leaves",
        "tree_leaves_user_id_reservation_id_fkey",
        "brz_tree_leaves_user_id_reservation_id_fkey",
      ],
      // Pre-multi-tenant FK (single-column). Rename so the post-tenant
      // migration's `DROP CONSTRAINT IF EXISTS brz_*_fkey` finds it.
      [
        "brz_tree_leaves",
        "tree_leaves_reservation_id_fkey",
        "brz_tree_leaves_reservation_id_fkey",
      ],
      ["brz_tree_spent_leaves", "tree_spent_leaves_pkey", "brz_tree_spent_leaves_pkey"],
      ["brz_tree_swap_status", "tree_swap_status_pkey", "brz_tree_swap_status_pkey"],
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
      `ALTER TABLE IF EXISTS tree_schema_migrations RENAME TO brz_tree_schema_migrations`
    );
  }

  _log(level, message) {
    if (this.logger && typeof this.logger.log === "function") {
      this.logger.log({ line: message, level });
    } else if (level === "error") {
      console.error(`[TreeStoreMigrationManager] ${message}`);
    }
  }

  /**
   * Migrations matching the Rust PostgresTreeStore schema exactly.
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
        name: "Create tree store tables",
        sql: [
          `CREATE TABLE IF NOT EXISTS brz_tree_reservations (
            id TEXT PRIMARY KEY,
            purpose TEXT NOT NULL,
            pending_change_amount BIGINT NOT NULL DEFAULT 0,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
          )`,

          `CREATE TABLE IF NOT EXISTS brz_tree_leaves (
            id TEXT PRIMARY KEY,
            status TEXT NOT NULL,
            is_missing_from_operators BOOLEAN NOT NULL DEFAULT FALSE,
            reservation_id TEXT REFERENCES brz_tree_reservations(id) ON DELETE SET NULL,
            data JSONB NOT NULL,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            added_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
          )`,

          `CREATE TABLE IF NOT EXISTS brz_tree_spent_leaves (
            leaf_id TEXT PRIMARY KEY,
            spent_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
          )`,

          `CREATE INDEX IF NOT EXISTS brz_idx_tree_leaves_available ON brz_tree_leaves(status, is_missing_from_operators)
            WHERE status = 'Available' AND is_missing_from_operators = FALSE`,

          `CREATE INDEX IF NOT EXISTS brz_idx_tree_leaves_reservation ON brz_tree_leaves(reservation_id)
            WHERE reservation_id IS NOT NULL`,

          `CREATE INDEX IF NOT EXISTS brz_idx_tree_leaves_added_at ON brz_tree_leaves(added_at)`,
        ],
      },
      {
        name: "Add swap status tracking",
        sql: [
          `CREATE TABLE IF NOT EXISTS brz_tree_swap_status (
            id INTEGER PRIMARY KEY DEFAULT 1 CHECK (id = 1),
            last_completed_at TIMESTAMPTZ
          )`,
          `INSERT INTO brz_tree_swap_status (id) VALUES (1) ON CONFLICT DO NOTHING`,
        ],
      },
      {
        // Mirrors Rust migration 3 in spark-postgres/src/tree_store.rs.
        // Adds user_id to every tree-store table, backfills with the connecting
        // tenant's identity, and rewrites primary keys / FKs / indexes to lead
        // with user_id. The composite FK uses NO ACTION (the default) instead
        // of the previous single-column ON DELETE SET NULL — PG-only column-list
        // SET NULL is PG15+, and a whole-row SET NULL would null user_id (NOT
        // NULL). cleanupStaleReservations now releases leaves explicitly.
        name: "Multi-tenant scoping: add user_id and rewrite primary keys",
        sql: [
          // Drop the old single-column FK FIRST, before touching the
          // brz_tree_reservations PK it depends on.
          `ALTER TABLE brz_tree_leaves
             DROP CONSTRAINT IF EXISTS brz_tree_leaves_reservation_id_fkey`,

          // brz_tree_reservations: scope by user_id.
          `ALTER TABLE brz_tree_reservations ADD COLUMN user_id BYTEA`,
          `UPDATE brz_tree_reservations SET user_id = ${idLit}`,
          `ALTER TABLE brz_tree_reservations
             ALTER COLUMN user_id SET NOT NULL,
             DROP CONSTRAINT IF EXISTS brz_tree_reservations_pkey,
             ADD PRIMARY KEY (user_id, id)`,

          // brz_tree_leaves: add user_id, rekey, and re-add the composite FK.
          `ALTER TABLE brz_tree_leaves ADD COLUMN user_id BYTEA`,
          `UPDATE brz_tree_leaves SET user_id = ${idLit}`,
          `ALTER TABLE brz_tree_leaves
             ALTER COLUMN user_id SET NOT NULL,
             DROP CONSTRAINT IF EXISTS brz_tree_leaves_pkey,
             ADD PRIMARY KEY (user_id, id),
             ADD FOREIGN KEY (user_id, reservation_id)
                REFERENCES brz_tree_reservations(user_id, id)`,
          `DROP INDEX IF EXISTS brz_idx_tree_leaves_available`,
          `DROP INDEX IF EXISTS brz_idx_tree_leaves_reservation`,
          `DROP INDEX IF EXISTS brz_idx_tree_leaves_added_at`,
          `CREATE INDEX brz_idx_tree_leaves_user_available
             ON brz_tree_leaves(user_id, status, is_missing_from_operators)
             WHERE status = 'Available' AND is_missing_from_operators = FALSE`,
          `CREATE INDEX brz_idx_tree_leaves_user_reservation
             ON brz_tree_leaves(user_id, reservation_id)
             WHERE reservation_id IS NOT NULL`,
          `CREATE INDEX brz_idx_tree_leaves_user_added_at ON brz_tree_leaves(user_id, added_at)`,

          // brz_tree_spent_leaves: scope by user_id.
          `ALTER TABLE brz_tree_spent_leaves ADD COLUMN user_id BYTEA`,
          `UPDATE brz_tree_spent_leaves SET user_id = ${idLit}`,
          `ALTER TABLE brz_tree_spent_leaves
             ALTER COLUMN user_id SET NOT NULL,
             DROP CONSTRAINT IF EXISTS brz_tree_spent_leaves_pkey,
             ADD PRIMARY KEY (user_id, leaf_id)`,

          // brz_tree_swap_status was a singleton (PK id=1, CHECK id=1). Drop the id
          // column (CASCADE removes both PK and CHECK), then re-key by user_id.
          `ALTER TABLE brz_tree_swap_status DROP COLUMN id CASCADE`,
          `ALTER TABLE brz_tree_swap_status ADD COLUMN user_id BYTEA`,
          `UPDATE brz_tree_swap_status SET user_id = ${idLit}`,
          `ALTER TABLE brz_tree_swap_status
             ALTER COLUMN user_id SET NOT NULL,
             ADD PRIMARY KEY (user_id)`,
        ],
      },
    ];
  }
}

module.exports = { TreeStoreMigrationManager };
