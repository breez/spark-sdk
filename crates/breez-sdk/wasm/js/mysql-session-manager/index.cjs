/**
 * CommonJS implementation for Node.js MySQL Session Manager.
 *
 * Mirrors `postgres-session-manager/index.cjs` for MySQL 8.0+.
 */

let mysql;
try {
  const mainModule = require.main;
  if (mainModule) {
    mysql = mainModule.require("mysql2/promise");
  } else {
    mysql = require("mysql2/promise");
  }
} catch (error) {
  try {
    mysql = require("mysql2/promise");
  } catch (fallbackError) {
    throw new Error(
      `mysql2 not found. Please install it in your project: npm install mysql2@^3.11.0\n` +
        `Original error: ${error.message}\nFallback error: ${fallbackError.message}`
    );
  }
}

const { SessionManagerError } = require("./errors.cjs");
const { MysqlSessionManagerMigrationManager } = require("./migrations.cjs");

class MysqlSessionManager {
  /**
   * @param {import('mysql2/promise').Pool} pool
   * @param {Buffer|Uint8Array} identity - 33-byte secp256k1 compressed pubkey
   *   identifying the tenant. All reads and writes are scoped by this.
   * @param {object} [logger]
   */
  constructor(pool, identity, logger = null) {
    if (!identity || identity.length !== 33) {
      throw new SessionManagerError(
        "tenant identity (33-byte secp256k1 pubkey) is required"
      );
    }
    this.pool = pool;
    this.identity = Buffer.from(identity);
    this.logger = logger;
  }

  async initialize() {
    try {
      const migrationManager = new MysqlSessionManagerMigrationManager(this.logger);
      await migrationManager.migrate(this.pool);
      return this;
    } catch (error) {
      throw new SessionManagerError(
        `Failed to initialize MySQL session manager: ${error.message}`,
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
   * @param {string} serviceIdentityKey - hex-encoded 33-byte secp256k1 pubkey
   * @returns {Promise<{token: string, expiration: number} | null>}
   */
  async getSession(serviceIdentityKey) {
    const serviceKey = _decodePubkey(serviceIdentityKey);
    try {
      const [rows] = await this.pool.execute(
        `SELECT token, expiration FROM sessions
         WHERE user_id = ? AND service_identity_key = ?`,
        [this.identity, serviceKey]
      );
      if (!rows || rows.length === 0) {
        return null;
      }
      const row = rows[0];
      return {
        token: row.token,
        expiration: Number(row.expiration),
      };
    } catch (error) {
      throw new SessionManagerError(
        `Failed to read session: ${error.message}`,
        error
      );
    }
  }

  /**
   * @param {string} serviceIdentityKey - hex-encoded 33-byte secp256k1 pubkey
   * @param {{token: string, expiration: number}} session
   */
  async setSession(serviceIdentityKey, session) {
    const serviceKey = _decodePubkey(serviceIdentityKey);
    try {
      await this.pool.execute(
        `INSERT INTO sessions (user_id, service_identity_key, token, expiration)
         VALUES (?, ?, ?, ?)
         ON DUPLICATE KEY UPDATE token = VALUES(token), expiration = VALUES(expiration)`,
        [this.identity, serviceKey, session.token, session.expiration]
      );
    } catch (error) {
      throw new SessionManagerError(
        `Failed to write session: ${error.message}`,
        error
      );
    }
  }
}

function _decodePubkey(hex) {
  if (typeof hex !== "string" || hex.length !== 66) {
    throw new SessionManagerError(
      "service_identity_key must be a 66-character hex-encoded 33-byte pubkey"
    );
  }
  return Buffer.from(hex, "hex");
}

async function createMysqlSessionManager(poolConfig, identity, logger = null) {
  const pool = mysql.createPool(poolConfig);
  const manager = new MysqlSessionManager(pool, identity, logger);
  await manager.initialize();
  return manager;
}

async function createMysqlSessionManagerWithPool(pool, identity, logger = null) {
  const manager = new MysqlSessionManager(pool, identity, logger);
  await manager.initialize();
  return manager;
}

module.exports = {
  MysqlSessionManager,
  createMysqlSessionManager,
  createMysqlSessionManagerWithPool,
  SessionManagerError,
};
