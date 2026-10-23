/**
 * Migrations for the x402 replay store in PostgreSQL. Mirrors the Rust
 * `PostgresX402ReplayStore` schema.
 */

const { X402StoreError } = require("./errors.cjs");

const MIGRATIONS_TABLE = "brz_x402_schema_migrations";
// The lock id the Rust migration runner derives, so Rust and JS servers
// sharing a database migrate in turn.
const MIGRATION_LOCK_ID = [...`migration_lock_${MIGRATIONS_TABLE}`].reduce(
  (sum, c) => sum + c.charCodeAt(0),
  0
);
const MIGRATIONS = [
  [
    `CREATE TABLE IF NOT EXISTS brz_x402_replay (
       user_id BYTEA NOT NULL,
       replay_key TEXT NOT NULL,
       retain_until BIGINT NOT NULL,
       PRIMARY KEY (user_id, replay_key)
     )`,
    `CREATE INDEX IF NOT EXISTS brz_idx_x402_replay_retain
       ON brz_x402_replay (user_id, retain_until)`,
  ],
];

class X402StoreMigrationManager {
  /** @param {import('pg').Pool} pool */
  async migrate(pool) {
    const client = await pool.connect();
    try {
      await client.query("BEGIN");
      await client.query("SELECT pg_advisory_xact_lock($1)", [MIGRATION_LOCK_ID]);
      await client.query(
        `CREATE TABLE IF NOT EXISTS ${MIGRATIONS_TABLE} (
           version INTEGER PRIMARY KEY,
           applied_at TIMESTAMPTZ DEFAULT NOW()
         )`
      );
      const { rows } = await client.query(
        `SELECT COALESCE(MAX(version), 0) AS version FROM ${MIGRATIONS_TABLE}`
      );
      for (let v = rows[0].version; v < MIGRATIONS.length; v++) {
        for (const sql of MIGRATIONS[v]) {
          await client.query(sql);
        }
        await client.query(`INSERT INTO ${MIGRATIONS_TABLE} (version) VALUES ($1)`, [v + 1]);
      }
      await client.query("COMMIT");
    } catch (error) {
      await client.query("ROLLBACK").catch(() => {});
      throw new X402StoreError(`x402 store migration failed: ${error.message}`, error);
    } finally {
      client.release();
    }
  }
}

module.exports = { X402StoreMigrationManager };
