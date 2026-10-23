/**
 * x402 replay records in the wallet's SQLite storage file, for the Node.js
 * build. Mirrors the Rust `SqliteX402ReplayStore`.
 */

function requireDriver(name) {
  try {
    return (require.main ?? module).require(name);
  } catch {
    return require(name);
  }
}

const Database = requireDriver("better-sqlite3");

const { X402StoreError } = require("./errors.cjs");
const { X402StoreMigrationManager } = require("./migrations.cjs");

class SqliteX402ReplayStore {
  /** @param {string} dbPath */
  constructor(dbPath) {
    this.db = new Database(dbPath);
    new X402StoreMigrationManager(this.db).migrate();
  }

  /**
   * Records `key` as used, unless it already is.
   * @param {string} key
   * @param {bigint} retainUntilSecs - When the record may be deleted.
   * @returns {Promise<boolean>} Whether this call recorded it.
   */
  async consume(key, retainUntilSecs) {
    const now = Math.floor(Date.now() / 1000);
    try {
      return this.db.transaction(() => {
        this.db.prepare("DELETE FROM brz_x402_replay WHERE retain_until < ?").run(now);
        const { changes } = this.db
          .prepare(
            "INSERT OR IGNORE INTO brz_x402_replay (replay_key, retain_until) VALUES (?, ?)"
          )
          .run(key, retainUntilSecs);
        return changes === 1;
      })();
    } catch (error) {
      throw new X402StoreError(`Failed to record x402 payment: ${error.message}`, error);
    }
  }
}

/** @param {string} dbPath */
async function createSqliteX402ReplayStore(dbPath) {
  return new SqliteX402ReplayStore(dbPath);
}

/**
 * The replay store in the storage file `createDefaultStorage` opens for the
 * same directory.
 * @param {string} dataDir
 */
async function createDefaultX402ReplayStore(dataDir) {
  const path = require("path");
  await require("fs").promises.mkdir(dataDir, { recursive: true });
  return new SqliteX402ReplayStore(path.join(dataDir, "storage.sql"));
}

module.exports = {
  SqliteX402ReplayStore,
  createSqliteX402ReplayStore,
  createDefaultX402ReplayStore,
  X402StoreError,
};
