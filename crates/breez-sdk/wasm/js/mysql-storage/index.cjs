/**
 * CommonJS implementation for Node.js MySQL Storage.
 *
 * Mirrors `postgres-storage/index.cjs` for MySQL 8.0+. SQL translation rules:
 * - `$N` placeholders → `?`
 * - `JSONB` operators (`::jsonb->>`, `@>`) → `JSON_EXTRACT`/`JSON_UNQUOTE`/`JSON_CONTAINS`
 * - `ON CONFLICT (col) DO UPDATE SET col = EXCLUDED.col` →
 *     `ON DUPLICATE KEY UPDATE col = VALUES(col)`
 * - `ON CONFLICT DO NOTHING` → `INSERT … ON DUPLICATE KEY UPDATE <pk> = <pk>`
 *   (avoid `INSERT IGNORE`: it silently swallows non-PK errors too)
 * - `pg`'s `pool.query(sql, params)` returns `{ rows, rowCount }`; mysql2's
 *   `pool.query(sql, params)` returns `[rows, fields]` for SELECT and
 *   `[okPacket, fields]` (with `affectedRows`) for write operations.
 * - Reserved words like `key` need backtick quoting in MySQL.
 * - `pool.connect()` → `pool.getConnection()`; `client.query("BEGIN")`/COMMIT/ROLLBACK
 *   → `conn.beginTransaction()`/`conn.commit()`/`conn.rollback()`.
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

const { StorageError } = require("./errors.cjs");
const { MysqlMigrationManager } = require("./migrations.cjs");

/**
 * Base query for payment lookups. All columns are accessed by name in _rowToPayment.
 * parent_payment_id is only used by getPaymentsByParentIds.
 */
const SELECT_PAYMENT_SQL = `
    SELECT p.id,
           p.payment_type,
           p.status,
           p.amount,
           p.fees,
           p.timestamp,
           p.method,
           p.withdraw_tx_id,
           p.deposit_tx_id,
           p.spark,
           l.invoice AS lightning_invoice,
           l.payment_hash AS lightning_payment_hash,
           l.destination_pubkey AS lightning_destination_pubkey,
           COALESCE(l.description, pm.lnurl_description) AS lightning_description,
           l.preimage AS lightning_preimage,
           l.htlc_status AS lightning_htlc_status,
           l.htlc_expiry_time AS lightning_htlc_expiry_time,
           pm.lnurl_pay_info,
           pm.lnurl_withdraw_info,
           pm.conversion_info,
           pm.conversion_status,
           t.metadata AS token_metadata,
           t.tx_hash AS token_tx_hash,
           t.tx_type AS token_tx_type,
           t.invoice_details AS token_invoice_details,
           s.invoice_details AS spark_invoice_details,
           s.htlc_details AS spark_htlc_details,
           lrm.nostr_zap_request AS lnurl_nostr_zap_request,
           lrm.nostr_zap_receipt AS lnurl_nostr_zap_receipt,
           lrm.sender_comment AS lnurl_sender_comment,
           lrm.payment_hash AS lnurl_payment_hash,
           pm.parent_payment_id
      FROM payments p
      LEFT JOIN payment_details_lightning l ON p.id = l.payment_id
      LEFT JOIN payment_details_token t ON p.id = t.payment_id
      LEFT JOIN payment_details_spark s ON p.id = s.payment_id
      LEFT JOIN payment_metadata pm ON p.id = pm.payment_id
      LEFT JOIN lnurl_receive_metadata lrm ON l.payment_hash = lrm.payment_hash`;

/**
 * mysql2 may return JSON columns as either parsed objects or raw strings
 * depending on driver/server behavior. This helper normalizes both shapes.
 */
function parseJson(value) {
  if (value == null) return null;
  if (typeof value === "string") return JSON.parse(value);
  return value;
}

/** Normalize MySQL's TINYINT(1) to a JS boolean. */
function toBool(value) {
  if (value == null) return null;
  if (typeof value === "boolean") return value;
  return value === 1 || value === "1" || value === true;
}

class MysqlStorage {
  constructor(pool, logger = null) {
    this.pool = pool;
    this.logger = logger;
  }

  /** Initialize the database (run migrations). */
  async initialize() {
    try {
      const migrationManager = new MysqlMigrationManager(this.logger);
      await migrationManager.migrate(this.pool);
      return this;
    } catch (error) {
      throw new StorageError(
        `Failed to initialize MySQL database: ${error.message}`,
        error
      );
    }
  }

  /** Close the pool. */
  async close() {
    if (this.pool) {
      await this.pool.end();
      this.pool = null;
    }
  }

