/**
 * x402 replay records in MySQL, scoped to one tenant identity. Mirrors the
 * Rust `MysqlX402ReplayStore`.
 */

const { X402StoreError } = require("./errors.cjs");
const { X402StoreMigrationManager } = require("./migrations.cjs");

const ER_DUP_ENTRY = 1062;

class MysqlX402ReplayStore {
  /**
   * @param {import('mysql2/promise').Pool} pool
   * @param {Buffer|Uint8Array} identity - 33-byte tenant public key.
   */
  constructor(pool, identity) {
    if (!identity || identity.length !== 33) {
      throw new X402StoreError("tenant identity (33-byte secp256k1 pubkey) is required");
    }
    this.pool = pool;
    this.identity = Buffer.from(identity);
  }

  /**
   * Records `key` as used, unless it already is.
   * @param {string} key
   * @param {bigint} retainUntilSecs - When the record may be deleted.
   * @returns {Promise<boolean>} Whether this call recorded it.
   */
  async consume(key, retainUntilSecs) {
    try {
      // The database's clock, which every instance sharing the table agrees on.
      await this.pool.query(
        "DELETE FROM brz_x402_replay WHERE user_id = ? AND retain_until < UNIX_TIMESTAMP()",
        [this.identity]
      );
      // A plain insert, not `ON DUPLICATE KEY UPDATE`: mysql2 sets
      // `CLIENT_FOUND_ROWS`, under which a duplicate also reports one affected
      // row, so only the error tells them apart.
      await this.pool.query(
        "INSERT INTO brz_x402_replay (user_id, replay_key, retain_until) VALUES (?, ?, ?)",
        [this.identity, key, retainUntilSecs]
      );
      return true;
    } catch (error) {
      if (error.errno === ER_DUP_ENTRY) {
        return false;
      }
      throw new X402StoreError(`Failed to record x402 payment: ${error.message}`, error);
    }
  }
}

/**
 * Creates the store over an existing pool, shared with the other MySQL
 * stores.
 */
async function createMysqlX402ReplayStoreWithPool(pool, identity, runMigration = true) {
  const store = new MysqlX402ReplayStore(pool, identity);
  if (runMigration) {
    await new X402StoreMigrationManager().migrate(pool);
  }
  return store;
}

module.exports = {
  MysqlX402ReplayStore,
  createMysqlX402ReplayStoreWithPool,
  X402StoreError,
};
