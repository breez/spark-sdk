/**
 * Tree store migrations for MySQL 8.0+. Mirrors `postgres-tree-store/migrations.cjs`.
 */

const { TreeStoreError } = require("./errors.cjs");

const TREE_MIGRATIONS_TABLE = "brz_tree_schema_migrations";
const MIGRATION_LOCK_NAME = "breez_mysql_tree_store_migration_lock";
const MIGRATION_LOCK_TIMEOUT = 60;

/**
 * Runs a single migration step. Plain strings are run as-is; tagged objects
 * (`{ op: 'dropPrimaryKey', table }`, `{ op: 'dropForeignKey', table, name }`)
 * are guarded against partial-apply replay (and against the `Disabled`
 * foreign-key mode where the FK was never created) by checking
 * `information_schema` first. MySQL DDL implicitly commits, so if the
 * migration crashes between two DDL statements the version row never gets
 * recorded — and on retry, an unguarded DROP would fail because the
 * constraint is already gone.
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
  if (step.op === "dropForeignKey") {
    const [rows] = await conn.query(
      `SELECT COUNT(*) AS c FROM information_schema.table_constraints
       WHERE table_schema = DATABASE()
         AND table_name = ?
         AND constraint_type = 'FOREIGN KEY'
         AND constraint_name = ?`,
      [step.table, step.name]
    );
    if (rows[0].c > 0) {
      await conn.query(
        `ALTER TABLE \`${step.table}\` DROP FOREIGN KEY \`${step.name}\``
      );
    }
    return;
  }
  if (step.op === "addForeignKey") {
    const [rows] = await conn.query(
      `SELECT COUNT(*) AS c FROM information_schema.table_constraints
       WHERE table_schema = DATABASE()
         AND table_name = ?
         AND constraint_type = 'FOREIGN KEY'
         AND constraint_name = ?`,
      [step.table, step.name]
    );
    if (rows[0].c === 0) {
      await conn.query(
        `ALTER TABLE \`${step.table}\` ADD CONSTRAINT \`${step.name}\` ${step.definition}`
      );
    }
    return;
  }
  throw new Error(`Unknown migration step op: ${JSON.stringify(step)}`);
}

class MysqlTreeStoreMigrationManager {
  constructor(logger = null, foreignKeyMode = "Enforced") {
    this.logger = logger;
    this.foreignKeyMode = foreignKeyMode;
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
        await this._applySchemaRenames(conn);

        await conn.query("START TRANSACTION");

        await conn.query(`
          CREATE TABLE IF NOT EXISTS \`${TREE_MIGRATIONS_TABLE}\` (
            version INT PRIMARY KEY,
            applied_at DATETIME(6) NOT NULL DEFAULT (UTC_TIMESTAMP(6))
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
            `INSERT INTO \`${TREE_MIGRATIONS_TABLE}\` (version, applied_at) VALUES (?, UTC_TIMESTAMP(6))`,
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

  /**
   * Pre-prefix rename. Canary-gated on the legacy `tree_schema_migrations`
   * table.
   * @param {import('mysql2/promise').PoolConnection} conn
   */
  async _applySchemaRenames(conn) {
    if (!(await _mysqlTableExists(conn, "tree_schema_migrations"))) {
      return;
    }

    const tableRenames = [
      ["tree_reservations", "brz_tree_reservations"],
      ["tree_leaves", "brz_tree_leaves"],
      ["tree_spent_leaves", "brz_tree_spent_leaves"],
      ["tree_swap_status", "brz_tree_swap_status"],
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
      ["brz_tree_leaves", "idx_tree_leaves_user_available", "brz_idx_tree_leaves_user_available"],
      [
        "brz_tree_leaves",
        "idx_tree_leaves_user_reservation",
        "brz_idx_tree_leaves_user_reservation",
      ],
      ["brz_tree_leaves", "idx_tree_leaves_user_added_at", "brz_idx_tree_leaves_user_added_at"],
      ["brz_tree_leaves", "idx_tree_leaves_user_slim", "brz_idx_tree_leaves_user_slim"],
      // Pre-multi-tenant indexes (dropped by the multi-tenant migration).
      ["brz_tree_leaves", "idx_tree_leaves_available", "brz_idx_tree_leaves_available"],
      ["brz_tree_leaves", "idx_tree_leaves_reservation", "brz_idx_tree_leaves_reservation"],
      ["brz_tree_leaves", "idx_tree_leaves_added_at", "brz_idx_tree_leaves_added_at"],
      ["brz_tree_leaves", "idx_tree_leaves_slim", "brz_idx_tree_leaves_slim"],
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

    // MySQL has no RENAME CONSTRAINT for foreign keys: drop the legacy FK
    // and re-add it under the brz_ name.
    const fkRenames = [
      {
        table: "brz_tree_leaves",
        oldName: "fk_tree_leaves_reservation_user",
        newName: "brz_fk_tree_leaves_reservation_user",
        definition:
          "FOREIGN KEY (user_id, reservation_id) REFERENCES `brz_tree_reservations`(user_id, id)",
      },
      // Pre-multi-tenant FK (single-column). Rename so the post-tenant
      // migration's drop-foreign-key step finds it.
      {
        table: "brz_tree_leaves",
        oldName: "fk_tree_leaves_reservation",
        newName: "brz_fk_tree_leaves_reservation",
        definition:
          "FOREIGN KEY (reservation_id) REFERENCES `brz_tree_reservations`(id) ON DELETE SET NULL",
      },
    ];
    for (const fk of fkRenames) {
      if (await _mysqlForeignKeyExists(conn, fk.table, fk.newName)) {
        continue;
      }
      if (!(await _mysqlForeignKeyExists(conn, fk.table, fk.oldName))) {
        continue;
      }
      await conn.query(
        `ALTER TABLE \`${fk.table}\`` +
          ` DROP FOREIGN KEY \`${fk.oldName}\`,` +
          ` ADD CONSTRAINT \`${fk.newName}\` ${fk.definition}`
      );
    }

    if (
      (await _mysqlTableExists(conn, "tree_schema_migrations")) &&
      !(await _mysqlTableExists(conn, TREE_MIGRATIONS_TABLE))
    ) {
      await conn.query(
        `RENAME TABLE \`tree_schema_migrations\` TO \`${TREE_MIGRATIONS_TABLE}\``
      );
    }
  }

  _getMigrations(identity) {
    const idHex = Buffer.from(identity).toString("hex");
    const idLit = `UNHEX('${idHex}')`;
    const foreignKeyModeEnforced = this.foreignKeyMode === "Enforced";

    const initialSql = [
          `CREATE TABLE IF NOT EXISTS brz_tree_reservations (
            id VARCHAR(255) NOT NULL PRIMARY KEY,
            purpose VARCHAR(64) NOT NULL,
            pending_change_amount BIGINT NOT NULL DEFAULT 0,
            created_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6)
          )`,
          `CREATE TABLE IF NOT EXISTS brz_tree_leaves (
            id VARCHAR(255) NOT NULL PRIMARY KEY,
            status VARCHAR(64) NOT NULL,
            is_missing_from_operators TINYINT(1) NOT NULL DEFAULT 0,
            reservation_id VARCHAR(255) NULL,
            data JSON NOT NULL,
            created_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6),
            added_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6)
          )`,
          `CREATE TABLE IF NOT EXISTS brz_tree_spent_leaves (
            leaf_id VARCHAR(255) NOT NULL PRIMARY KEY,
            spent_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6)
          )`,
          `CREATE INDEX brz_idx_tree_leaves_available
            ON brz_tree_leaves(status, is_missing_from_operators)`,
          `CREATE INDEX brz_idx_tree_leaves_reservation ON brz_tree_leaves(reservation_id)`,
          `CREATE INDEX brz_idx_tree_leaves_added_at ON brz_tree_leaves(added_at)`,
    ];
    if (foreignKeyModeEnforced) {
      initialSql.push({
        op: "addForeignKey",
        table: "brz_tree_leaves",
        name: "brz_fk_tree_leaves_reservation",
        definition: `FOREIGN KEY (reservation_id) REFERENCES brz_tree_reservations(id) ON DELETE SET NULL`,
      });
    }

    return [
      {
        name: "Create tree store tables",
        sql: initialSql,
      },
      {
        name: "Add swap status tracking",
        sql: [
          `CREATE TABLE IF NOT EXISTS brz_tree_swap_status (
            id INT NOT NULL PRIMARY KEY DEFAULT 1,
            last_completed_at DATETIME(6) NULL,
            CHECK (id = 1)
          )`,
          `INSERT INTO brz_tree_swap_status (id) VALUES (1)
            ON DUPLICATE KEY UPDATE id = id`,
        ],
      },
      {
        name: "Promote leaf value to BIGINT column with covering index",
        sql: [
          `ALTER TABLE brz_tree_leaves
            ADD COLUMN value BIGINT NOT NULL DEFAULT 0`,
          `UPDATE brz_tree_leaves
            SET value = CAST(JSON_UNQUOTE(JSON_EXTRACT(data, '$.value')) AS UNSIGNED)
            WHERE value = 0`,
          `CREATE INDEX brz_idx_tree_leaves_slim
            ON brz_tree_leaves(status, is_missing_from_operators, reservation_id, value)`,
        ],
      },
      {
        name: "Multi-tenant scoping: add user_id and rewrite primary keys / FKs",
        sql: [
          // Drop the existing FK so we can rewrite the parent PK. Guarded so
          // that databases created with `Disabled` foreign-key mode (where the
          // FK was never created) skip the DROP rather than erroring.
          {
            op: "dropForeignKey",
            table: "brz_tree_leaves",
            name: "brz_fk_tree_leaves_reservation",
          },

          // brz_tree_reservations: scope by user_id.
          `ALTER TABLE brz_tree_reservations ADD COLUMN user_id VARBINARY(33) NULL`,
          `UPDATE brz_tree_reservations SET user_id = ${idLit} WHERE user_id IS NULL`,
          `ALTER TABLE brz_tree_reservations MODIFY COLUMN user_id VARBINARY(33) NOT NULL`,
          `ALTER TABLE brz_tree_reservations DROP PRIMARY KEY, ADD PRIMARY KEY (user_id, id)`,

          // brz_tree_leaves: scope by user_id, rekey, optionally re-add composite FK.
          `ALTER TABLE brz_tree_leaves ADD COLUMN user_id VARBINARY(33) NULL`,
          `UPDATE brz_tree_leaves SET user_id = ${idLit} WHERE user_id IS NULL`,
          `ALTER TABLE brz_tree_leaves MODIFY COLUMN user_id VARBINARY(33) NOT NULL`,
          `ALTER TABLE brz_tree_leaves DROP PRIMARY KEY, ADD PRIMARY KEY (user_id, id)`,
          ...(foreignKeyModeEnforced
            ? [
                {
                  op: "addForeignKey",
                  table: "brz_tree_leaves",
                  name: "brz_fk_tree_leaves_reservation_user",
                  definition: `FOREIGN KEY (user_id, reservation_id) REFERENCES brz_tree_reservations(user_id, id)`,
                },
              ]
            : []),
          `DROP INDEX brz_idx_tree_leaves_available ON brz_tree_leaves`,
          `DROP INDEX brz_idx_tree_leaves_reservation ON brz_tree_leaves`,
          `DROP INDEX brz_idx_tree_leaves_added_at ON brz_tree_leaves`,
          `DROP INDEX brz_idx_tree_leaves_slim ON brz_tree_leaves`,
          `CREATE INDEX brz_idx_tree_leaves_user_available
             ON brz_tree_leaves(user_id, status, is_missing_from_operators)`,
          `CREATE INDEX brz_idx_tree_leaves_user_reservation
             ON brz_tree_leaves(user_id, reservation_id)`,
          `CREATE INDEX brz_idx_tree_leaves_user_added_at ON brz_tree_leaves(user_id, added_at)`,
          `CREATE INDEX brz_idx_tree_leaves_user_slim
             ON brz_tree_leaves(user_id, status, is_missing_from_operators, reservation_id, value)`,

          // brz_tree_spent_leaves: scope by user_id.
          `ALTER TABLE brz_tree_spent_leaves ADD COLUMN user_id VARBINARY(33) NULL`,
          `UPDATE brz_tree_spent_leaves SET user_id = ${idLit} WHERE user_id IS NULL`,
          `ALTER TABLE brz_tree_spent_leaves MODIFY COLUMN user_id VARBINARY(33) NOT NULL`,
          `ALTER TABLE brz_tree_spent_leaves DROP PRIMARY KEY, ADD PRIMARY KEY (user_id, leaf_id)`,

          // brz_tree_swap_status was a singleton (PK id=1, CHECK id=1). Drop the PK
          // and the id column, then re-key by user_id.
          { op: "dropPrimaryKey", table: "brz_tree_swap_status" },
          `ALTER TABLE brz_tree_swap_status DROP COLUMN id`,
          `ALTER TABLE brz_tree_swap_status ADD COLUMN user_id VARBINARY(33) NULL`,
          `UPDATE brz_tree_swap_status SET user_id = ${idLit} WHERE user_id IS NULL`,
          `ALTER TABLE brz_tree_swap_status MODIFY COLUMN user_id VARBINARY(33) NOT NULL`,
          `ALTER TABLE brz_tree_swap_status ADD PRIMARY KEY (user_id)`,
        ],
      },
      {
        // Pin DATETIME defaults to UTC. Server-side INSERTs already pass
        // `UTC_TIMESTAMP(6)` explicitly; this migration makes the column
        // default match, so any future callsite that omits the column also
        // gets a UTC value rather than a session-TZ-dependent one.
        name: "Pin DATETIME defaults to UTC",
        sql: [
          `ALTER TABLE brz_tree_reservations      MODIFY COLUMN created_at DATETIME(6) NOT NULL DEFAULT (UTC_TIMESTAMP(6))`,
          `ALTER TABLE brz_tree_leaves            MODIFY COLUMN created_at DATETIME(6) NOT NULL DEFAULT (UTC_TIMESTAMP(6))`,
          `ALTER TABLE brz_tree_leaves            MODIFY COLUMN added_at   DATETIME(6) NOT NULL DEFAULT (UTC_TIMESTAMP(6))`,
          `ALTER TABLE brz_tree_spent_leaves      MODIFY COLUMN spent_at   DATETIME(6) NOT NULL DEFAULT (UTC_TIMESTAMP(6))`,
          `ALTER TABLE brz_tree_schema_migrations MODIFY COLUMN applied_at DATETIME(6) NOT NULL DEFAULT (UTC_TIMESTAMP(6))`,
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

async function _mysqlForeignKeyExists(conn, tableName, constraintName) {
  const [rows] = await conn.query(
    `SELECT COUNT(*) AS c FROM information_schema.table_constraints
     WHERE table_schema = DATABASE()
       AND table_name = ?
       AND constraint_type = 'FOREIGN KEY'
       AND constraint_name = ?`,
    [tableName, constraintName]
  );
  return Number(rows[0].c) > 0;
}

module.exports = { MysqlTreeStoreMigrationManager };
