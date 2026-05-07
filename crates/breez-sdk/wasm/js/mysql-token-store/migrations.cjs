/**
 * Token store migrations for MySQL 8.0+. Mirrors `postgres-token-store/migrations.cjs`.
 */

const { TokenStoreError } = require("./errors.cjs");

const TOKEN_MIGRATIONS_TABLE = "token_schema_migrations";
const MIGRATION_LOCK_NAME = "breez_mysql_token_store_migration_lock";
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

class MysqlTokenStoreMigrationManager {
  constructor(logger = null) {
    this.logger = logger;
  }

  /**
   * @param {import('mysql2/promise').Pool} pool
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
    const conn = await pool.getConnection();
    try {
      const [lockRows] = await conn.query(
        "SELECT GET_LOCK(?, ?) AS acquired",
        [MIGRATION_LOCK_NAME, MIGRATION_LOCK_TIMEOUT]
      );
      if (!lockRows || lockRows[0].acquired !== 1) {
        throw new TokenStoreError(
          `Failed to acquire token store migration lock within ${MIGRATION_LOCK_TIMEOUT}s`
        );
      }

      try {
        await conn.query("START TRANSACTION");

        await conn.query(`
          CREATE TABLE IF NOT EXISTS \`${TOKEN_MIGRATIONS_TABLE}\` (
            version INT PRIMARY KEY,
            applied_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6)
          )
        `);

        const [versionRows] = await conn.query(
          `SELECT COALESCE(MAX(version), 0) AS version FROM \`${TOKEN_MIGRATIONS_TABLE}\``
        );
        const currentVersion = versionRows[0].version;

        const migrations = this._getMigrations(identity);

        if (currentVersion >= migrations.length) {
          await conn.query("COMMIT");
          return;
        }

        for (let i = currentVersion; i < migrations.length; i++) {
          const migration = migrations[i];
          const version = i + 1;
          for (const step of migration.sql) {
            await runMigrationStep(conn, step);
          }
          await conn.query(
            `INSERT INTO \`${TOKEN_MIGRATIONS_TABLE}\` (version) VALUES (?)`,
            [version]
          );
        }

        await conn.query("COMMIT");
      } catch (error) {
        await conn.query("ROLLBACK").catch(() => {});
        throw new TokenStoreError(
          `Token store migration failed: ${error.message}`,
          error
        );
      } finally {
        await conn
          .query("SELECT RELEASE_LOCK(?)", [MIGRATION_LOCK_NAME])
          .catch(() => {});
      }
    } finally {
      conn.release();
    }
  }

  /**
   * @param {Buffer|Uint8Array} identity - tenant identity inlined as a hex
   *   `UNHEX('…')` literal in the multi-tenant scoping migration. Safe because
   *   the bytes come from a typed secp256k1 pubkey (`[0-9a-f]{66}` after hex
   *   encoding) — not user-controlled input.
   */
  _getMigrations(identity) {
    const idHex = Buffer.from(identity).toString("hex");
    const idLit = `UNHEX('${idHex}')`;

    return [
      {
        name: "Create token store tables",
        sql: [
          `CREATE TABLE IF NOT EXISTS token_metadata (
            identifier VARCHAR(255) NOT NULL PRIMARY KEY,
            issuer_public_key VARCHAR(255) NOT NULL,
            name VARCHAR(255) NOT NULL,
            ticker VARCHAR(64) NOT NULL,
            decimals INT NOT NULL,
            max_supply VARCHAR(128) NOT NULL,
            is_freezable TINYINT(1) NOT NULL,
            creation_entity_public_key VARCHAR(255) NULL
          )`,
          `CREATE INDEX idx_token_metadata_issuer_pk
            ON token_metadata (issuer_public_key)`,
          `CREATE TABLE IF NOT EXISTS token_reservations (
            id VARCHAR(255) NOT NULL PRIMARY KEY,
            purpose VARCHAR(64) NOT NULL,
            created_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6)
          )`,
          `CREATE TABLE IF NOT EXISTS token_outputs (
            id VARCHAR(255) NOT NULL PRIMARY KEY,
            token_identifier VARCHAR(255) NOT NULL,
            owner_public_key VARCHAR(255) NOT NULL,
            revocation_commitment VARCHAR(255) NOT NULL,
            withdraw_bond_sats BIGINT NOT NULL,
            withdraw_relative_block_locktime BIGINT NOT NULL,
            token_public_key VARCHAR(255) NULL,
            token_amount VARCHAR(128) NOT NULL,
            prev_tx_hash VARCHAR(255) NOT NULL,
            prev_tx_vout INT NOT NULL,
            reservation_id VARCHAR(255) NULL,
            added_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6),
            CONSTRAINT fk_token_outputs_metadata FOREIGN KEY (token_identifier)
                REFERENCES token_metadata(identifier),
            CONSTRAINT fk_token_outputs_reservation FOREIGN KEY (reservation_id)
                REFERENCES token_reservations(id) ON DELETE SET NULL
          )`,
          `CREATE INDEX idx_token_outputs_identifier
            ON token_outputs (token_identifier)`,
          `CREATE INDEX idx_token_outputs_reservation
            ON token_outputs (reservation_id)`,
          `CREATE TABLE IF NOT EXISTS token_spent_outputs (
            output_id VARCHAR(255) NOT NULL PRIMARY KEY,
            spent_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6)
          )`,
          `CREATE TABLE IF NOT EXISTS token_swap_status (
            id INT NOT NULL PRIMARY KEY DEFAULT 1,
            last_completed_at DATETIME(6) NULL,
            CHECK (id = 1)
          )`,
          `INSERT INTO token_swap_status (id) VALUES (1)
            ON DUPLICATE KEY UPDATE id = id`,
        ],
      },
      {
        // Mirrors Rust migration 2 in spark-mysql/src/token_store.rs and the
        // postgres equivalent. Adds user_id to every token-store table
        // (including token_metadata — per-tenant to avoid 0-balance leakage
        // for tokens a tenant never owned), backfills with the connecting
        // tenant, and rewrites primary keys / FKs / indexes to lead with
        // user_id. Composite FKs use NO ACTION because column-list SET NULL
        // would null user_id (NOT NULL).
        name: "Multi-tenant scoping: add user_id and rewrite primary keys / FKs",
        sql: [
          // Drop dependent FKs FIRST so we can rewrite the parent PKs.
          `ALTER TABLE token_outputs DROP FOREIGN KEY fk_token_outputs_metadata`,
          `ALTER TABLE token_outputs DROP FOREIGN KEY fk_token_outputs_reservation`,

          // token_metadata: per-tenant scoping (privacy — see header).
          `ALTER TABLE token_metadata ADD COLUMN user_id VARBINARY(33) NULL`,
          `UPDATE token_metadata SET user_id = ${idLit} WHERE user_id IS NULL`,
          `ALTER TABLE token_metadata MODIFY COLUMN user_id VARBINARY(33) NOT NULL`,
          `ALTER TABLE token_metadata DROP PRIMARY KEY, ADD PRIMARY KEY (user_id, identifier)`,
          `DROP INDEX idx_token_metadata_issuer_pk ON token_metadata`,
          `CREATE INDEX idx_token_metadata_user_issuer_pk
             ON token_metadata (user_id, issuer_public_key)`,

          // token_reservations: scope by user_id.
          `ALTER TABLE token_reservations ADD COLUMN user_id VARBINARY(33) NULL`,
          `UPDATE token_reservations SET user_id = ${idLit} WHERE user_id IS NULL`,
          `ALTER TABLE token_reservations MODIFY COLUMN user_id VARBINARY(33) NOT NULL`,
          `ALTER TABLE token_reservations DROP PRIMARY KEY, ADD PRIMARY KEY (user_id, id)`,

          // token_outputs: scope by user_id, rekey, re-add composite FKs.
          `ALTER TABLE token_outputs ADD COLUMN user_id VARBINARY(33) NULL`,
          `UPDATE token_outputs SET user_id = ${idLit} WHERE user_id IS NULL`,
          `ALTER TABLE token_outputs MODIFY COLUMN user_id VARBINARY(33) NOT NULL`,
          `ALTER TABLE token_outputs DROP PRIMARY KEY, ADD PRIMARY KEY (user_id, id)`,
          `ALTER TABLE token_outputs
             ADD CONSTRAINT fk_token_outputs_metadata_user
             FOREIGN KEY (user_id, token_identifier)
             REFERENCES token_metadata(user_id, identifier)`,
          `ALTER TABLE token_outputs
             ADD CONSTRAINT fk_token_outputs_reservation_user
             FOREIGN KEY (user_id, reservation_id)
             REFERENCES token_reservations(user_id, id)`,
          `DROP INDEX idx_token_outputs_identifier ON token_outputs`,
          `DROP INDEX idx_token_outputs_reservation ON token_outputs`,
          `CREATE INDEX idx_token_outputs_user_identifier
             ON token_outputs (user_id, token_identifier)`,
          `CREATE INDEX idx_token_outputs_user_reservation
             ON token_outputs (user_id, reservation_id)`,

          // token_spent_outputs: scope by user_id.
          `ALTER TABLE token_spent_outputs ADD COLUMN user_id VARBINARY(33) NULL`,
          `UPDATE token_spent_outputs SET user_id = ${idLit} WHERE user_id IS NULL`,
          `ALTER TABLE token_spent_outputs MODIFY COLUMN user_id VARBINARY(33) NOT NULL`,
          `ALTER TABLE token_spent_outputs DROP PRIMARY KEY, ADD PRIMARY KEY (user_id, output_id)`,

          // token_swap_status was a singleton (PK id=1, CHECK id=1). Drop the
          // PK and the id column, then re-key by user_id.
          { op: "dropPrimaryKey", table: "token_swap_status" },
          `ALTER TABLE token_swap_status DROP COLUMN id`,
          `ALTER TABLE token_swap_status ADD COLUMN user_id VARBINARY(33) NULL`,
          `UPDATE token_swap_status SET user_id = ${idLit} WHERE user_id IS NULL`,
          `ALTER TABLE token_swap_status MODIFY COLUMN user_id VARBINARY(33) NOT NULL`,
          `ALTER TABLE token_swap_status ADD PRIMARY KEY (user_id)`,
        ],
      },
    ];
  }
}

module.exports = { MysqlTokenStoreMigrationManager };
