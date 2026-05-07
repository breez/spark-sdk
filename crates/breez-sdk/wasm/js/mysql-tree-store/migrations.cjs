/**
 * Tree store migrations for MySQL 8.0+. Mirrors `postgres-tree-store/migrations.cjs`.
 */

const { TreeStoreError } = require("./errors.cjs");

const TREE_MIGRATIONS_TABLE = "tree_schema_migrations";
const MIGRATION_LOCK_NAME = "breez_mysql_tree_store_migration_lock";
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

class MysqlTreeStoreMigrationManager {
  constructor(logger = null) {
    this.logger = logger;
  }

  async migrate(pool, identity) {
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

  _getMigrations(identity) {
    const idHex = Buffer.from(identity).toString("hex");
    const idLit = `UNHEX('${idHex}')`;

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
          `INSERT INTO tree_swap_status (id) VALUES (1)
            ON DUPLICATE KEY UPDATE id = id`,
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
      {
        name: "Multi-tenant scoping: add user_id and rewrite primary keys / FKs",
        sql: [
          // Drop the existing FK so we can rewrite the parent PK.
          `ALTER TABLE tree_leaves DROP FOREIGN KEY fk_tree_leaves_reservation`,

          // tree_reservations: scope by user_id.
          `ALTER TABLE tree_reservations ADD COLUMN user_id VARBINARY(33) NULL`,
          `UPDATE tree_reservations SET user_id = ${idLit} WHERE user_id IS NULL`,
          `ALTER TABLE tree_reservations MODIFY COLUMN user_id VARBINARY(33) NOT NULL`,
          `ALTER TABLE tree_reservations DROP PRIMARY KEY, ADD PRIMARY KEY (user_id, id)`,

          // tree_leaves: scope by user_id, rekey, re-add composite FK.
          `ALTER TABLE tree_leaves ADD COLUMN user_id VARBINARY(33) NULL`,
          `UPDATE tree_leaves SET user_id = ${idLit} WHERE user_id IS NULL`,
          `ALTER TABLE tree_leaves MODIFY COLUMN user_id VARBINARY(33) NOT NULL`,
          `ALTER TABLE tree_leaves DROP PRIMARY KEY, ADD PRIMARY KEY (user_id, id)`,
          `ALTER TABLE tree_leaves
             ADD CONSTRAINT fk_tree_leaves_reservation_user
             FOREIGN KEY (user_id, reservation_id)
             REFERENCES tree_reservations(user_id, id)`,
          `DROP INDEX idx_tree_leaves_available ON tree_leaves`,
          `DROP INDEX idx_tree_leaves_reservation ON tree_leaves`,
          `DROP INDEX idx_tree_leaves_added_at ON tree_leaves`,
          `DROP INDEX idx_tree_leaves_slim ON tree_leaves`,
          `CREATE INDEX idx_tree_leaves_user_available
             ON tree_leaves(user_id, status, is_missing_from_operators)`,
          `CREATE INDEX idx_tree_leaves_user_reservation
             ON tree_leaves(user_id, reservation_id)`,
          `CREATE INDEX idx_tree_leaves_user_added_at ON tree_leaves(user_id, added_at)`,
          `CREATE INDEX idx_tree_leaves_user_slim
             ON tree_leaves(user_id, status, is_missing_from_operators, reservation_id, value)`,

          // tree_spent_leaves: scope by user_id.
          `ALTER TABLE tree_spent_leaves ADD COLUMN user_id VARBINARY(33) NULL`,
          `UPDATE tree_spent_leaves SET user_id = ${idLit} WHERE user_id IS NULL`,
          `ALTER TABLE tree_spent_leaves MODIFY COLUMN user_id VARBINARY(33) NOT NULL`,
          `ALTER TABLE tree_spent_leaves DROP PRIMARY KEY, ADD PRIMARY KEY (user_id, leaf_id)`,

          // tree_swap_status was a singleton (PK id=1, CHECK id=1). Drop the PK
          // and the id column, then re-key by user_id.
          { op: "dropPrimaryKey", table: "tree_swap_status" },
          `ALTER TABLE tree_swap_status DROP COLUMN id`,
          `ALTER TABLE tree_swap_status ADD COLUMN user_id VARBINARY(33) NULL`,
          `UPDATE tree_swap_status SET user_id = ${idLit} WHERE user_id IS NULL`,
          `ALTER TABLE tree_swap_status MODIFY COLUMN user_id VARBINARY(33) NOT NULL`,
          `ALTER TABLE tree_swap_status ADD PRIMARY KEY (user_id)`,
        ],
      },
    ];
  }
}

module.exports = { MysqlTreeStoreMigrationManager };
