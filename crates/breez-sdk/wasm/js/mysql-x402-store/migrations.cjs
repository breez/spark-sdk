/**
 * Migrations for the x402 replay store in MySQL. Mirrors the Rust
 * `MysqlX402ReplayStore` schema.
 */

const { X402StoreError } = require("./errors.cjs");

const MIGRATIONS_TABLE = "brz_x402_schema_migrations";
// The lock the Rust migration runner takes, so Rust and JS servers sharing a
// database migrate in turn.
const MIGRATION_LOCK_NAME = `migration_lock_${MIGRATIONS_TABLE}`;
const MIGRATION_LOCK_TIMEOUT_SECS = 60;
const MIGRATIONS = [
  [
    `CREATE TABLE IF NOT EXISTS brz_x402_replay (
       user_id VARBINARY(33) NOT NULL,
       replay_key VARCHAR(255) NOT NULL,
       retain_until BIGINT NOT NULL,
       PRIMARY KEY (user_id, replay_key),
       KEY brz_idx_x402_replay_retain (user_id, retain_until)
     )`,
  ],
];

class X402StoreMigrationManager {
  /** @param {import('mysql2/promise').Pool} pool */
  async migrate(pool) {
    const conn = await pool.getConnection();
    try {
      const [lockRows] = await conn.query("SELECT GET_LOCK(?, ?) AS acquired", [
        MIGRATION_LOCK_NAME,
        MIGRATION_LOCK_TIMEOUT_SECS,
      ]);
      if (Number(lockRows[0].acquired) !== 1) {
        throw new Error(
          `failed to acquire the migration lock within ${MIGRATION_LOCK_TIMEOUT_SECS}s`
        );
      }
      try {
        await conn.query(
          `CREATE TABLE IF NOT EXISTS \`${MIGRATIONS_TABLE}\` (
             version INT PRIMARY KEY,
             applied_at DATETIME(6) NOT NULL DEFAULT (UTC_TIMESTAMP(6))
           )`
        );
        const [rows] = await conn.query(
          `SELECT COALESCE(MAX(version), 0) AS version FROM \`${MIGRATIONS_TABLE}\``
        );
        for (let v = Number(rows[0].version); v < MIGRATIONS.length; v++) {
          for (const sql of MIGRATIONS[v]) {
            await conn.query(sql);
          }
          await conn.query(
            `INSERT INTO \`${MIGRATIONS_TABLE}\` (version, applied_at) VALUES (?, UTC_TIMESTAMP(6))`,
            [v + 1]
          );
        }
      } finally {
        await conn.query("SELECT RELEASE_LOCK(?)", [MIGRATION_LOCK_NAME]).catch(() => {});
      }
    } catch (error) {
      throw new X402StoreError(`x402 store migration failed: ${error.message}`, error);
    } finally {
      conn.release();
    }
  }
}

module.exports = { X402StoreMigrationManager };
