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

  deleteCachedItem(key) {
    try {
      const stmt = this.db.prepare("DELETE FROM settings WHERE key = ?");
      stmt.run(key);
      return Promise.resolve();
    } catch (error) {
      return Promise.reject(
        new StorageError(
          `Failed to delete cached item '${key}': ${error.message}`,
          error
        )
      );
    }
  }

  // ===== Payment Operations =====

  listPayments(request) {
    try {
      // Handle null/undefined values by using default values
      const actualOffset = request.offset != null ? request.offset : 0;
      const actualLimit = request.limit != null ? request.limit : 4294967295; // u32::MAX

      // Build WHERE clauses based on filters
      const whereClauses = [];
      const params = [];

      // Filter by payment type
      if (request.typeFilter && request.typeFilter.length > 0) {
        const placeholders = request.typeFilter.map(() => "?").join(", ");
        whereClauses.push(`p.payment_type IN (${placeholders})`);
        params.push(...request.typeFilter);
      }

      // Filter by status
      if (request.statusFilter && request.statusFilter.length > 0) {
        const placeholders = request.statusFilter.map(() => "?").join(", ");
        whereClauses.push(`p.status IN (${placeholders})`);
        params.push(...request.statusFilter);
      }

      // Filter by timestamp range
      if (request.fromTimestamp != null) {
        whereClauses.push("p.timestamp >= ?");
        params.push(request.fromTimestamp);
      }

      if (request.toTimestamp != null) {
        whereClauses.push("p.timestamp < ?");
        params.push(request.toTimestamp);
      }

      // Filter by payment details/method
      if (request.assetFilter) {
        const assetFilter = request.assetFilter;
        if (assetFilter.type === "bitcoin") {
          whereClauses.push("t.metadata IS NULL");
        } else if (assetFilter.type === "token") {
          whereClauses.push("t.metadata IS NOT NULL");
          if (assetFilter.tokenIdentifier) {
            whereClauses.push("json_extract(t.metadata, '$.identifier') = ?");
            params.push(assetFilter.tokenIdentifier);
          }
        }
      }

      // Build the WHERE clause
      const whereSql =
        whereClauses.length > 0 ? `WHERE ${whereClauses.join(" AND ")}` : "";

      // Determine sort order
      const orderDirection = request.sortAscending ? "ASC" : "DESC";

      const query = `
            SELECT p.id
            ,       p.payment_type
            ,       p.status
            ,       p.amount
            ,       p.fees
            ,       p.timestamp
            ,       p.method
            ,       p.withdraw_tx_id
            ,       p.deposit_tx_id
            ,       p.spark
            ,       l.invoice AS lightning_invoice
            ,       l.payment_hash AS lightning_payment_hash
            ,       l.destination_pubkey AS lightning_destination_pubkey
            ,       COALESCE(l.description, pm.lnurl_description) AS lightning_description
            ,       l.preimage AS lightning_preimage
            ,       pm.lnurl_pay_info
            ,       pm.lnurl_withdraw_info
            ,       t.metadata AS token_metadata
            ,       t.tx_hash AS token_tx_hash
            ,       t.invoice_details AS token_invoice_details
            ,       s.invoice_details AS spark_invoice_details
             FROM payments p
             LEFT JOIN payment_details_lightning l ON p.id = l.payment_id
             LEFT JOIN payment_details_token t ON p.id = t.payment_id
             LEFT JOIN payment_details_spark s ON p.id = s.payment_id
             LEFT JOIN payment_metadata pm ON p.id = pm.payment_id
             ${whereSql}
             ORDER BY p.timestamp ${orderDirection}
             LIMIT ? OFFSET ?
             `;

      params.push(actualLimit, actualOffset);
      const stmt = this.db.prepare(query);
      const rows = stmt.all(...params);
      return Promise.resolve(rows.map(this._rowToPayment.bind(this)));
    } catch (error) {
      return Promise.reject(
        new StorageError(
          `Failed to list payments (request: ${JSON.stringify(request)}: ${
            error.message
          }`,
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

      const paymentInsert = this.db.prepare(
        `INSERT OR REPLACE INTO payments (id, payment_type, status, amount, fees, timestamp, method, withdraw_tx_id, deposit_tx_id, spark) 
         VALUES (@id, @paymentType, @status, @amount, @fees, @timestamp, @method, @withdrawTxId, @depositTxId, @spark)`
      );
      const lightningInsert = this.db.prepare(
        `INSERT OR REPLACE INTO payment_details_lightning 
          (payment_id, invoice, payment_hash, destination_pubkey, description, preimage) 
          VALUES (@id, @invoice, @paymentHash, @destinationPubkey, @description, @preimage)`
      );
      const tokenInsert = this.db.prepare(
        `INSERT OR REPLACE INTO payment_details_token 
          (payment_id, metadata, tx_hash, invoice_details) 
          VALUES (@id, @metadata, @txHash, @invoiceDetails)`
      );
      const sparkInsert = this.db.prepare(
        `INSERT OR REPLACE INTO payment_details_spark 
          (payment_id, invoice_details) 
          VALUES (@id, @invoiceDetails)`
      );
      const transaction = this.db.transaction(() => {
        paymentInsert.run({
          id: payment.id,
          paymentType: payment.paymentType,
          status: payment.status,
          amount: payment.amount.toString(),
          fees: payment.fees.toString(),
          timestamp: payment.timestamp,
          method: payment.method ? JSON.stringify(payment.method) : null,
          withdrawTxId:
            payment.details?.type === "withdraw" ? payment.details.txId : null,
          depositTxId:
            payment.details?.type === "deposit" ? payment.details.txId : null,
          spark: payment.details?.type === "spark" ? 1 : null,
        });

        if (
          payment.details?.type === "spark" &&
          payment.details.invoiceDetails != null
        ) {
          sparkInsert.run({
            id: payment.id,
            invoiceDetails: payment.details.invoiceDetails
              ? JSON.stringify(payment.details.invoiceDetails)
              : null,
          });
        }

        if (payment.details?.type === "lightning") {
          lightningInsert.run({
            id: payment.id,
            invoice: payment.details.invoice,
            paymentHash: payment.details.paymentHash,
            destinationPubkey: payment.details.destinationPubkey,
            description: payment.details.description,
            preimage: payment.details.preimage,
          });
        }

        if (payment.details?.type === "token") {
          tokenInsert.run({
            id: payment.id,
            metadata: JSON.stringify(payment.details.metadata),
            txHash: payment.details.txHash,
            invoiceDetails: payment.details.invoiceDetails
              ? JSON.stringify(payment.details.invoiceDetails)
              : null,
          });
        }
      });

      transaction();
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
            SELECT p.id
            ,       p.payment_type
            ,       p.status
            ,       p.amount
            ,       p.fees
            ,       p.timestamp
            ,       p.method
            ,       p.withdraw_tx_id
            ,       p.deposit_tx_id
            ,       p.spark
            ,       l.invoice AS lightning_invoice
            ,       l.payment_hash AS lightning_payment_hash
            ,       l.destination_pubkey AS lightning_destination_pubkey
            ,       COALESCE(l.description, pm.lnurl_description) AS lightning_description
            ,       l.preimage AS lightning_preimage
            ,       pm.lnurl_pay_info
            ,       pm.lnurl_withdraw_info
            ,       t.metadata AS token_metadata
            ,       t.tx_hash AS token_tx_hash
            ,       t.invoice_details AS token_invoice_details
            ,       s.invoice_details AS spark_invoice_details
             FROM payments p
             LEFT JOIN payment_details_lightning l ON p.id = l.payment_id
             LEFT JOIN payment_details_token t ON p.id = t.payment_id
             LEFT JOIN payment_details_spark s ON p.id = s.payment_id
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

  getPaymentByInvoice(invoice) {
    try {
      if (!invoice) {
        return Promise.reject(
          new StorageError("Invoice cannot be null or undefined")
        );
      }

      const stmt = this.db.prepare(`
            SELECT p.id
            ,       p.payment_type
            ,       p.status
            ,       p.amount
            ,       p.fees
            ,       p.timestamp
            ,       p.method
            ,       p.withdraw_tx_id
            ,       p.deposit_tx_id
            ,       p.spark
            ,       l.invoice AS lightning_invoice
            ,       l.payment_hash AS lightning_payment_hash
            ,       l.destination_pubkey AS lightning_destination_pubkey
            ,       COALESCE(l.description, pm.lnurl_description) AS lightning_description
            ,       l.preimage AS lightning_preimage
            ,       pm.lnurl_pay_info
            ,       pm.lnurl_withdraw_info
            ,       t.metadata AS token_metadata
            ,       t.tx_hash AS token_tx_hash
            ,       t.invoice_details AS token_invoice_details
            ,       s.invoice_details AS spark_invoice_details
             FROM payments p
             LEFT JOIN payment_details_lightning l ON p.id = l.payment_id
             LEFT JOIN payment_details_token t ON p.id = t.payment_id
             LEFT JOIN payment_details_spark s ON p.id = s.payment_id
             LEFT JOIN payment_metadata pm ON p.id = pm.payment_id
             WHERE l.invoice = ?
            `);

      const row = stmt.get(invoice);
      if (!row) {
        return Promise.resolve(null);
      }

      return Promise.resolve(this._rowToPayment(row));
    } catch (error) {
      if (error instanceof StorageError) return Promise.reject(error);
      const paymentId = id || "unknown";
      return Promise.reject(
        new StorageError(
          `Failed to get payment by invoice '${invoice}': ${error.message}`,
          error
        )
      );
    }
  }

  setPaymentMetadata(paymentId, metadata) {
    try {
      const stmt = this.db.prepare(`
                INSERT OR REPLACE INTO payment_metadata (payment_id, lnurl_pay_info, lnurl_withdraw_info, lnurl_description) 
                VALUES (?, ?, ?, ?)
            `);

      stmt.run(
        paymentId,
        metadata.lnurlPayInfo ? JSON.stringify(metadata.lnurlPayInfo) : null,
        metadata.lnurlWithdrawInfo
          ? JSON.stringify(metadata.lnurlWithdrawInfo)
          : null,
        metadata.lnurlDescription
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

  // ===== Payment Request Metadata Operations =====

  getPaymentRequestMetadata(paymentRequest) {
    try {
      const stmt = this.db.prepare(`
                SELECT lnurl_withdraw_request_details, expires
                FROM payment_request_metadata 
                WHERE payment_request = ?
            `);

      const row = stmt.get(paymentRequest);
      if (!row) {
        return Promise.resolve(null);
      }

      return Promise.resolve({
        paymentRequest,
        lnurlWithdrawRequestDetails: row.lnurl_withdraw_request_details
          ? JSON.parse(row.lnurl_withdraw_request_details)
          : null,
        expires: row.expires,
      });
    } catch (error) {
      return Promise.reject(
        new StorageError(
          `Failed to get payment request metadata for '${paymentRequest}': ${error.message}`,
          error
        )
      );
    }
  }

  setPaymentRequestMetadata(metadata) {
    try {
      const stmt = this.db.prepare(`
                INSERT OR REPLACE INTO payment_request_metadata (payment_request, lnurl_withdraw_request_details, expires) 
                VALUES (?, ?, ?)
            `);

      stmt.run(
        metadata.paymentRequest,
        metadata.lnurlWithdrawRequestDetails
          ? JSON.stringify(metadata.lnurlWithdrawRequestDetails)
          : null,
        metadata.expires
      );
      return Promise.resolve();
    } catch (error) {
      return Promise.reject(
        new StorageError(
          `Failed to set payment request metadata for '${metadata.paymentRequest}': ${error.message}`,
          error
        )
      );
    }
  }

  deletePaymentRequestMetadata(paymentRequest) {
    try {
      const stmt = this.db.prepare(`
                DELETE FROM payment_request_metadata WHERE payment_request = ?
            `);

      stmt.run(paymentRequest);
      return Promise.resolve();
    } catch (error) {
      return Promise.reject(
        new StorageError(
          `Failed to delete payment request metadata for '${paymentRequest}': ${error.message}`,
          error
        )
      );
    }
  }

  deleteExpiredPaymentRequestMetadata(nowSecs) {
    try {
      const stmt = this.db.prepare(`
                DELETE FROM payment_request_metadata WHERE expires < ?
            `);

      stmt.run(nowSecs);
      return Promise.resolve();
    } catch (error) {
      return Promise.reject(
        new StorageError(
          `Failed to delete expired payment request metadata: ${error.message}`,
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
    if (row.lightning_invoice) {
      details = {
        type: "lightning",
        invoice: row.lightning_invoice,
        paymentHash: row.lightning_payment_hash,
        destinationPubkey: row.lightning_destination_pubkey,
        description: row.lightning_description,
        preimage: row.lightning_preimage,
      };

      if (row.lnurl_pay_info) {
        try {
          details.lnurlPayInfo = JSON.parse(row.lnurl_pay_info);
        } catch (e) {
          throw new StorageError(
            `Failed to parse lnurl_pay_info JSON for payment ${row.id}: ${e.message}`,
            e
          );
        }
      }

      if (row.lnurl_withdraw_info) {
        try {
          details.lnurlWithdrawInfo = JSON.parse(row.lnurl_withdraw_info);
        } catch (e) {
          throw new StorageError(
            `Failed to parse lnurl_withdraw_info JSON for payment ${row.id}: ${e.message}`,
            e
          );
        }
      }
    } else if (row.withdraw_tx_id) {
      details = {
        type: "withdraw",
        txId: row.withdraw_tx_id,
      };
    } else if (row.deposit_tx_id) {
      details = {
        type: "deposit",
        txId: row.deposit_tx_id,
      };
    } else if (row.spark) {
      details = {
        type: "spark",
        invoiceDetails: row.spark_invoice_details
          ? JSON.parse(row.spark_invoice_details)
          : null,
      };
    } else if (row.token_metadata) {
      details = {
        type: "token",
        metadata: JSON.parse(row.token_metadata),
        txHash: row.token_tx_hash,
        invoiceDetails: row.token_invoice_details
          ? JSON.parse(row.token_invoice_details)
          : null,
      };
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
  const fs = require("fs").promises;

  // Create directory if it doesn't exist
  await fs.mkdir(dataDir, { recursive: true });

  const dbPath = path.join(dataDir, "storage.sql");
  const storage = new SqliteStorage(dbPath, logger);
  storage.initialize();
  return storage;
}

// CommonJS exports
module.exports = { SqliteStorage, createDefaultStorage, StorageError };
