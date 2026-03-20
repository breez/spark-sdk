/**
 * CommonJS implementation for Node.js PostgreSQL Storage
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

const { StorageError } = require("./errors.cjs");
const { PostgresMigrationManager } = require("./migrations.cjs");

/**
 * Base query for payment lookups.
 * All columns are accessed by name in _rowToPayment.
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

class PostgresStorage {
  constructor(pool, logger = null) {
    this.pool = pool;
    this.logger = logger;
  }

  /**
   * Initialize the database (run migrations)
   */
  async initialize() {
    try {
      const migrationManager = new PostgresMigrationManager(this.logger);
      await migrationManager.migrate(this.pool);
      return this;
    } catch (error) {
      throw new StorageError(
        `Failed to initialize PostgreSQL database: ${error.message}`,
        error
      );
    }
  }

  /**
   * Close the pool
   */
  async close() {
    if (this.pool) {
      await this.pool.end();
      this.pool = null;
    }
  }

  /**
   * Run a function inside a transaction.
   * @param {function(import('pg').PoolClient): Promise<T>} fn
   * @returns {Promise<T>}
   * @template T
   */
  async _withTransaction(fn) {
    const client = await this.pool.connect();
    try {
      await client.query("BEGIN");
      const result = await fn(client);
      await client.query("COMMIT");
      return result;
    } catch (error) {
      await client.query("ROLLBACK").catch(() => {});
      throw error;
    } finally {
      client.release();
    }
  }

  // ===== Cache Operations =====

  async getCachedItem(key) {
    try {
      const result = await this.pool.query(
        "SELECT value FROM settings WHERE key = $1",
        [key]
      );
      return result.rows.length > 0 ? result.rows[0].value : null;
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
        `INSERT INTO settings (key, value) VALUES ($1, $2)
         ON CONFLICT(key) DO UPDATE SET value = EXCLUDED.value`,
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
      await this.pool.query("DELETE FROM settings WHERE key = $1", [key]);
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
      let paramIdx = 1;

      // Filter by payment type
      if (request.typeFilter && request.typeFilter.length > 0) {
        const placeholders = request.typeFilter.map(
          () => `$${paramIdx++}`
        );
        whereClauses.push(
          `p.payment_type IN (${placeholders.join(", ")})`
        );
        params.push(...request.typeFilter);
      }

      // Filter by status
      if (request.statusFilter && request.statusFilter.length > 0) {
        const placeholders = request.statusFilter.map(
          () => `$${paramIdx++}`
        );
        whereClauses.push(`p.status IN (${placeholders.join(", ")})`);
        params.push(...request.statusFilter);
      }

      // Filter by timestamp range
      if (request.fromTimestamp != null) {
        whereClauses.push(`p.timestamp >= $${paramIdx++}`);
        params.push(request.fromTimestamp);
      }

      if (request.toTimestamp != null) {
        whereClauses.push(`p.timestamp < $${paramIdx++}`);
        params.push(request.toTimestamp);
      }

      // Filter by payment details
      if (
        request.paymentDetailsFilter &&
        request.paymentDetailsFilter.length > 0
      ) {
        const allPaymentDetailsClauses = [];
        for (const paymentDetailsFilter of request.paymentDetailsFilter) {
          const paymentDetailsClauses = [];

          // Filter by HTLC status (Spark or Lightning)
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
            const placeholders = paymentDetailsFilter.htlcStatus.map(
              () => `$${paramIdx++}`
            );
            if (htlcAlias === "l") {
              paymentDetailsClauses.push(
                `l.htlc_status IN (${placeholders.join(", ")})`
              );
            } else {
              paymentDetailsClauses.push(
                `s.htlc_details::jsonb->>'status' IN (${placeholders.join(", ")})`
              );
            }
            params.push(...paymentDetailsFilter.htlcStatus);
          }

          // Filter by conversion refund needed
          if (
            (paymentDetailsFilter.type === "spark" ||
              paymentDetailsFilter.type === "token") &&
            paymentDetailsFilter.conversionRefundNeeded !== undefined
          ) {
            const typeCheck =
              paymentDetailsFilter.type === "spark"
                ? "p.spark = true"
                : "p.spark IS NULL";
            const refundNeeded =
              paymentDetailsFilter.conversionRefundNeeded === true
                ? "= 'refundNeeded'"
                : "!= 'refundNeeded'";
            paymentDetailsClauses.push(
              `${typeCheck} AND pm.conversion_info IS NOT NULL AND
               pm.conversion_info::jsonb->>'status' ${refundNeeded}`
            );
          }

          // Filter by token transaction hash
          if (
            paymentDetailsFilter.type === "token" &&
            paymentDetailsFilter.txHash !== undefined
          ) {
            paymentDetailsClauses.push(`t.tx_hash = $${paramIdx++}`);
            params.push(paymentDetailsFilter.txHash);
          }

          // Filter by token transaction type
          if (
            paymentDetailsFilter.type === "token" &&
            paymentDetailsFilter.txType !== undefined
          ) {
            paymentDetailsClauses.push(`t.tx_type = $${paramIdx++}`);
            params.push(paymentDetailsFilter.txType);
          }

          if (paymentDetailsClauses.length > 0) {
            allPaymentDetailsClauses.push(
              `(${paymentDetailsClauses.join(" AND ")})`
            );
          }
        }

        if (allPaymentDetailsClauses.length > 0) {
          whereClauses.push(
            `(${allPaymentDetailsClauses.join(" OR ")})`
          );
        }
      }

      // Filter by asset
      if (request.assetFilter) {
        const assetFilter = request.assetFilter;
        if (assetFilter.type === "bitcoin") {
          whereClauses.push("t.metadata IS NULL");
        } else if (assetFilter.type === "token") {
          whereClauses.push("t.metadata IS NOT NULL");
          if (assetFilter.tokenIdentifier) {
            whereClauses.push(
              `t.metadata::jsonb->>'identifier' = $${paramIdx++}`
            );
            params.push(assetFilter.tokenIdentifier);
          }
        }
      }

      // Exclude child payments
      whereClauses.push("pm.parent_payment_id IS NULL");

      const whereSql =
        whereClauses.length > 0
          ? `WHERE ${whereClauses.join(" AND ")}`
          : "";

      const orderDirection = request.sortAscending ? "ASC" : "DESC";
      const query = `${SELECT_PAYMENT_SQL} ${whereSql} ORDER BY p.timestamp ${orderDirection} LIMIT $${paramIdx++} OFFSET $${paramIdx++}`;

      params.push(actualLimit, actualOffset);
      const result = await this.pool.query(query, params);
      return result.rows.map(this._rowToPayment.bind(this));
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

      await this._withTransaction(async (client) => {
        const withdrawTxId =
          payment.details?.type === "withdraw" ? payment.details.txId : null;
        const depositTxId =
          payment.details?.type === "deposit" ? payment.details.txId : null;
        const spark = payment.details?.type === "spark" ? true : null;

        await client.query(
          `INSERT INTO payments (id, payment_type, status, amount, fees, timestamp, method, withdraw_tx_id, deposit_tx_id, spark)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
           ON CONFLICT(id) DO UPDATE SET
             payment_type=EXCLUDED.payment_type,
             status=EXCLUDED.status,
             amount=EXCLUDED.amount,
             fees=EXCLUDED.fees,
             timestamp=EXCLUDED.timestamp,
             method=EXCLUDED.method,
             withdraw_tx_id=EXCLUDED.withdraw_tx_id,
             deposit_tx_id=EXCLUDED.deposit_tx_id,
             spark=EXCLUDED.spark`,
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
          await client.query(
            `INSERT INTO payment_details_spark (payment_id, invoice_details, htlc_details)
             VALUES ($1, $2, $3)
             ON CONFLICT(payment_id) DO UPDATE SET
               invoice_details=COALESCE(EXCLUDED.invoice_details, payment_details_spark.invoice_details),
               htlc_details=COALESCE(EXCLUDED.htlc_details, payment_details_spark.htlc_details)`,
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
          await client.query(
            `INSERT INTO payment_details_lightning
              (payment_id, invoice, payment_hash, destination_pubkey, description, preimage, htlc_status, htlc_expiry_time)
              VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
              ON CONFLICT(payment_id) DO UPDATE SET
                invoice=EXCLUDED.invoice,
                payment_hash=EXCLUDED.payment_hash,
                destination_pubkey=EXCLUDED.destination_pubkey,
                description=EXCLUDED.description,
                preimage=COALESCE(EXCLUDED.preimage, payment_details_lightning.preimage),
                htlc_status=COALESCE(EXCLUDED.htlc_status, payment_details_lightning.htlc_status),
                htlc_expiry_time=COALESCE(EXCLUDED.htlc_expiry_time, payment_details_lightning.htlc_expiry_time)`,
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
          await client.query(
            `INSERT INTO payment_details_token
              (payment_id, metadata, tx_hash, tx_type, invoice_details)
              VALUES ($1, $2, $3, $4, $5)
              ON CONFLICT(payment_id) DO UPDATE SET
                metadata=EXCLUDED.metadata,
                tx_hash=EXCLUDED.tx_hash,
                tx_type=EXCLUDED.tx_type,
                invoice_details=COALESCE(EXCLUDED.invoice_details, payment_details_token.invoice_details)`,
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

      const result = await this.pool.query(
        `${SELECT_PAYMENT_SQL} WHERE p.id = $1`,
        [id]
      );

      if (result.rows.length === 0) {
        throw new StorageError(`Payment with id '${id}' not found`);
      }

      return this._rowToPayment(result.rows[0]);
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

      const result = await this.pool.query(
        `${SELECT_PAYMENT_SQL} WHERE l.invoice = $1`,
        [invoice]
      );

      if (result.rows.length === 0) {
        return null;
      }

      return this._rowToPayment(result.rows[0]);
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

      // Early exit if no related payments exist
      const hasRelatedResult = await this.pool.query(
        "SELECT EXISTS(SELECT 1 FROM payment_metadata WHERE parent_payment_id IS NOT NULL LIMIT 1)"
      );
      if (!hasRelatedResult.rows[0].exists) {
        return {};
      }

      const placeholders = parentPaymentIds.map(
        (_, i) => `$${i + 1}`
      );
      const query = `${SELECT_PAYMENT_SQL} WHERE pm.parent_payment_id IN (${placeholders.join(", ")}) ORDER BY p.timestamp ASC`;

      const queryResult = await this.pool.query(
        query,
        parentPaymentIds
      );

      const result = {};
      for (const row of queryResult.rows) {
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
        `INSERT INTO payment_metadata (payment_id, parent_payment_id, lnurl_pay_info, lnurl_withdraw_info, lnurl_description, conversion_info)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT(payment_id) DO UPDATE SET
           parent_payment_id = COALESCE(EXCLUDED.parent_payment_id, payment_metadata.parent_payment_id),
           lnurl_pay_info = COALESCE(EXCLUDED.lnurl_pay_info, payment_metadata.lnurl_pay_info),
           lnurl_withdraw_info = COALESCE(EXCLUDED.lnurl_withdraw_info, payment_metadata.lnurl_withdraw_info),
           lnurl_description = COALESCE(EXCLUDED.lnurl_description, payment_metadata.lnurl_description),
           conversion_info = COALESCE(EXCLUDED.conversion_info, payment_metadata.conversion_info)`,
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

  async addDeposit(txid, vout, amountSats) {
    try {
      await this.pool.query(
        `INSERT INTO unclaimed_deposits (txid, vout, amount_sats)
         VALUES ($1, $2, $3)
         ON CONFLICT(txid, vout) DO NOTHING`,
        [txid, vout, amountSats]
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
        "DELETE FROM unclaimed_deposits WHERE txid = $1 AND vout = $2",
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
      const result = await this.pool.query(
        "SELECT txid, vout, amount_sats, claim_error, refund_tx, refund_tx_id FROM unclaimed_deposits"
      );

      return result.rows.map((row) => ({
        txid: row.txid,
        vout: row.vout,
        amountSats: row.amount_sats != null ? BigInt(row.amount_sats) : BigInt(0),
        claimError: row.claim_error || null,
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
           SET claim_error = $1, refund_tx = NULL, refund_tx_id = NULL
           WHERE txid = $2 AND vout = $3`,
          [JSON.stringify(payload.error), txid, vout]
        );
      } else if (payload.type === "refund") {
        await this.pool.query(
          `UPDATE unclaimed_deposits
           SET refund_tx = $1, refund_tx_id = $2, claim_error = NULL
           WHERE txid = $3 AND vout = $4`,
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
      await this._withTransaction(async (client) => {
        for (const item of metadata) {
          await client.query(
            `INSERT INTO lnurl_receive_metadata (payment_hash, nostr_zap_request, nostr_zap_receipt, sender_comment)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT(payment_hash) DO UPDATE SET
               nostr_zap_request = EXCLUDED.nostr_zap_request,
               nostr_zap_receipt = EXCLUDED.nostr_zap_receipt,
               sender_comment = EXCLUDED.sender_comment`,
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
              expiryTime:
                Number(row.lightning_htlc_expiry_time) ?? 0,
              status: row.lightning_htlc_status,
            }
          : (() => {
              throw new StorageError(
                `htlc_status is required for Lightning payment ${row.id}`
              );
            })(),
      };

      if (row.lnurl_pay_info) {
        details.lnurlPayInfo =
          typeof row.lnurl_pay_info === "string"
            ? JSON.parse(row.lnurl_pay_info)
            : row.lnurl_pay_info;
      }

      if (row.lnurl_withdraw_info) {
        details.lnurlWithdrawInfo =
          typeof row.lnurl_withdraw_info === "string"
            ? JSON.parse(row.lnurl_withdraw_info)
            : row.lnurl_withdraw_info;
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
    } else if (row.spark) {
      details = {
        type: "spark",
        invoiceDetails: row.spark_invoice_details
          ? typeof row.spark_invoice_details === "string"
            ? JSON.parse(row.spark_invoice_details)
            : row.spark_invoice_details
          : null,
        htlcDetails: row.spark_htlc_details
          ? typeof row.spark_htlc_details === "string"
            ? JSON.parse(row.spark_htlc_details)
            : row.spark_htlc_details
          : null,
        conversionInfo: row.conversion_info
          ? typeof row.conversion_info === "string"
            ? JSON.parse(row.conversion_info)
            : row.conversion_info
          : null,
      };
    } else if (row.token_metadata) {
      details = {
        type: "token",
        metadata:
          typeof row.token_metadata === "string"
            ? JSON.parse(row.token_metadata)
            : row.token_metadata,
        txHash: row.token_tx_hash,
        txType: row.token_tx_type,
        invoiceDetails: row.token_invoice_details
          ? typeof row.token_invoice_details === "string"
            ? JSON.parse(row.token_invoice_details)
            : row.token_invoice_details
          : null,
        conversionInfo: row.conversion_info
          ? typeof row.conversion_info === "string"
            ? JSON.parse(row.conversion_info)
            : row.conversion_info
          : null,
      };
    }

    let method = null;
    if (row.method) {
      try {
        method =
          typeof row.method === "string"
            ? JSON.parse(row.method)
            : row.method;
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
      conversionDetails: null,
    };
  }

  // ===== Contact Operations =====

  async listContacts(request) {
    try {
      const offset = request.offset != null ? request.offset : 0;
      const limit = request.limit != null ? request.limit : 4294967295;

      const result = await this.pool.query(
        `SELECT id, name, payment_identifier, created_at, updated_at
         FROM contacts
         ORDER BY name ASC
         LIMIT $1 OFFSET $2`,
        [limit, offset]
      );

      return result.rows.map((row) => ({
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
      const result = await this.pool.query(
        `SELECT id, name, payment_identifier, created_at, updated_at
         FROM contacts
         WHERE id = $1`,
        [id]
      );

      if (result.rows.length === 0) {
        return null;
      }

      const row = result.rows[0];
      return {
        id: row.id,
        name: row.name,
        paymentIdentifier: row.payment_identifier,
        createdAt: Number(row.created_at),
        updatedAt: Number(row.updated_at),
      };
    } catch (error) {
      throw new StorageError(
        `Failed to get contact: ${error.message}`,
        error
      );
    }
  }

  async insertContact(contact) {
    try {
      await this.pool.query(
        `INSERT INTO contacts (id, name, payment_identifier, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT(id) DO UPDATE SET
           name = EXCLUDED.name,
           payment_identifier = EXCLUDED.payment_identifier,
           updated_at = EXCLUDED.updated_at`,
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
      await this.pool.query("DELETE FROM contacts WHERE id = $1", [id]);
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
      return await this._withTransaction(async (client) => {
        const revisionResult = await client.query(
          "SELECT COALESCE(MAX(revision), 0) + 1 AS revision FROM sync_outgoing"
        );
        const revision = BigInt(revisionResult.rows[0].revision);

        await client.query(
          `INSERT INTO sync_outgoing (
            record_type,
            data_id,
            schema_version,
            commit_time,
            updated_fields_json,
            revision
          ) VALUES ($1, $2, $3, $4, $5, $6)`,
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
      await this._withTransaction(async (client) => {
        const deleteResult = await client.query(
          "DELETE FROM sync_outgoing WHERE record_type = $1 AND data_id = $2 AND revision = $3",
          [
            record.id.type,
            record.id.dataId,
            localRevision.toString(),
          ]
        );

        if (deleteResult.rowCount === 0) {
          const msg = `complete_outgoing_sync: DELETE from sync_outgoing matched 0 rows (type=${record.id.type}, data_id=${record.id.dataId}, revision=${localRevision})`;
          if (this.logger && typeof this.logger.log === "function") {
            this.logger.log({ line: msg, level: "warn" });
          } else {
            console.warn(`[PostgresStorage] ${msg}`);
          }
        }

        await client.query(
          `INSERT INTO sync_state (
            record_type,
            data_id,
            revision,
            schema_version,
            commit_time,
            data
          ) VALUES ($1, $2, $3, $4, $5, $6)
          ON CONFLICT(record_type, data_id) DO UPDATE SET
            schema_version = EXCLUDED.schema_version,
            commit_time = EXCLUDED.commit_time,
            data = EXCLUDED.data,
            revision = EXCLUDED.revision`,
          [
            record.id.type,
            record.id.dataId,
            record.revision.toString(),
            record.schemaVersion,
            Math.floor(Date.now() / 1000),
            JSON.stringify(record.data),
          ]
        );

        await client.query(
          "UPDATE sync_revision SET revision = GREATEST(revision, $1)",
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
      const result = await this.pool.query(
        `SELECT
          o.record_type,
          o.data_id,
          o.schema_version,
          o.commit_time,
          o.updated_fields_json,
          o.revision,
          e.schema_version as existing_schema_version,
          e.commit_time as existing_commit_time,
          e.data as existing_data,
          e.revision as existing_revision
        FROM sync_outgoing o
        LEFT JOIN sync_state e ON
          o.record_type = e.record_type AND
          o.data_id = e.data_id
        ORDER BY o.revision ASC
        LIMIT $1`,
        [limit]
      );

      return result.rows.map((row) => {
        const change = {
          id: {
            type: row.record_type,
            dataId: row.data_id,
          },
          schemaVersion: row.schema_version,
          updatedFields:
            typeof row.updated_fields_json === "string"
              ? JSON.parse(row.updated_fields_json)
              : row.updated_fields_json,
          localRevision: BigInt(row.revision),
        };

        let parent = null;
        if (row.existing_data) {
          parent = {
            id: {
              type: row.record_type,
              dataId: row.data_id,
            },
            revision: BigInt(row.existing_revision),
            schemaVersion: row.existing_schema_version,
            data:
              typeof row.existing_data === "string"
                ? JSON.parse(row.existing_data)
                : row.existing_data,
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
      const result = await this.pool.query(
        "SELECT revision FROM sync_revision"
      );
      return result.rows.length > 0
        ? BigInt(result.rows[0].revision)
        : BigInt(0);
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

      await this._withTransaction(async (client) => {
        for (const record of records) {
          await client.query(
            `INSERT INTO sync_incoming (
              record_type,
              data_id,
              schema_version,
              commit_time,
              data,
              revision
            ) VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT(record_type, data_id, revision) DO UPDATE SET
              schema_version = EXCLUDED.schema_version,
              commit_time = EXCLUDED.commit_time,
              data = EXCLUDED.data`,
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
         WHERE record_type = $1
         AND data_id = $2
         AND revision = $3`,
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
      const result = await this.pool.query(
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
         LIMIT $1`,
        [limit]
      );

      return result.rows.map((row) => {
        const newState = {
          id: {
            type: row.record_type,
            dataId: row.data_id,
          },
          revision: BigInt(row.revision),
          schemaVersion: row.schema_version,
          data:
            typeof row.data === "string"
              ? JSON.parse(row.data)
              : row.data,
        };

        let oldState = null;
        if (row.existing_data) {
          oldState = {
            id: {
              type: row.record_type,
              dataId: row.data_id,
            },
            revision: BigInt(row.existing_revision),
            schemaVersion: row.existing_schema_version,
            data:
              typeof row.existing_data === "string"
                ? JSON.parse(row.existing_data)
                : row.existing_data,
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
      const result = await this.pool.query(
        `SELECT
          o.record_type,
          o.data_id,
          o.schema_version,
          o.commit_time,
          o.updated_fields_json,
          o.revision,
          e.schema_version as existing_schema_version,
          e.commit_time as existing_commit_time,
          e.data as existing_data,
          e.revision as existing_revision
        FROM sync_outgoing o
        LEFT JOIN sync_state e ON
          o.record_type = e.record_type AND
          o.data_id = e.data_id
        ORDER BY o.revision DESC
        LIMIT 1`
      );

      if (result.rows.length === 0) {
        return null;
      }

      const row = result.rows[0];

      const change = {
        id: {
          type: row.record_type,
          dataId: row.data_id,
        },
        schemaVersion: row.schema_version,
        updatedFields:
          typeof row.updated_fields_json === "string"
            ? JSON.parse(row.updated_fields_json)
            : row.updated_fields_json,
        localRevision: BigInt(row.revision),
      };

      let parent = null;
      if (row.existing_data) {
        parent = {
          id: {
            type: row.record_type,
            dataId: row.data_id,
          },
          revision: BigInt(row.existing_revision),
          schemaVersion: row.existing_schema_version,
          data:
            typeof row.existing_data === "string"
              ? JSON.parse(row.existing_data)
              : row.existing_data,
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
      await this._withTransaction(async (client) => {
        await client.query(
          `INSERT INTO sync_state (
            record_type,
            data_id,
            revision,
            schema_version,
            commit_time,
            data
          ) VALUES ($1, $2, $3, $4, $5, $6)
          ON CONFLICT(record_type, data_id) DO UPDATE SET
            schema_version = EXCLUDED.schema_version,
            commit_time = EXCLUDED.commit_time,
            data = EXCLUDED.data,
            revision = EXCLUDED.revision`,
          [
            record.id.type,
            record.id.dataId,
            record.revision.toString(),
            record.schemaVersion,
            Math.floor(Date.now() / 1000),
            JSON.stringify(record.data),
          ]
        );

        await client.query(
          "UPDATE sync_revision SET revision = GREATEST(revision, $1)",
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
 * Creates a PostgresStorageConfig with the given connection string and default pool settings.
 *
 * Default values (from pg.Pool):
 * - maxPoolSize: 10
 * - createTimeoutSecs: 0 (no timeout)
 * - recycleTimeoutSecs: 10 (10 seconds idle before disconnect)
 *
 * @param {string} connectionString - PostgreSQL connection string
 * @returns {object} PostgreSQL storage configuration
 */
function defaultPostgresStorageConfig(connectionString) {
  return {
    connectionString,
    maxPoolSize: 10,
    createTimeoutSecs: 0,
    recycleTimeoutSecs: 10,
  };
}

/**
 * Create a PostgresStorage instance from a config object.
 * Use {@link defaultPostgresStorageConfig} to create a config with sensible defaults.
 *
 * @param {object} config - PostgreSQL configuration (from defaultPostgresStorageConfig)
 * @param {string} config.connectionString - PostgreSQL connection string
 * @param {number} config.maxPoolSize - Maximum number of connections in the pool
 * @param {number} config.createTimeoutSecs - Timeout in seconds for establishing a new connection
 * @param {number} config.recycleTimeoutSecs - Timeout in seconds before recycling an idle connection
 * @param {object} [logger] - Optional logger
 * @returns {Promise<PostgresStorage>}
 */
async function createPostgresStorage(config, logger = null) {
  const pool = new pg.Pool({
    connectionString: config.connectionString,
    max: config.maxPoolSize,
    connectionTimeoutMillis: config.createTimeoutSecs * 1000,
    idleTimeoutMillis: config.recycleTimeoutSecs * 1000,
  });

  const storage = new PostgresStorage(pool, logger);
  await storage.initialize();
  return storage;
}

module.exports = { PostgresStorage, createPostgresStorage, defaultPostgresStorageConfig, StorageError };