  /**
   * Run a function inside a transaction.
   * @param {function(import('mysql2/promise').PoolConnection): Promise<T>} fn
   * @returns {Promise<T>}
   * @template T
   */
  async _withTransaction(fn) {
    const conn = await this.pool.getConnection();
    try {
      await conn.beginTransaction();
      const result = await fn(conn);
      await conn.commit();
      return result;
    } catch (error) {
      await conn.rollback().catch(() => {});
      throw error;
    } finally {
      conn.release();
    }
  }

  // ===== Cache Operations =====

  async getCachedItem(key) {
    try {
      const [rows] = await this.pool.query(
        "SELECT value FROM settings WHERE `key` = ?",
        [key]
      );
      return rows.length > 0 ? rows[0].value : null;
    } catch (error) {
      throw new StorageError(
        `Failed to get cached item '${key}': ${error.message}`,
        error
      );
    }
  }

  async setCachedItem(key, value) {
    try {
      await this.pool.query(
        "INSERT INTO settings (`key`, value) VALUES (?, ?) " +
          "ON DUPLICATE KEY UPDATE value = VALUES(value)",
        [key, value]
      );
    } catch (error) {
      throw new StorageError(
        `Failed to set cached item '${key}': ${error.message}`,
        error
      );
    }
  }

  async deleteCachedItem(key) {
    try {
      await this.pool.query("DELETE FROM settings WHERE `key` = ?", [key]);
    } catch (error) {
      throw new StorageError(
        `Failed to delete cached item '${key}': ${error.message}`,
        error
      );
    }
  }

  // ===== Payment Operations =====

  async listPayments(request) {
    try {
      const actualOffset = request.offset != null ? request.offset : 0;
      const actualLimit =
        request.limit != null ? request.limit : 4294967295;

      const whereClauses = [];
      const params = [];

      if (request.typeFilter && request.typeFilter.length > 0) {
        const placeholders = request.typeFilter.map(() => "?");
        whereClauses.push(
          `p.payment_type IN (${placeholders.join(", ")})`
        );
        params.push(...request.typeFilter);
      }

      if (request.statusFilter && request.statusFilter.length > 0) {
        const placeholders = request.statusFilter.map(() => "?");
        whereClauses.push(`p.status IN (${placeholders.join(", ")})`);
        params.push(...request.statusFilter);
      }

      if (request.fromTimestamp != null) {
        whereClauses.push("p.timestamp >= ?");
        params.push(request.fromTimestamp);
      }

      if (request.toTimestamp != null) {
        whereClauses.push("p.timestamp < ?");
        params.push(request.toTimestamp);
      }

      if (
        request.paymentDetailsFilter &&
        request.paymentDetailsFilter.length > 0
      ) {
        const allPaymentDetailsClauses = [];
        for (const paymentDetailsFilter of request.paymentDetailsFilter) {
          const paymentDetailsClauses = [];

          const htlcAlias =
            paymentDetailsFilter.type === "spark"
              ? "s"
              : paymentDetailsFilter.type === "lightning"
                ? "l"
                : null;
          if (
            htlcAlias &&
            paymentDetailsFilter.htlcStatus !== undefined &&
            paymentDetailsFilter.htlcStatus.length > 0
          ) {
            const placeholders = paymentDetailsFilter.htlcStatus.map(() => "?");
            if (htlcAlias === "l") {
              paymentDetailsClauses.push(
                `l.htlc_status IN (${placeholders.join(", ")})`
              );
            } else {
              paymentDetailsClauses.push(
                `JSON_UNQUOTE(JSON_EXTRACT(s.htlc_details, '$.status')) IN (${placeholders.join(", ")})`
              );
            }
            params.push(...paymentDetailsFilter.htlcStatus);
          }

          if (
            (paymentDetailsFilter.type === "spark" ||
              paymentDetailsFilter.type === "token") &&
            paymentDetailsFilter.conversionRefundNeeded !== undefined
          ) {
            const typeCheck =
              paymentDetailsFilter.type === "spark"
                ? "p.spark = 1"
                : "p.spark IS NULL";
            const refundNeeded =
              paymentDetailsFilter.conversionRefundNeeded === true
                ? "= 'refundNeeded'"
                : "!= 'refundNeeded'";
            paymentDetailsClauses.push(
              `${typeCheck} AND pm.conversion_info IS NOT NULL AND
               JSON_UNQUOTE(JSON_EXTRACT(pm.conversion_info, '$.status')) ${refundNeeded}`
            );
          }

          if (
            paymentDetailsFilter.type === "token" &&
            paymentDetailsFilter.txHash !== undefined
          ) {
            paymentDetailsClauses.push("t.tx_hash = ?");
            params.push(paymentDetailsFilter.txHash);
          }

          if (
            paymentDetailsFilter.type === "token" &&
            paymentDetailsFilter.txType !== undefined
          ) {
            paymentDetailsClauses.push("t.tx_type = ?");
            params.push(paymentDetailsFilter.txType);
          }

          if (paymentDetailsClauses.length > 0) {
            allPaymentDetailsClauses.push(
              `(${paymentDetailsClauses.join(" AND ")})`
            );
          }
        }

        if (allPaymentDetailsClauses.length > 0) {
          whereClauses.push(`(${allPaymentDetailsClauses.join(" OR ")})`);
        }
      }

      if (request.assetFilter) {
        const assetFilter = request.assetFilter;
        if (assetFilter.type === "bitcoin") {
          whereClauses.push("t.metadata IS NULL");
        } else if (assetFilter.type === "token") {
          whereClauses.push("t.metadata IS NOT NULL");
          if (assetFilter.tokenIdentifier) {
            whereClauses.push(
              "JSON_UNQUOTE(JSON_EXTRACT(t.metadata, '$.identifier')) = ?"
            );
            params.push(assetFilter.tokenIdentifier);
          }
        }
      }

      whereClauses.push("pm.parent_payment_id IS NULL");

      const whereSql =
        whereClauses.length > 0 ? `WHERE ${whereClauses.join(" AND ")}` : "";

      const orderDirection = request.sortAscending ? "ASC" : "DESC";
      const query = `${SELECT_PAYMENT_SQL} ${whereSql} ORDER BY p.timestamp ${orderDirection} LIMIT ? OFFSET ?`;

      params.push(actualLimit, actualOffset);
      const [rows] = await this.pool.query(query, params);
      return rows.map(this._rowToPayment.bind(this));
    } catch (error) {
      if (error instanceof StorageError) throw error;
      throw new StorageError(
        `Failed to list payments (request: ${JSON.stringify(request)}): ${error.message}`,
        error
      );
    }
  }

