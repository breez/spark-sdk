/**
 * x402 replay records in PostgreSQL, scoped to one tenant identity. Mirrors
 * the Rust `PostgresX402ReplayStore`.
 */

const { X402StoreError } = require("./errors.cjs");
const { X402StoreMigrationManager } = require("./migrations.cjs");

class PostgresX402ReplayStore {
  /**
   * @param {import('pg').Pool} pool
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
        `DELETE FROM brz_x402_replay
         WHERE user_id = $1 AND retain_until < EXTRACT(EPOCH FROM now())::BIGINT`,
        [this.identity]
      );
      const { rowCount } = await this.pool.query(
        `INSERT INTO brz_x402_replay (user_id, replay_key, retain_until)
         VALUES ($1, $2, $3) ON CONFLICT DO NOTHING`,
        [this.identity, key, retainUntilSecs]
      );
      return rowCount === 1;
    } catch (error) {
      throw new X402StoreError(`Failed to record x402 payment: ${error.message}`, error);
    }
  }
}

/**
 * Creates the store over an existing pool, shared with the other PostgreSQL
 * stores.
 */
async function createPostgresX402ReplayStoreWithPool(pool, identity, runMigration = true) {
  const store = new PostgresX402ReplayStore(pool, identity);
  if (runMigration) {
    await new X402StoreMigrationManager().migrate(pool);
  }
  return store;
}

module.exports = {
  PostgresX402ReplayStore,
  createPostgresX402ReplayStoreWithPool,
  X402StoreError,
};
