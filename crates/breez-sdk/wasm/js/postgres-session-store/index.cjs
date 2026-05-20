/**
 * CommonJS implementation for Node.js PostgreSQL Session Store.
 *
 * Implements the JS-side `SessionStore` interface consumed by the Breez
 * SDK WASM bindings: `getSession(serviceIdentityKey)` returns the cached
 * session for the (tenant, service) pair or `null` when not found, and
 * `setSession(serviceIdentityKey, session)` upserts a session.
 *
 * Tenant identity is bound at construction so multiple tenants can share
 * a single Postgres database without leaking brz_sessions across tenants.
 */

let pg;
try {
  const mainModule = require.main;
  if (mainModule) {
    pg = mainModule.require("pg");
  } else {
    pg = require("pg");
  }
} catch (error) {
  try {
    pg = require("pg");
  } catch (fallbackError) {
    throw new Error(
      `pg not found. Please install it in your project: npm install pg@^8.18.0\n` +
        `Original error: ${error.message}\nFallback error: ${fallbackError.message}`
    );
  }
}

const { SessionStoreError } = require("./errors.cjs");
const { SessionStoreMigrationManager } = require("./migrations.cjs");

class PostgresSessionStore {
  /**
   * @param {import('pg').Pool} pool
   * @param {Buffer|Uint8Array} identity - 33-byte secp256k1 compressed pubkey
   *   identifying the tenant. All reads and writes are scoped by this.
   * @param {object} [logger]
   */
  constructor(pool, identity, logger = null, runMigration = true) {
    if (!identity || identity.length !== 33) {
      throw new SessionStoreError(
        "tenant identity (33-byte secp256k1 pubkey) is required"
      );
    }
    this.pool = pool;
    this.identity = Buffer.from(identity);
    this.logger = logger;
    this.runMigration = runMigration;
  }

  async initialize() {
    try {
      if (this.runMigration) {
        const migrationManager = new SessionStoreMigrationManager(this.logger);
        await migrationManager.migrate(this.pool);
      }
      return this;
    } catch (error) {
      throw new SessionStoreError(
        `Failed to initialize PostgreSQL session store: ${error.message}`,
        error
      );
    }
  }

  async close() {
    if (this.pool) {
      await this.pool.end();
      this.pool = null;
    }
  }

  /**
   * Returns the cached session for the given service identity key, or `null`
   * if no session is cached. The Rust adapter maps `null` to
   * `SessionStoreError::NotFound`.
   * @param {string} serviceIdentityKey - hex-encoded 33-byte secp256k1 pubkey
   * @returns {Promise<{token: string, expiration: number} | null>}
   */
  async getSession(serviceIdentityKey) {
    const serviceKey = _decodePubkey(serviceIdentityKey);
    try {
      const { rows } = await this.pool.query(
        `SELECT token, expiration FROM brz_sessions
         WHERE user_id = $1 AND service_identity_key = $2`,
        [this.identity, serviceKey]
      );
      if (rows.length === 0) {
        return null;
      }
      const row = rows[0];
      return {
        token: row.token,
        expiration: Number(row.expiration),
      };
    } catch (error) {
      throw new SessionStoreError(
        `Failed to read session: ${error.message}`,
        error
      );
    }
  }

  /**
   * Upserts a session for the given service identity key.
   * @param {string} serviceIdentityKey - hex-encoded 33-byte secp256k1 pubkey
   * @param {{token: string, expiration: number}} session
   */
  async setSession(serviceIdentityKey, session) {
    const serviceKey = _decodePubkey(serviceIdentityKey);
    try {
      await this.pool.query(
        `INSERT INTO brz_sessions (user_id, service_identity_key, token, expiration)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (user_id, service_identity_key)
         DO UPDATE SET token = EXCLUDED.token, expiration = EXCLUDED.expiration`,
        [this.identity, serviceKey, session.token, session.expiration]
      );
    } catch (error) {
      throw new SessionStoreError(
        `Failed to write session: ${error.message}`,
        error
      );
    }
  }
}

function _decodePubkey(hex) {
  if (typeof hex !== "string" || hex.length !== 66) {
    throw new SessionStoreError(
      "service_identity_key must be a 66-character hex-encoded 33-byte pubkey"
    );
  }
  return Buffer.from(hex, "hex");
}

/**
 * Convenience factory: creates a pool from a Pool config and returns an
 * initialized `PostgresSessionStore`. Most callers should use
 * `createPostgresSessionStoreWithPool` instead so the pool can be shared
 * across stores.
 */
async function createPostgresSessionStore(poolConfig, identity, logger = null) {
  const pool = new pg.Pool(poolConfig);
  const manager = new PostgresSessionStore(
    pool,
    identity,
    logger,
    poolConfig.runMigration !== false
  );
  await manager.initialize();
  return manager;
}

/**
 * Wraps an existing pool — useful when sharing the pool with the storage,
 * tree store, and token store implementations.
 */
async function createPostgresSessionStoreWithPool(
  pool,
  identity,
  logger = null,
  runMigration = true
) {
  const manager = new PostgresSessionStore(
    pool,
    identity,
    logger,
    runMigration
  );
  await manager.initialize();
  return manager;
}

module.exports = {
  PostgresSessionStore,
  createPostgresSessionStore,
  createPostgresSessionStoreWithPool,
  SessionStoreError,
};
