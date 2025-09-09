/**
 * CommonJS implementation for Node.js SQLite Storage
 */

// Try to require better-sqlite3 from the calling module's context
let Database;
try {
  // Get the calling module's directory to resolve dependencies from there
  const mainModule = require.main;
  if (mainModule) {
    Database = mainModule.require("better-sqlite3");
  } else {
    Database = require("better-sqlite3");
  }
} catch (error) {
  // Fallback: try direct require
  try {
    Database = require("better-sqlite3");
  } catch (fallbackError) {
    throw new Error(
      `better-sqlite3 not found. Please install it in your project: npm install better-sqlite3@^9.2.2\n` +
        `Original error: ${error.message}\nFallback error: ${fallbackError.message}`
    );
  }
}

const { StorageError } = require("./errors.cjs");
const { MigrationManager } = require("./migrations.cjs");

class SqliteStorage {
  constructor(dbPath, logger = null) {
    this.dbPath = dbPath;
    this.db = null;
    this.migrationManager = null;
    this.logger = logger;
  }

  /**
   * Initialize the database
   */
  initialize() {
    try {
      this.db = new Database(this.dbPath);

      this.migrationManager = new MigrationManager(
        this.db,
        StorageError,
        this.logger
      );
      this.migrationManager.migrate();

      return this;
    } catch (error) {
      throw new StorageError(
        `Failed to initialize database at '${this.dbPath}': ${error.message}`,
        error
      );
    }
  }

  /**
   * Close the database connection
   */
  close() {
    if (this.db) {
      this.db.close();
      this.db = null;
    }
  }

  // ===== Cache Operations =====

  getCachedItem(key) {
    try {
      const stmt = this.db.prepare("SELECT value FROM settings WHERE key = ?");
      const row = stmt.get(key);
      return Promise.resolve(row ? row.value : null);
    } catch (error) {
      return Promise.reject(
        new StorageError(
          `Failed to get cached item '${key}': ${error.message}`,
          error
        )
      );
    }
  }

  setCachedItem(key, value) {
    try {
      const stmt = this.db.prepare(
        "INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)"
      );
      stmt.run(key, value);
      return Promise.resolve();
    } catch (error) {
      return Promise.reject(
        new StorageError(
          `Failed to set cached item '${key}': ${error.message}`,
          error
        )
      );
    }
  }

  // ===== Payment Operations =====

  listPayments(offset = null, limit = null) {
    try {
      // Handle null values by using default values
      const actualOffset = offset !== null ? offset : 0;
      const actualLimit = limit !== null ? limit : 4294967295; // u32::MAX

      const stmt = this.db.prepare(`
                SELECT p.id, p.payment_type, p.status, p.amount, p.fees, p.timestamp, p.details, p.method, pm.lnurl_pay_info
                FROM payments p
                LEFT JOIN payment_metadata pm ON p.id = pm.payment_id
                ORDER BY p.timestamp DESC 
                LIMIT ? OFFSET ?
            `);

      const rows = stmt.all(actualLimit, actualOffset);
      return Promise.resolve(rows.map(this._rowToPayment.bind(this)));
    } catch (error) {
      return Promise.reject(
        new StorageError(
          `Failed to list payments (offset: ${offset}, limit: ${limit}): ${error.message}`,
          error
        )
      );
    }
  }

  insertPayment(payment) {
    try {
      if (!payment) {
        return Promise.reject(
          new StorageError("Payment cannot be null or undefined")
        );
      }

      const stmt = this.db.prepare(`
                 INSERT OR REPLACE INTO payments (id, payment_type, status, amount, fees, timestamp, details, method) 
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)
             `);

      stmt.run(
        payment.id,
        payment.paymentType,
        payment.status,
        payment.amount.toString(),
        payment.fees.toString(),
        payment.timestamp,
        payment.details ? JSON.stringify(payment.details) : null,
        payment.method ? JSON.stringify(payment.method) : null
      );
      return Promise.resolve();
    } catch (error) {
      return Promise.reject(
        new StorageError(
          `Failed to insert payment '${payment.id}': ${error.message}`,
          error
        )
      );
    }
  }

  getPaymentById(id) {
    try {
      if (!id) {
        return Promise.reject(
          new StorageError("Payment ID cannot be null or undefined")
        );
      }

      const stmt = this.db.prepare(`
                SELECT p.id, p.payment_type, p.status, p.amount, p.fees, p.timestamp, p.details, p.method, pm.lnurl_pay_info
                FROM payments p
                LEFT JOIN payment_metadata pm ON p.id = pm.payment_id
                WHERE p.id = ?
            `);

      const row = stmt.get(id);
      if (!row) {
        return Promise.reject(
          new StorageError(`Payment with id '${id}' not found`)
        );
      }

      return Promise.resolve(this._rowToPayment(row));
    } catch (error) {
      if (error instanceof StorageError) return Promise.reject(error);
      const paymentId = id || "unknown";
      return Promise.reject(
        new StorageError(
          `Failed to get payment by id '${paymentId}': ${error.message}`,
          error
        )
      );
    }
  }