  async insertPayment(payment) {
    try {
      if (!payment) {
        throw new StorageError("Payment cannot be null or undefined");
      }

      await this._withTransaction(async (conn) => {
        const withdrawTxId =
          payment.details?.type === "withdraw" ? payment.details.txId : null;
        const depositTxId =
          payment.details?.type === "deposit" ? payment.details.txId : null;
        const spark = payment.details?.type === "spark" ? 1 : null;

        await conn.query(
          `INSERT INTO payments (id, payment_type, status, amount, fees, timestamp, method, withdraw_tx_id, deposit_tx_id, spark)
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
           ON DUPLICATE KEY UPDATE
             payment_type=VALUES(payment_type),
             status=VALUES(status),
             amount=VALUES(amount),
             fees=VALUES(fees),
             timestamp=VALUES(timestamp),
             method=VALUES(method),
             withdraw_tx_id=VALUES(withdraw_tx_id),
             deposit_tx_id=VALUES(deposit_tx_id),
             spark=VALUES(spark)`,
          [
            payment.id,
            payment.paymentType,
            payment.status,
            payment.amount.toString(),
            payment.fees.toString(),
            payment.timestamp,
            payment.method ? JSON.stringify(payment.method) : null,
            withdrawTxId,
            depositTxId,
            spark,
          ]
        );

        if (
          payment.details?.type === "spark" &&
          (payment.details.invoiceDetails != null ||
            payment.details.htlcDetails != null)
        ) {
          await conn.query(
            `INSERT INTO payment_details_spark (payment_id, invoice_details, htlc_details)
             VALUES (?, ?, ?)
             ON DUPLICATE KEY UPDATE
               invoice_details=COALESCE(VALUES(invoice_details), invoice_details),
               htlc_details=COALESCE(VALUES(htlc_details), htlc_details)`,
            [
              payment.id,
              payment.details.invoiceDetails
                ? JSON.stringify(payment.details.invoiceDetails)
                : null,
              payment.details.htlcDetails
                ? JSON.stringify(payment.details.htlcDetails)
                : null,
            ]
          );
        }

        if (payment.details?.type === "lightning") {
          await conn.query(
            `INSERT INTO payment_details_lightning
              (payment_id, invoice, payment_hash, destination_pubkey, description, preimage, htlc_status, htlc_expiry_time)
              VALUES (?, ?, ?, ?, ?, ?, ?, ?)
              ON DUPLICATE KEY UPDATE
                invoice=VALUES(invoice),
                payment_hash=VALUES(payment_hash),
                destination_pubkey=VALUES(destination_pubkey),
                description=VALUES(description),
                preimage=COALESCE(VALUES(preimage), preimage),
                htlc_status=COALESCE(VALUES(htlc_status), htlc_status),
                htlc_expiry_time=COALESCE(VALUES(htlc_expiry_time), htlc_expiry_time)`,
            [
              payment.id,
              payment.details.invoice,
              payment.details.htlcDetails.paymentHash,
              payment.details.destinationPubkey,
              payment.details.description,
              payment.details.htlcDetails?.preimage,
              payment.details.htlcDetails?.status ?? null,
              payment.details.htlcDetails?.expiryTime ?? 0,
            ]
          );
        }

        if (payment.details?.type === "token") {
          await conn.query(
            `INSERT INTO payment_details_token
              (payment_id, metadata, tx_hash, tx_type, invoice_details)
              VALUES (?, ?, ?, ?, ?)
              ON DUPLICATE KEY UPDATE
                metadata=VALUES(metadata),
                tx_hash=VALUES(tx_hash),
                tx_type=VALUES(tx_type),
                invoice_details=COALESCE(VALUES(invoice_details), invoice_details)`,
            [
              payment.id,
              JSON.stringify(payment.details.metadata),
              payment.details.txHash,
              payment.details.txType,
              payment.details.invoiceDetails
                ? JSON.stringify(payment.details.invoiceDetails)
                : null,
            ]
          );
        }
      });
    } catch (error) {
      if (error instanceof StorageError) throw error;
      throw new StorageError(
        `Failed to insert payment '${payment.id}': ${error.message}`,
        error
      );
    }
  }

