/**
 * Token store migrations for MySQL 8.0+. Mirrors `postgres-token-store/migrations.cjs`.
 */

const { TokenStoreError } = require("./errors.cjs");

const TOKEN_MIGRATIONS_TABLE = "token_schema_migrations";
const MIGRATION_LOCK_NAME = "breez_mysql_token_store_migration_lock";
const MIGRATION_LOCK_TIMEOUT = 60;

class MysqlTokenStoreMigrationManager {
  constructor(logger = null) {
    this.logger = logger;
  }

  async migrate(pool) {
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

        const migrations = this._getMigrations();

        if (currentVersion >= migrations.length) {
          await conn.query("COMMIT");
          return;
        }

        for (let i = currentVersion; i < migrations.length; i++) {
          const migration = migrations[i];
          const version = i + 1;
          for (const sql of migration.sql) {
            await conn.query(sql);
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

  _getMigrations() {
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
          `INSERT IGNORE INTO token_swap_status (id) VALUES (1)`,
        ],
      },
    ];
  }
}

module.exports = { MysqlTokenStoreMigrationManager };