  setPaymentMetadata(paymentId, metadata) {
    try {
      const stmt = this.db.prepare(`
                INSERT OR REPLACE INTO payment_metadata (payment_id, lnurl_pay_info) 
                VALUES (?, ?)
            `);

      stmt.run(
        paymentId,
        metadata.lnurlPayInfo ? JSON.stringify(metadata.lnurlPayInfo) : null
      );
      return Promise.resolve();
    } catch (error) {
      return Promise.reject(
        new StorageError(
          `Failed to set payment metadata for '${paymentId}': ${error.message}`,
          error
        )
      );
    }
  }

  // ===== Deposit Operations =====

  addDeposit(txid, vout, amountSats) {
    try {
      const stmt = this.db.prepare(`
                 INSERT OR IGNORE INTO unclaimed_deposits (txid, vout, amount_sats) 
                 VALUES (?, ?, ?)
             `);

      stmt.run(txid, vout, amountSats);
      return Promise.resolve();
    } catch (error) {
      return Promise.reject(
        new StorageError(
          `Failed to add deposit '${txid}:${vout}': ${error.message}`,
          error
        )
      );
    }
  }

  deleteDeposit(txid, vout) {
    try {
      const stmt = this.db.prepare(`
                DELETE FROM unclaimed_deposits WHERE txid = ? AND vout = ?
            `);

      stmt.run(txid, vout);
      return Promise.resolve();
    } catch (error) {
      return Promise.reject(
        new StorageError(
          `Failed to delete deposit '${txid}:${vout}': ${error.message}`,
          error
        )
      );
    }
  }

  listDeposits() {
    try {
      const stmt = this.db.prepare(`
                SELECT txid, vout, amount_sats, claim_error, refund_tx, refund_tx_id 
                FROM unclaimed_deposits
            `);

      const rows = stmt.all();
      return Promise.resolve(
        rows.map((row) => ({
          txid: row.txid,
          vout: row.vout,
          amountSats: row.amount_sats,
          claimError: row.claim_error ? JSON.parse(row.claim_error) : null,
          refundTx: row.refund_tx,
          refundTxId: row.refund_tx_id,
        }))
      );
    } catch (error) {
      return Promise.reject(
        new StorageError(`Failed to list deposits: ${error.message}`, error)
      );
    }
  }

  updateDeposit(txid, vout, payload) {
    try {
      if (payload.type === "claimError") {
        const stmt = this.db.prepare(`
          UPDATE unclaimed_deposits 
          SET claim_error = ?, refund_tx = NULL, refund_tx_id = NULL 
          WHERE txid = ? AND vout = ?
        `);

        stmt.run(JSON.stringify(payload.error), txid, vout);
      } else if (payload.type === "refund") {
        const stmt = this.db.prepare(`
          UPDATE unclaimed_deposits 
          SET refund_tx = ?, refund_tx_id = ?, claim_error = NULL 
          WHERE txid = ? AND vout = ?
        `);

        stmt.run(payload.refundTx, payload.refundTxid, txid, vout);
      } else {
        return Promise.reject(
          new StorageError(`Unknown payload type: ${payload.type}`)
        );
      }
      return Promise.resolve();
    } catch (error) {
      if (error instanceof StorageError) return Promise.reject(error);
      return Promise.reject(
        new StorageError(
          `Failed to update deposit '${txid}:${vout}': ${error.message}`,
          error
        )
      );
    }
  }

  // ===== Private Helper Methods =====

  _rowToPayment(row) {
    let details = null;
    if (row.details) {
      try {
        details = JSON.parse(row.details);
      } catch (e) {
        throw new StorageError(
          `Failed to parse payment details JSON for payment ${row.id}: ${e.message}`,
          e
        );
      }
    }

    let method = null;
    if (row.method) {
      try {
        method = JSON.parse(row.method);
      } catch (e) {
        throw new StorageError(
          `Failed to parse payment method JSON for payment ${row.id}: ${e.message}`,
          e
        );
      }
    }

    // If this is a Lightning payment and we have lnurl_pay_info, add it to details
    if (row.lnurl_pay_info && details && details.Lightning) {
      try {
        details.Lightning.lnurlPayInfo = JSON.parse(row.lnurl_pay_info);
      } catch (e) {
        throw new StorageError(
          `Failed to parse lnurl_pay_info JSON for payment ${row.id}: ${e.message}`,
          e
        );
      }
    }

    return {
      id: row.id,
      paymentType: row.payment_type,
      status: row.status,
      amount: BigInt(row.amount),
      fees: BigInt(row.fees),
      timestamp: row.timestamp,
      method,
      details,
    };
  }
}

async function createDefaultStorage(dataDir, logger = null) {
  const path = require("path");
  const dbPath = path.join(dataDir, "storage.sql");
  const storage = new SqliteStorage(dbPath, logger);
  storage.initialize();
  return storage;
}

// CommonJS exports
module.exports = { SqliteStorage, createDefaultStorage, StorageError };
