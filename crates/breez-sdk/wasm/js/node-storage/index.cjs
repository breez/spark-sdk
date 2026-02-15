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
           pm.parent_payment_id
      FROM payments p
      LEFT JOIN payment_details_lightning l ON p.id = l.payment_id
      LEFT JOIN payment_details_token t ON p.id = t.payment_id
      LEFT JOIN payment_details_spark s ON p.id = s.payment_id
      LEFT JOIN payment_metadata pm ON p.id = pm.payment_id
      LEFT JOIN lnurl_receive_metadata lrm ON l.payment_hash = lrm.payment_hash`;

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

      // Filter by payment details. If any filter matches, we include the payment
      if (request.paymentDetailsFilter && request.paymentDetailsFilter.length > 0) {
        const allPaymentDetailsClauses = [];
        for (const paymentDetailsFilter of request.paymentDetailsFilter) {
          const paymentDetailsClauses = [];
          // Filter by Spark HTLC status
          if (
            paymentDetailsFilter.type === "spark" &&
            paymentDetailsFilter.htlcStatus !== undefined &&
            paymentDetailsFilter.htlcStatus.length > 0
          ) {
            const placeholders = paymentDetailsFilter.htlcStatus
              .map(() => "?")
              .join(", ");
            paymentDetailsClauses.push(
              `json_extract(s.htlc_details, '$.status') IN (${placeholders})`
            );
            params.push(...paymentDetailsFilter.htlcStatus);
          }
          // Filter by token conversion info presence
          if (
            (paymentDetailsFilter.type === "spark" || paymentDetailsFilter.type === "token") &&
              paymentDetailsFilter.conversionRefundNeeded !== undefined
          ) {
            const typeCheck = paymentDetailsFilter.type === "spark" ? "p.spark = 1" : "p.spark IS NULL";
            const refundNeeded =
              paymentDetailsFilter.conversionRefundNeeded === true
                ? "= 'refundNeeded'"
                : "!= 'refundNeeded'";
            paymentDetailsClauses.push(
              `${typeCheck} AND pm.conversion_info IS NOT NULL AND
              json_extract(pm.conversion_info, '$.status') ${refundNeeded}`
            );
          }
          // Filter by token transaction hash
          if (
            paymentDetailsFilter.type === "token" &&
            paymentDetailsFilter.txHash !== undefined
          ) {
            paymentDetailsClauses.push("t.tx_hash = ?");
            params.push(paymentDetailsFilter.txHash);
          }
          // Filter by token transaction type
          if (
            paymentDetailsFilter.type === "token" &&
            paymentDetailsFilter.txType !== undefined
          ) {
            paymentDetailsClauses.push("t.tx_type = ?");
            params.push(paymentDetailsFilter.txType);
          }

          if (paymentDetailsClauses.length > 0) {
            allPaymentDetailsClauses.push(`(${paymentDetailsClauses.join(" AND ")})`);
          }
        }

        if (allPaymentDetailsClauses.length > 0) {
          whereClauses.push(`(${allPaymentDetailsClauses.join(" OR ")})`);
        }
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

      // Exclude child payments (those with a parent_payment_id)
      whereClauses.push("pm.parent_payment_id IS NULL");

      // Build the WHERE clause
      const whereSql =
        whereClauses.length > 0 ? `WHERE ${whereClauses.join(" AND ")}` : "";

      // Determine sort order
      const orderDirection = request.sortAscending ? "ASC" : "DESC";
      const query = `${SELECT_PAYMENT_SQL} ${whereSql} ORDER BY p.timestamp ${orderDirection} LIMIT ? OFFSET ?`;

      params.push(actualLimit, actualOffset);
      const stmt = this.db.prepare(query);
      const rows = stmt.all(...params);
      const payments = rows.map(this._rowToPayment.bind(this));
      return Promise.resolve(payments);
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
        `INSERT INTO payments (id, payment_type, status, amount, fees, timestamp, method, withdraw_tx_id, deposit_tx_id, spark) 
         VALUES (@id, @paymentType, @status, @amount, @fees, @timestamp, @method, @withdrawTxId, @depositTxId, @spark)
         ON CONFLICT(id) DO UPDATE SET
           payment_type=excluded.payment_type,
           status=excluded.status,
           amount=excluded.amount,
           fees=excluded.fees,
           timestamp=excluded.timestamp,
           method=excluded.method,
           withdraw_tx_id=excluded.withdraw_tx_id,
           deposit_tx_id=excluded.deposit_tx_id,
           spark=excluded.spark`
      );
      const lightningInsert = this.db.prepare(
        `INSERT INTO payment_details_lightning 
          (payment_id, invoice, payment_hash, destination_pubkey, description, preimage) 
          VALUES (@id, @invoice, @paymentHash, @destinationPubkey, @description, @preimage)
          ON CONFLICT(payment_id) DO UPDATE SET
            invoice=excluded.invoice,
            payment_hash=excluded.payment_hash,
            destination_pubkey=excluded.destination_pubkey,
            description=excluded.description,
            preimage=excluded.preimage`
      );
      const tokenInsert = this.db.prepare(
        `INSERT INTO payment_details_token 
          (payment_id, metadata, tx_hash, tx_type, invoice_details) 
          VALUES (@id, @metadata, @txHash, @txType, @invoiceDetails)
          ON CONFLICT(payment_id) DO UPDATE SET
            metadata=excluded.metadata,
            tx_hash=excluded.tx_hash,
            tx_type=excluded.tx_type,
            invoice_details=COALESCE(excluded.invoice_details, payment_details_token.invoice_details)`
      );
      const sparkInsert = this.db.prepare(
        `INSERT INTO payment_details_spark 
          (payment_id, invoice_details, htlc_details) 
          VALUES (@id, @invoiceDetails, @htlcDetails)
          ON CONFLICT(payment_id) DO UPDATE SET
            invoice_details=COALESCE(excluded.invoice_details, payment_details_spark.invoice_details),
            htlc_details=COALESCE(excluded.htlc_details, payment_details_spark.htlc_details)`
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
          (payment.details.invoiceDetails != null ||
            payment.details.htlcDetails != null)
        ) {
          sparkInsert.run({
            id: payment.id,
            invoiceDetails: payment.details.invoiceDetails
              ? JSON.stringify(payment.details.invoiceDetails)
              : null,
            htlcDetails: payment.details.htlcDetails
              ? JSON.stringify(payment.details.htlcDetails)
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
            txType: payment.details.txType,
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

      const stmt = this.db.prepare(`${SELECT_PAYMENT_SQL} WHERE p.id = ?`);
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

      const stmt = this.db.prepare(`${SELECT_PAYMENT_SQL} WHERE l.invoice = ?`);
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

  /**
   * Gets payments that have any of the specified parent payment IDs.
   * @param {string[]} parentPaymentIds - Array of parent payment IDs
   * @returns {Promise<Object>} Map of parentPaymentId -> array of RelatedPayment objects
   */
  getPaymentsByParentIds(parentPaymentIds) {
    try {
      if (!parentPaymentIds || parentPaymentIds.length === 0) {
        return Promise.resolve({});
      }

      // Early exit if no related payments exist
      const hasRelated = this.db
        .prepare(
          "SELECT EXISTS(SELECT 1 FROM payment_metadata WHERE parent_payment_id IS NOT NULL LIMIT 1)"
        )
        .pluck()
        .get();
      if (!hasRelated) {
        return Promise.resolve({});
      }

      const placeholders = parentPaymentIds.map(() => "?").join(", ");
      const query = `${SELECT_PAYMENT_SQL} WHERE pm.parent_payment_id IN (${placeholders}) ORDER BY p.timestamp ASC`;

      const stmt = this.db.prepare(query);
      const rows = stmt.all(...parentPaymentIds);

      // Group payments by parent_payment_id
      const result = {};
      for (const row of rows) {
        const parentId = row.parent_payment_id;
        if (!result[parentId]) {
          result[parentId] = [];
        }
        result[parentId].push(this._rowToPayment(row));
      }

      return Promise.resolve(result);
    } catch (error) {
      return Promise.reject(
        new StorageError(
          `Failed to get payments by parent ids: ${error.message}`,
          error
        )
      );
    }
  }

  insertPaymentMetadata(paymentId, metadata) {
    try {
      const stmt = this.db.prepare(`
                INSERT INTO payment_metadata (payment_id, parent_payment_id, lnurl_pay_info, lnurl_withdraw_info, lnurl_description, conversion_info)
                VALUES (?, ?, ?, ?, ?, ?)
                ON CONFLICT(payment_id) DO UPDATE SET
                    parent_payment_id = COALESCE(excluded.parent_payment_id, parent_payment_id),
                    lnurl_pay_info = COALESCE(excluded.lnurl_pay_info, lnurl_pay_info),
                    lnurl_withdraw_info = COALESCE(excluded.lnurl_withdraw_info, lnurl_withdraw_info),
                    lnurl_description = COALESCE(excluded.lnurl_description, lnurl_description),
                    conversion_info = COALESCE(excluded.conversion_info, conversion_info)
            `);

      stmt.run(
        paymentId,
        metadata.parentPaymentId,
        metadata.lnurlPayInfo ? JSON.stringify(metadata.lnurlPayInfo) : null,
        metadata.lnurlWithdrawInfo
          ? JSON.stringify(metadata.lnurlWithdrawInfo)
          : null,
        metadata.lnurlDescription,
        metadata.conversionInfo
          ? JSON.stringify(metadata.conversionInfo)
          : null
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

  setLnurlMetadata(metadata) {
    try {
      const stmt = this.db.prepare(
        "INSERT OR REPLACE INTO lnurl_receive_metadata (payment_hash, nostr_zap_request, nostr_zap_receipt, sender_comment) VALUES (?, ?, ?, ?)"
      );

      const transaction = this.db.transaction(() => {
        for (const item of metadata) {
          stmt.run(
            item.paymentHash,
            item.nostrZapRequest || null,
            item.nostrZapReceipt || null,
            item.senderComment || null
          );
        }
      });

      transaction();
      return Promise.resolve();
    } catch (error) {
      return Promise.reject(
        new StorageError(
          `Failed to add lnurl metadata: ${error.message}`,
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

      if (row.lnurl_nostr_zap_request || row.lnurl_sender_comment) {
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
          ? JSON.parse(row.spark_invoice_details)
          : null,
        htlcDetails: row.spark_htlc_details
          ? JSON.parse(row.spark_htlc_details)
          : null,
        conversionInfo: row.conversion_info
          ? JSON.parse(row.conversion_info)
          : null,
      };
    } else if (row.token_metadata) {
      details = {
        type: "token",
        metadata: JSON.parse(row.token_metadata),
        txHash: row.token_tx_hash,
        txType: row.token_tx_type,
        invoiceDetails: row.token_invoice_details
          ? JSON.parse(row.token_invoice_details)
          : null,
        conversionInfo: row.conversion_info
          ? JSON.parse(row.conversion_info)
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

  // ===== Sync Operations =====

  syncAddOutgoingChange(record) {
    try {
      const transaction = this.db.transaction(() => {
        // This revision is a local queue id for pending rows, not a server revision.
        const revisionQuery = this.db.prepare(`
          SELECT CAST(COALESCE(MAX(revision), 0) + 1 AS TEXT) AS revision
          FROM sync_outgoing
        `);
        const revision = BigInt(revisionQuery.get().revision);

        // Insert the record
        const stmt = this.db.prepare(`
          INSERT INTO sync_outgoing (
            record_type, 
            data_id, 
            schema_version,
            commit_time,
            updated_fields_json, 
            revision
          ) VALUES (?, ?, ?, ?, ?, CAST(? AS INTEGER))
        `);

        stmt.run(
          record.id.type,
          record.id.dataId,
          record.schemaVersion,
          Math.floor(Date.now() / 1000),
          JSON.stringify(record.updatedFields),
          revision.toString()
        );

        return revision;
      });

      return Promise.resolve(transaction());
    } catch (error) {
      return Promise.reject(
        new StorageError(
          `Failed to add outgoing change: ${error.message}`,
          error
        )
      );
    }
  }

  syncCompleteOutgoingSync(record, localRevision) {
    try {
      const transaction = this.db.transaction(() => {
        // Delete records that have been synced
        const deleteStmt = this.db.prepare(`
          DELETE FROM sync_outgoing
          WHERE record_type = ? AND data_id = ? AND revision = CAST(? AS INTEGER)
        `);

        deleteStmt.run(
          record.id.type,
          record.id.dataId,
          localRevision.toString()
        );

        // Update or insert the sync state
        const updateStateStmt = this.db.prepare(`
          INSERT OR REPLACE INTO sync_state (
            record_type, 
            data_id, 
            revision,
            schema_version,
            commit_time,
            data
          ) VALUES (?, ?, CAST(? AS INTEGER), ?, ?, ?)
        `);

        updateStateStmt.run(
          record.id.type,
          record.id.dataId,
          record.revision.toString(),
          record.schemaVersion,
          Math.floor(Date.now() / 1000),
          JSON.stringify(record.data)
        );

        // Update sync_revision to track the highest known revision
        const updateRevisionStmt = this.db.prepare(`
          UPDATE sync_revision SET revision = MAX(revision, CAST(? AS INTEGER))
        `);
        updateRevisionStmt.run(record.revision.toString());
      });

      transaction();
      return Promise.resolve();
    } catch (error) {
      return Promise.reject(
        new StorageError(
          `Failed to complete outgoing sync: ${error.message}`,
          error
        )
      );
    }
  }

  syncGetPendingOutgoingChanges(limit) {
    try {
      const stmt = this.db.prepare(`
        SELECT 
          o.record_type, 
          o.data_id, 
          o.schema_version,
          o.commit_time,
          o.updated_fields_json, 
          CAST(o.revision AS TEXT) as revision,
          e.schema_version as existing_schema_version,
          e.commit_time as existing_commit_time,
          e.data as existing_data,
          CAST(e.revision AS TEXT) as existing_revision
        FROM sync_outgoing o
        LEFT JOIN sync_state e ON 
          o.record_type = e.record_type AND 
          o.data_id = e.data_id
        ORDER BY o.revision ASC
        LIMIT ?
      `);

      const rows = stmt.all(limit);

      const changes = rows.map((row) => {
        const change = {
          id: {
            type: row.record_type,
            dataId: row.data_id,
          },
          schemaVersion: row.schema_version,
          updatedFields: JSON.parse(row.updated_fields_json),
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
            data: JSON.parse(row.existing_data),
          };
        }

        return {
          change,
          parent,
        };
      });

      return Promise.resolve(changes);
    } catch (error) {
      return Promise.reject(
        new StorageError(
          `Failed to get pending outgoing changes: ${error.message}`,
          error
        )
      );
    }
  }

  syncGetLastRevision() {
    try {
      const stmt = this.db.prepare(
        `SELECT CAST(revision AS TEXT) as revision FROM sync_revision`
      );
      const row = stmt.get();

      return Promise.resolve(row ? BigInt(row.revision) : BigInt(0));
    } catch (error) {
      return Promise.reject(
        new StorageError(`Failed to get last revision: ${error.message}`, error)
      );
    }
  }

  syncInsertIncomingRecords(records) {
    try {
      if (!records || records.length === 0) {
        return Promise.resolve();
      }

      const transaction = this.db.transaction(() => {
        const stmt = this.db.prepare(`
          INSERT OR REPLACE INTO sync_incoming (
            record_type, 
            data_id,
            schema_version,
            commit_time,
            data,
            revision
          ) VALUES (?, ?, ?, ?, ?, CAST(? AS INTEGER))
        `);

        for (const record of records) {
          stmt.run(
            record.id.type,
            record.id.dataId,
            record.schemaVersion,
            Math.floor(Date.now() / 1000),
            JSON.stringify(record.data),
            record.revision.toString()
          );
        }
      });

      transaction();
      return Promise.resolve();
    } catch (error) {
      return Promise.reject(
        new StorageError(
          `Failed to insert incoming records: ${error.message}`,
          error
        )
      );
    }
  }

  syncDeleteIncomingRecord(record) {
    try {
      const stmt = this.db.prepare(`
        DELETE FROM sync_incoming
        WHERE record_type = ? 
        AND data_id = ?
        AND revision = CAST(? AS INTEGER)
      `);

      stmt.run(record.id.type, record.id.dataId, record.revision.toString());
      return Promise.resolve();
    } catch (error) {
      return Promise.reject(
        new StorageError(
          `Failed to delete incoming record: ${error.message}`,
          error
        )
      );
    }
  }

  syncGetIncomingRecords(limit) {
    try {
      const transaction = this.db.transaction(() => {
        // Get records and then delete them (following the SQLite pattern)
        const stmt = this.db.prepare(`
          SELECT  i.record_type
          ,       i.data_id
          ,       i.schema_version
          ,       i.data
          ,       CAST(i.revision AS TEXT) AS revision
          ,       e.schema_version AS existing_schema_version
          ,       e.commit_time AS existing_commit_time
          ,       e.data AS existing_data
          ,       CAST(e.revision AS TEXT) AS existing_revision
           FROM sync_incoming i
           LEFT JOIN sync_state e ON i.record_type = e.record_type AND i.data_id = e.data_id
           ORDER BY i.revision ASC
           LIMIT ?
        `);

        const rows = stmt.all(limit);

        // Join with parent records from sync_state
        const results = rows.map((row) => {
          // Create the record
          const newState = {
            id: {
              type: row.record_type,
              dataId: row.data_id,
            },
            revision: BigInt(row.revision),
            schemaVersion: row.schema_version,
            data: JSON.parse(row.data),
          };

          // Create parent if exists
          let oldState = null;
          if (row.existing_data) {
            oldState = {
              id: {
                type: row.record_type,
                dataId: row.data_id,
              },
              revision: BigInt(row.existing_revision),
              schemaVersion: row.existing_schema_version,
              data: JSON.parse(row.existing_data),
            };
          }

          return {
            newState,
            oldState,
          };
        });

        return results;
      });

      return Promise.resolve(transaction());
    } catch (error) {
      return Promise.reject(
        new StorageError(
          `Failed to get incoming records: ${error.message}`,
          error
        )
      );
    }
  }

  syncGetLatestOutgoingChange() {
    try {
      // Get the latest outgoing change
      const stmt = this.db.prepare(`
        SELECT 
          o.record_type, 
          o.data_id, 
          o.schema_version,
          o.commit_time,
          o.updated_fields_json, 
          CAST(o.revision AS TEXT) AS revision,
          e.schema_version as existing_schema_version,
          e.commit_time as existing_commit_time,
          e.data as existing_data,
          CAST(e.revision AS TEXT) AS existing_revision
        FROM sync_outgoing o
        LEFT JOIN sync_state e ON 
          o.record_type = e.record_type AND 
          o.data_id = e.data_id
        ORDER BY o.revision DESC
        LIMIT 1
      `);

      const row = stmt.get();

      if (!row) {
        return Promise.resolve(null);
      }

      const change = {
        id: {
          type: row.record_type,
          dataId: row.data_id,
        },
        schemaVersion: row.schema_version,
        updatedFields: JSON.parse(row.updated_fields_json),
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
          data: JSON.parse(row.existing_data),
        };
      }

      return Promise.resolve({
        change,
        parent,
      });
    } catch (error) {
      return Promise.reject(
        new StorageError(
          `Failed to get latest outgoing change: ${error.message}`,
          error
        )
      );
    }
  }

  syncUpdateRecordFromIncoming(record) {
    try {
      const transaction = this.db.transaction(() => {
        const stmt = this.db.prepare(`
          INSERT OR REPLACE INTO sync_state (
            record_type,
            data_id,
            revision,
            schema_version,
            commit_time,
            data
          ) VALUES (?, ?, CAST(? AS INTEGER), ?, ?, ?)
        `);

        stmt.run(
          record.id.type,
          record.id.dataId,
          record.revision.toString(),
          record.schemaVersion,
          Math.floor(Date.now() / 1000),
          JSON.stringify(record.data)
        );

        // Update sync_revision to track the highest known revision
        const updateRevisionStmt = this.db.prepare(`
          UPDATE sync_revision SET revision = MAX(revision, CAST(? AS INTEGER))
        `);
        updateRevisionStmt.run(record.revision.toString());
      });

      transaction();
      return Promise.resolve();
    } catch (error) {
      return Promise.reject(
        new StorageError(
          `Failed to update record from incoming: ${error.message}`,
          error
        )
      );
    }
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
