/**
 * Migrations for the x402 replay store in the SDK's SQLite storage file.
 * Versioned in their own table, since the main storage owns the file's
 * `PRAGMA user_version`. Mirrors the Rust `SqliteX402ReplayStore` schema.
 */

const { X402StoreError } = require("./errors.cjs");

const MIGRATIONS_TABLE = "brz_x402_schema_migrations";
const MIGRATIONS = [
  `CREATE TABLE brz_x402_replay (
     replay_key TEXT PRIMARY KEY,
     retain_until INTEGER NOT NULL
   );
   CREATE INDEX brz_idx_x402_replay_retain ON brz_x402_replay (retain_until);`,
];

class X402StoreMigrationManager {
  /** @param {import('better-sqlite3').Database} db */
  constructor(db) {
    this.db = db;
  }

  migrate() {
    try {
      this.db.transaction(() => this._apply()).immediate();
    } catch (error) {
      throw new X402StoreError(`x402 store migration failed: ${error.message}`, error);
    }
  }

  _apply() {
    this.db.exec(
      `CREATE TABLE IF NOT EXISTS ${MIGRATIONS_TABLE} (version INTEGER PRIMARY KEY)`
    );
    const { version } = this.db
      .prepare(`SELECT COALESCE(MAX(version), 0) AS version FROM ${MIGRATIONS_TABLE}`)
      .get();
    for (let v = version; v < MIGRATIONS.length; v++) {
      this.db.exec(MIGRATIONS[v]);
      this.db.prepare(`INSERT INTO ${MIGRATIONS_TABLE} (version) VALUES (?)`).run(v + 1);
    }
  }
}

module.exports = { X402StoreMigrationManager };
