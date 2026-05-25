/**
 * Session store migrations for MySQL 8.0+. Mirrors the Rust
 * `MysqlSessionStore` schema exactly.
 */

const { SessionStoreError } = require("./errors.cjs");

const SESSION_MIGRATIONS_TABLE = "brz_session_schema_migrations";
const MIGRATION_LOCK_NAME = "breez_mysql_session_manager_migration_lock";
const MIGRATION_LOCK_TIMEOUT = 60;

class MysqlSessionStoreMigrationManager {
  constructor(logger = null) {
    this.logger = logger;
  }

  /**
   * @param {import('mysql2/promise').Pool} pool
   */
  async migrate(pool) {
    const conn = await pool.getConnection();
    try {
      const [lockRows] = await conn.query(
        "SELECT GET_LOCK(?, ?) AS acquired",
        [MIGRATION_LOCK_NAME, MIGRATION_LOCK_TIMEOUT]
      );
      if (!lockRows || lockRows[0].acquired !== 1) {
        throw new SessionStoreError(
          `Failed to acquire session store migration lock within ${MIGRATION_LOCK_TIMEOUT}s`
        );
      }

      try {
        await this._applySchemaRenames(conn);

        await conn.query("START TRANSACTION");

        await conn.query(`
          CREATE TABLE IF NOT EXISTS \`${SESSION_MIGRATIONS_TABLE}\` (
            version INT PRIMARY KEY,
            applied_at DATETIME(6) NOT NULL DEFAULT (UTC_TIMESTAMP(6))
          )
        `);

        const [versionRows] = await conn.query(
          `SELECT COALESCE(MAX(version), 0) AS version FROM \`${SESSION_MIGRATIONS_TABLE}\``
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
            `INSERT INTO \`${SESSION_MIGRATIONS_TABLE}\` (version, applied_at) VALUES (?, UTC_TIMESTAMP(6))`,
            [version]
          );
        }

        await conn.query("COMMIT");
      } catch (error) {
        await conn.query("ROLLBACK").catch(() => {});
        throw new SessionStoreError(
          `Session store migration failed: ${error.message}`,
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
   * Pre-prefix rename. Canary-gated on the legacy `session_schema_migrations`
   * table.
   * @param {import('mysql2/promise').PoolConnection} conn
   */
  async _applySchemaRenames(conn) {
    if (!(await _mysqlTableExists(conn, "session_schema_migrations"))) {
      return;
    }

    if (
      (await _mysqlTableExists(conn, "sessions")) &&
      !(await _mysqlTableExists(conn, "brz_sessions"))
    ) {
      await conn.query("RENAME TABLE `sessions` TO `brz_sessions`");
    }

    if (await _mysqlTableExists(conn, "session_schema_migrations")) {
      if (!(await _mysqlTableExists(conn, SESSION_MIGRATIONS_TABLE))) {
        await conn.query(
          `RENAME TABLE \`session_schema_migrations\` TO \`${SESSION_MIGRATIONS_TABLE}\``
        );
      }
    }
  }

  _getMigrations() {
    return [
      {
        name: "Create brz_sessions table",
        sql: [
          `CREATE TABLE IF NOT EXISTS brz_sessions (
            user_id VARBINARY(33) NOT NULL,
            service_identity_key VARBINARY(33) NOT NULL,
            token TEXT NOT NULL,
            expiration BIGINT NOT NULL,
            PRIMARY KEY (user_id, service_identity_key)
          )`,
        ],
      },
      {
        name: "Pin schema-migrations applied_at default to UTC",
        sql: [
          `ALTER TABLE \`${SESSION_MIGRATIONS_TABLE}\` MODIFY COLUMN applied_at DATETIME(6) NOT NULL DEFAULT (UTC_TIMESTAMP(6))`,
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

module.exports = { MysqlSessionStoreMigrationManager };