  async getPaymentById(id) {
    try {
      if (!id) {
        throw new StorageError("Payment ID cannot be null or undefined");
      }

      const [rows] = await this.pool.query(
        `${SELECT_PAYMENT_SQL} WHERE p.id = ?`,
        [id]
      );

      if (rows.length === 0) {
        throw new StorageError(`Payment with id '${id}' not found`);
      }

      return this._rowToPayment(rows[0]);
    } catch (error) {
      if (error instanceof StorageError) throw error;
      throw new StorageError(
        `Failed to get payment by id '${id || "unknown"}': ${error.message}`,
        error
      );
    }
  }

  async getPaymentByInvoice(invoice) {
    try {
      if (!invoice) {
        throw new StorageError("Invoice cannot be null or undefined");
      }

      const [rows] = await this.pool.query(
        `${SELECT_PAYMENT_SQL} WHERE l.invoice = ?`,
        [invoice]
      );

      if (rows.length === 0) {
        return null;
      }

      return this._rowToPayment(rows[0]);
    } catch (error) {
      if (error instanceof StorageError) throw error;
      throw new StorageError(
        `Failed to get payment by invoice '${invoice}': ${error.message}`,
        error
      );
    }
  }

  async getPaymentsByParentIds(parentPaymentIds) {
    try {
      if (!parentPaymentIds || parentPaymentIds.length === 0) {
        return {};
      }

      // Early exit if no related payments exist. mysql2 returns EXISTS as 0/1.
      const [hasRelatedRows] = await this.pool.query(
        "SELECT EXISTS(SELECT 1 FROM payment_metadata WHERE parent_payment_id IS NOT NULL LIMIT 1) AS has_related"
      );
      if (!hasRelatedRows[0].has_related) {
        return {};
      }

      const placeholders = parentPaymentIds.map(() => "?");
      const query = `${SELECT_PAYMENT_SQL} WHERE pm.parent_payment_id IN (${placeholders.join(", ")}) ORDER BY p.timestamp ASC`;

      const [rows] = await this.pool.query(query, parentPaymentIds);

      const result = {};
      for (const row of rows) {
        const parentId = row.parent_payment_id;
        if (!result[parentId]) {
          result[parentId] = [];
        }
        result[parentId].push(this._rowToPayment(row));
      }

      return result;
    } catch (error) {
      if (error instanceof StorageError) throw error;
      throw new StorageError(
        `Failed to get payments by parent ids: ${error.message}`,
        error
      );
    }
  }

  async insertPaymentMetadata(paymentId, metadata) {
    try {
      await this.pool.query(
        `INSERT INTO payment_metadata (payment_id, parent_payment_id, lnurl_pay_info, lnurl_withdraw_info, lnurl_description, conversion_info, conversion_status)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON DUPLICATE KEY UPDATE
           parent_payment_id = COALESCE(VALUES(parent_payment_id), parent_payment_id),
           lnurl_pay_info = COALESCE(VALUES(lnurl_pay_info), lnurl_pay_info),
           lnurl_withdraw_info = COALESCE(VALUES(lnurl_withdraw_info), lnurl_withdraw_info),
           lnurl_description = COALESCE(VALUES(lnurl_description), lnurl_description),
           conversion_info = COALESCE(VALUES(conversion_info), conversion_info),
           conversion_status = COALESCE(VALUES(conversion_status), conversion_status)`,
        [
          paymentId,
          metadata.parentPaymentId,
          metadata.lnurlPayInfo
            ? JSON.stringify(metadata.lnurlPayInfo)
            : null,
          metadata.lnurlWithdrawInfo
            ? JSON.stringify(metadata.lnurlWithdrawInfo)
            : null,
          metadata.lnurlDescription,
          metadata.conversionInfo
            ? JSON.stringify(metadata.conversionInfo)
            : null,
          metadata.conversionStatus ?? null,
        ]
      );
    } catch (error) {
      throw new StorageError(
        `Failed to set payment metadata for '${paymentId}': ${error.message}`,
        error
      );
    }
  }

