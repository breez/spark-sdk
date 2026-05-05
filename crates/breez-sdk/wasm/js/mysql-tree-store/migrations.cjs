/**
 * Tree store migrations for MySQL 8.0+. Mirrors `postgres-tree-store/migrations.cjs`.
 */

const { TreeStoreError } = require("./errors.cjs");

const TREE_MIGRATIONS_TABLE = "tree_schema_migrations";
const MIGRATION_LOCK_NAME = "breez_mysql_tree_store_migration_lock";
const MIGRATION_LOCK_TIMEOUT = 60;

class MysqlTreeStoreMigrationManager {
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
        throw new TreeStoreError(
          `Failed to acquire tree store migration lock within ${MIGRATION_LOCK_TIMEOUT}s`
        );
      }

      try {
        await conn.query("START TRANSACTION");

        await conn.query(`
          CREATE TABLE IF NOT EXISTS \`${TREE_MIGRATIONS_TABLE}\` (
            version INT PRIMARY KEY,
            applied_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6)
          )
        `);

        const [versionRows] = await conn.query(
          `SELECT COALESCE(MAX(version), 0) AS version FROM \`${TREE_MIGRATIONS_TABLE}\``
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
            `INSERT INTO \`${TREE_MIGRATIONS_TABLE}\` (version) VALUES (?)`,
            [version]
          );
        }

        await conn.query("COMMIT");
      } catch (error) {
        await conn.query("ROLLBACK").catch(() => {});
        throw new TreeStoreError(
          `Tree store migration failed: ${error.message}`,
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
        name: "Create tree store tables",
        sql: [
          `CREATE TABLE IF NOT EXISTS tree_reservations (
            id VARCHAR(255) NOT NULL PRIMARY KEY,
            purpose VARCHAR(64) NOT NULL,
            pending_change_amount BIGINT NOT NULL DEFAULT 0,
            created_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6)
          )`,
          `CREATE TABLE IF NOT EXISTS tree_leaves (
            id VARCHAR(255) NOT NULL PRIMARY KEY,
            status VARCHAR(64) NOT NULL,
            is_missing_from_operators TINYINT(1) NOT NULL DEFAULT 0,
            reservation_id VARCHAR(255) NULL,
            data JSON NOT NULL,
            created_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6),
            added_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6),
            CONSTRAINT fk_tree_leaves_reservation FOREIGN KEY (reservation_id)
                REFERENCES tree_reservations(id) ON DELETE SET NULL
          )`,
          `CREATE TABLE IF NOT EXISTS tree_spent_leaves (
            leaf_id VARCHAR(255) NOT NULL PRIMARY KEY,
            spent_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6)
          )`,
          `CREATE INDEX idx_tree_leaves_available
            ON tree_leaves(status, is_missing_from_operators)`,
          `CREATE INDEX idx_tree_leaves_reservation ON tree_leaves(reservation_id)`,
          `CREATE INDEX idx_tree_leaves_added_at ON tree_leaves(added_at)`,
        ],
      },
      {
        name: "Add swap status tracking",
        sql: [
          `CREATE TABLE IF NOT EXISTS tree_swap_status (
            id INT NOT NULL PRIMARY KEY DEFAULT 1,
            last_completed_at DATETIME(6) NULL,
            CHECK (id = 1)
          )`,
          `INSERT IGNORE INTO tree_swap_status (id) VALUES (1)`,
        ],
      },
      {
        name: "Promote leaf value to BIGINT column with covering index",
        sql: [
          `ALTER TABLE tree_leaves
            ADD COLUMN value BIGINT NOT NULL DEFAULT 0`,
          `UPDATE tree_leaves
            SET value = CAST(JSON_UNQUOTE(JSON_EXTRACT(data, '$.value')) AS UNSIGNED)
            WHERE value = 0`,
          `CREATE INDEX idx_tree_leaves_slim
            ON tree_leaves(status, is_missing_from_operators, reservation_id, value)`,
        ],
      },
    ];
  }
}

module.exports = { MysqlTreeStoreMigrationManager };