  // ===== Deposit Operations =====

  async addDeposit(txid, vout, amountSats, isMature) {
    try {
      await this.pool.query(
        `INSERT INTO unclaimed_deposits (txid, vout, amount_sats, is_mature)
         VALUES (?, ?, ?, ?)
         ON DUPLICATE KEY UPDATE is_mature = VALUES(is_mature), amount_sats = VALUES(amount_sats)`,
        [txid, vout, amountSats, isMature ? 1 : 0]
      );
    } catch (error) {
      throw new StorageError(
        `Failed to add deposit '${txid}:${vout}': ${error.message}`,
        error
      );
    }
  }

  async deleteDeposit(txid, vout) {
    try {
      await this.pool.query(
        "DELETE FROM unclaimed_deposits WHERE txid = ? AND vout = ?",
        [txid, vout]
      );
    } catch (error) {
      throw new StorageError(
        `Failed to delete deposit '${txid}:${vout}': ${error.message}`,
        error
      );
    }
  }

  async listDeposits() {
    try {
      const [rows] = await this.pool.query(
        "SELECT txid, vout, amount_sats, is_mature, claim_error, refund_tx, refund_tx_id FROM unclaimed_deposits"
      );

      return rows.map((row) => ({
        txid: row.txid,
        vout: row.vout,
        amountSats:
          row.amount_sats != null ? BigInt(row.amount_sats) : BigInt(0),
        isMature: toBool(row.is_mature) ?? true,
        claimError: parseJson(row.claim_error),
        refundTx: row.refund_tx,
        refundTxId: row.refund_tx_id,
      }));
    } catch (error) {
      throw new StorageError(
        `Failed to list deposits: ${error.message}`,
        error
      );
    }
  }

  async updateDeposit(txid, vout, payload) {
    try {
      if (payload.type === "claimError") {
        await this.pool.query(
          `UPDATE unclaimed_deposits
           SET claim_error = ?, refund_tx = NULL, refund_tx_id = NULL
           WHERE txid = ? AND vout = ?`,
          [JSON.stringify(payload.error), txid, vout]
        );
      } else if (payload.type === "refund") {
        await this.pool.query(
          `UPDATE unclaimed_deposits
           SET refund_tx = ?, refund_tx_id = ?, claim_error = NULL
           WHERE txid = ? AND vout = ?`,
          [payload.refundTx, payload.refundTxid, txid, vout]
        );
      } else {
        throw new StorageError(`Unknown payload type: ${payload.type}`);
      }
    } catch (error) {
      if (error instanceof StorageError) throw error;
      throw new StorageError(
        `Failed to update deposit '${txid}:${vout}': ${error.message}`,
        error
      );
    }
  }

  async setLnurlMetadata(metadata) {
    try {
      await this._withTransaction(async (conn) => {
        for (const item of metadata) {
          await conn.query(
            `INSERT INTO lnurl_receive_metadata (payment_hash, nostr_zap_request, nostr_zap_receipt, sender_comment)
             VALUES (?, ?, ?, ?)
             ON DUPLICATE KEY UPDATE
               nostr_zap_request = VALUES(nostr_zap_request),
               nostr_zap_receipt = VALUES(nostr_zap_receipt),
               sender_comment = VALUES(sender_comment)`,
            [
              item.paymentHash,
              item.nostrZapRequest || null,
              item.nostrZapReceipt || null,
              item.senderComment || null,
            ]
          );
        }
      });
    } catch (error) {
      if (error instanceof StorageError) throw error;
      throw new StorageError(
        `Failed to add lnurl metadata: ${error.message}`,
        error
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
        destinationPubkey: row.lightning_destination_pubkey,
        description: row.lightning_description,
        htlcDetails: row.lightning_htlc_status
          ? {
              paymentHash: row.lightning_payment_hash,
              preimage: row.lightning_preimage || null,
              expiryTime: Number(row.lightning_htlc_expiry_time) ?? 0,
              status: row.lightning_htlc_status,
            }
          : (() => {
              throw new StorageError(
                `htlc_status is required for Lightning payment ${row.id}`
              );
            })(),
      };

      if (row.lnurl_pay_info) {
        details.lnurlPayInfo = parseJson(row.lnurl_pay_info);
      }

      if (row.lnurl_withdraw_info) {
        details.lnurlWithdrawInfo = parseJson(row.lnurl_withdraw_info);
      }

      if (row.lnurl_payment_hash) {
        details.lnurlReceiveMetadata = {
          nostrZapRequest: row.lnurl_nostr_zap_request || null,
          nostrZapReceipt: row.lnurl_nostr_zap_receipt || null,
          senderComment: row.lnurl_sender_comment || null,
        };
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
    } else if (toBool(row.spark)) {
      details = {
        type: "spark",
        invoiceDetails: parseJson(row.spark_invoice_details),
        htlcDetails: parseJson(row.spark_htlc_details),
        conversionInfo: parseJson(row.conversion_info),
      };
    } else if (row.token_metadata) {
      details = {
        type: "token",
        metadata: parseJson(row.token_metadata),
        txHash: row.token_tx_hash,
        txType: row.token_tx_type,
        invoiceDetails: parseJson(row.token_invoice_details),
        conversionInfo: parseJson(row.conversion_info),
      };
    }

    let method = null;
    if (row.method) {
      try {
        method = parseJson(row.method);
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
      timestamp: Number(row.timestamp),
      method,
      details,
      conversionDetails: row.conversion_status
        ? { status: row.conversion_status, from: null, to: null }
        : null,
    };
  }

  // ===== Contact Operations =====

  async listContacts(request) {
    try {
      const offset = request.offset != null ? request.offset : 0;
      const limit = request.limit != null ? request.limit : 4294967295;

      const [rows] = await this.pool.query(
        `SELECT id, name, payment_identifier, created_at, updated_at
         FROM contacts
         ORDER BY name ASC
         LIMIT ? OFFSET ?`,
        [limit, offset]
      );

      return rows.map((row) => ({
        id: row.id,
        name: row.name,
        paymentIdentifier: row.payment_identifier,
        createdAt: Number(row.created_at),
        updatedAt: Number(row.updated_at),
      }));
    } catch (error) {
      throw new StorageError(
        `Failed to list contacts: ${error.message}`,
        error
      );
    }
  }

  async getContact(id) {
    try {
      const [rows] = await this.pool.query(
        `SELECT id, name, payment_identifier, created_at, updated_at
         FROM contacts
         WHERE id = ?`,
        [id]
      );

      if (rows.length === 0) {
        return null;
      }

      const row = rows[0];
      return {
        id: row.id,
        name: row.name,
        paymentIdentifier: row.payment_identifier,
        createdAt: Number(row.created_at),
        updatedAt: Number(row.updated_at),
      };
    } catch (error) {
      throw new StorageError(`Failed to get contact: ${error.message}`, error);
    }
  }

  async insertContact(contact) {
    try {
      await this.pool.query(
        `INSERT INTO contacts (id, name, payment_identifier, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?)
         ON DUPLICATE KEY UPDATE
           name = VALUES(name),
           payment_identifier = VALUES(payment_identifier),
           updated_at = VALUES(updated_at)`,
        [
          contact.id,
          contact.name,
          contact.paymentIdentifier,
          contact.createdAt,
          contact.updatedAt,
        ]
      );
    } catch (error) {
      throw new StorageError(
        `Failed to insert contact: ${error.message}`,
        error
      );
    }
  }

  async deleteContact(id) {
    try {
      await this.pool.query("DELETE FROM contacts WHERE id = ?", [id]);
    } catch (error) {
      throw new StorageError(
        `Failed to delete contact: ${error.message}`,
        error
      );
    }
  }

  // ===== Sync Operations =====

  async syncAddOutgoingChange(record) {
    try {
      return await this._withTransaction(async (conn) => {
        const [revisionRows] = await conn.query(
          "SELECT COALESCE(MAX(revision), 0) + 1 AS revision FROM sync_outgoing"
        );
        const revision = BigInt(revisionRows[0].revision);

        await conn.query(
          `INSERT INTO sync_outgoing (
            record_type,
            data_id,
            schema_version,
            commit_time,
            updated_fields_json,
            revision
          ) VALUES (?, ?, ?, ?, ?, ?)`,
          [
            record.id.type,
            record.id.dataId,
            record.schemaVersion,
            Math.floor(Date.now() / 1000),
            JSON.stringify(record.updatedFields),
            revision.toString(),
          ]
        );

        return revision;
      });
    } catch (error) {
      if (error instanceof StorageError) throw error;
      throw new StorageError(
        `Failed to add outgoing change: ${error.message}`,
        error
      );
    }
  }

  async syncCompleteOutgoingSync(record, localRevision) {
    try {
      await this._withTransaction(async (conn) => {
        const [deleteResult] = await conn.query(
          "DELETE FROM sync_outgoing WHERE record_type = ? AND data_id = ? AND revision = ?",
          [record.id.type, record.id.dataId, localRevision.toString()]
        );

        if (deleteResult.affectedRows === 0) {
          const msg = `complete_outgoing_sync: DELETE from sync_outgoing matched 0 rows (type=${record.id.type}, data_id=${record.id.dataId}, revision=${localRevision})`;
          if (this.logger && typeof this.logger.log === "function") {
            this.logger.log({ line: msg, level: "warn" });
          } else {
            // eslint-disable-next-line no-console
            console.warn(`[MysqlStorage] ${msg}`);
          }
        }

        await conn.query(
          `INSERT INTO sync_state (
            record_type,
            data_id,
            revision,
            schema_version,
            commit_time,
            data
          ) VALUES (?, ?, ?, ?, ?, ?)
          ON DUPLICATE KEY UPDATE
            schema_version = VALUES(schema_version),
            commit_time = VALUES(commit_time),
            data = VALUES(data),
            revision = VALUES(revision)`,
          [
            record.id.type,
            record.id.dataId,
            record.revision.toString(),
            record.schemaVersion,
            Math.floor(Date.now() / 1000),
            JSON.stringify(record.data),
          ]
        );

        await conn.query(
          "UPDATE sync_revision SET revision = GREATEST(revision, ?)",
          [record.revision.toString()]
        );
      });
    } catch (error) {
      if (error instanceof StorageError) throw error;
      throw new StorageError(
        `Failed to complete outgoing sync: ${error.message}`,
        error
      );
    }
  }

  async syncGetPendingOutgoingChanges(limit) {
    try {
      const [rows] = await this.pool.query(
        `SELECT
          o.record_type,
          o.data_id,
          o.schema_version,
          o.commit_time,
          o.updated_fields_json,
          o.revision,
          e.schema_version AS existing_schema_version,
          e.commit_time AS existing_commit_time,
          e.data AS existing_data,
          e.revision AS existing_revision
        FROM sync_outgoing o
        LEFT JOIN sync_state e ON
          o.record_type = e.record_type AND
          o.data_id = e.data_id
        ORDER BY o.revision ASC
        LIMIT ?`,
        [limit]
      );

      return rows.map((row) => {
        const change = {
          id: { type: row.record_type, dataId: row.data_id },
          schemaVersion: row.schema_version,
          updatedFields: parseJson(row.updated_fields_json),
          localRevision: BigInt(row.revision),
        };

        let parent = null;
        if (row.existing_data) {
          parent = {
            id: { type: row.record_type, dataId: row.data_id },
            revision: BigInt(row.existing_revision),
            schemaVersion: row.existing_schema_version,
            data: parseJson(row.existing_data),
          };
        }

        return { change, parent };
      });
    } catch (error) {
      throw new StorageError(
        `Failed to get pending outgoing changes: ${error.message}`,
        error
      );
    }
  }

  async syncGetLastRevision() {
    try {
      const [rows] = await this.pool.query(
        "SELECT revision FROM sync_revision"
      );
      return rows.length > 0 ? BigInt(rows[0].revision) : BigInt(0);
    } catch (error) {
      throw new StorageError(
        `Failed to get last revision: ${error.message}`,
        error
      );
    }
  }

  async syncInsertIncomingRecords(records) {
    try {
      if (!records || records.length === 0) {
        return;
      }

      await this._withTransaction(async (conn) => {
        for (const record of records) {
          await conn.query(
            `INSERT INTO sync_incoming (
              record_type,
              data_id,
              schema_version,
              commit_time,
              data,
              revision
            ) VALUES (?, ?, ?, ?, ?, ?)
            ON DUPLICATE KEY UPDATE
              schema_version = VALUES(schema_version),
              commit_time = VALUES(commit_time),
              data = VALUES(data)`,
            [
              record.id.type,
              record.id.dataId,
              record.schemaVersion,
              Math.floor(Date.now() / 1000),
              JSON.stringify(record.data),
              record.revision.toString(),
            ]
          );
        }
      });
    } catch (error) {
      if (error instanceof StorageError) throw error;
      throw new StorageError(
        `Failed to insert incoming records: ${error.message}`,
        error
      );
    }
  }

  async syncDeleteIncomingRecord(record) {
    try {
      await this.pool.query(
        `DELETE FROM sync_incoming
         WHERE record_type = ?
         AND data_id = ?
         AND revision = ?`,
        [record.id.type, record.id.dataId, record.revision.toString()]
      );
    } catch (error) {
      throw new StorageError(
        `Failed to delete incoming record: ${error.message}`,
        error
      );
    }
  }

  async syncGetIncomingRecords(limit) {
    try {
      const [rows] = await this.pool.query(
        `SELECT i.record_type,
                i.data_id,
                i.schema_version,
                i.data,
                i.revision,
                e.schema_version AS existing_schema_version,
                e.commit_time AS existing_commit_time,
                e.data AS existing_data,
                e.revision AS existing_revision
         FROM sync_incoming i
         LEFT JOIN sync_state e ON i.record_type = e.record_type AND i.data_id = e.data_id
         ORDER BY i.revision ASC
         LIMIT ?`,
        [limit]
      );

      return rows.map((row) => {
        const newState = {
          id: { type: row.record_type, dataId: row.data_id },
          revision: BigInt(row.revision),
          schemaVersion: row.schema_version,
          data: parseJson(row.data),
        };

        let oldState = null;
        if (row.existing_data) {
          oldState = {
            id: { type: row.record_type, dataId: row.data_id },
            revision: BigInt(row.existing_revision),
            schemaVersion: row.existing_schema_version,
            data: parseJson(row.existing_data),
          };
        }

        return { newState, oldState };
      });
    } catch (error) {
      throw new StorageError(
        `Failed to get incoming records: ${error.message}`,
        error
      );
    }
  }

  async syncGetLatestOutgoingChange() {
    try {
      const [rows] = await this.pool.query(
        `SELECT
          o.record_type,
          o.data_id,
          o.schema_version,
          o.commit_time,
          o.updated_fields_json,
          o.revision,
          e.schema_version AS existing_schema_version,
          e.commit_time AS existing_commit_time,
          e.data AS existing_data,
          e.revision AS existing_revision
        FROM sync_outgoing o
        LEFT JOIN sync_state e ON
          o.record_type = e.record_type AND
          o.data_id = e.data_id
        ORDER BY o.revision DESC
        LIMIT 1`
      );

      if (rows.length === 0) {
        return null;
      }

      const row = rows[0];

      const change = {
        id: { type: row.record_type, dataId: row.data_id },
        schemaVersion: row.schema_version,
        updatedFields: parseJson(row.updated_fields_json),
        localRevision: BigInt(row.revision),
      };

      let parent = null;
      if (row.existing_data) {
        parent = {
          id: { type: row.record_type, dataId: row.data_id },
          revision: BigInt(row.existing_revision),
          schemaVersion: row.existing_schema_version,
          data: parseJson(row.existing_data),
        };
      }

      return { change, parent };
    } catch (error) {
      throw new StorageError(
        `Failed to get latest outgoing change: ${error.message}`,
        error
      );
    }
  }

  async syncUpdateRecordFromIncoming(record) {
    try {
      await this._withTransaction(async (conn) => {
        await conn.query(
          `INSERT INTO sync_state (
            record_type,
            data_id,
            revision,
            schema_version,
            commit_time,
            data
          ) VALUES (?, ?, ?, ?, ?, ?)
          ON DUPLICATE KEY UPDATE
            schema_version = VALUES(schema_version),
            commit_time = VALUES(commit_time),
            data = VALUES(data),
            revision = VALUES(revision)`,
          [
            record.id.type,
            record.id.dataId,
            record.revision.toString(),
            record.schemaVersion,
            Math.floor(Date.now() / 1000),
            JSON.stringify(record.data),
          ]
        );

        await conn.query(
          "UPDATE sync_revision SET revision = GREATEST(revision, ?)",
          [record.revision.toString()]
        );
      });
    } catch (error) {
      if (error instanceof StorageError) throw error;
      throw new StorageError(
        `Failed to update record from incoming: ${error.message}`,
        error
      );
    }
  }
}

/**
 * Creates a MysqlStorageConfig with the given connection string and default pool settings.
 *
 * Default values:
 * - maxPoolSize: 10
 * - createTimeoutSecs: 0 (no timeout)
 * - recycleTimeoutSecs: 10 (10 seconds idle before disconnect)
 *
 * @param {string} connectionString - MySQL connection URL (mysql://user:pass@host:3306/db)
 * @returns {object} MySQL storage configuration
 */
function defaultMysqlStorageConfig(connectionString) {
  return {
    connectionString,
    maxPoolSize: 10,
    createTimeoutSecs: 0,
    recycleTimeoutSecs: 10,
  };
}

/**
 * Create a mysql2 pool from a config object.
 * The returned pool can be shared across multiple store implementations.
 */
function createMysqlPool(config) {
  return mysql.createPool({
    uri: config.connectionString,
    connectionLimit: config.maxPoolSize,
    connectTimeout: (config.createTimeoutSecs || 0) * 1000 || 10000,
    idleTimeout: (config.recycleTimeoutSecs || 0) * 1000 || 10000,
    waitForConnections: true,
  });
}

/**
 * Create a MysqlStorage instance from an existing mysql2 pool.
 */
async function createMysqlStorageWithPool(pool, logger = null) {
  const storage = new MysqlStorage(pool, logger);
  await storage.initialize();
  return storage;
}

/**
 * Create a MysqlStorage instance from a config object.
 * Use defaultMysqlStorageConfig to create a config with sensible defaults.
 */
async function createMysqlStorage(config, logger = null) {
  const pool = createMysqlPool(config);
  return createMysqlStorageWithPool(pool, logger);
}

module.exports = {
  MysqlStorage,
  createMysqlStorage,
  createMysqlPool,
  createMysqlStorageWithPool,
  defaultMysqlStorageConfig,
  StorageError,
};
