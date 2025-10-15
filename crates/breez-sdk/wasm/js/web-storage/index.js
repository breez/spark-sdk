/**
 * ES6 module for Web IndexedDB Storage Implementation
 * This provides an ES6 interface to IndexedDB storage for web browsers
 */

class MigrationManager {
  constructor(db, StorageError, logger = null) {
    this.db = db;
    this.StorageError = StorageError;
    this.logger = logger;
    this.migrations = this._getMigrations();
  }

  /**
   * Handle IndexedDB upgrade event - called during database opening
   */
  handleUpgrade(event, oldVersion, newVersion) {
    const db = event.target.result;
    const transaction = event.target.transaction;

    this._log(
      "info",
      `Upgrading IndexedDB from version ${oldVersion} to ${newVersion}`
    );

    try {
      for (let i = oldVersion; i < newVersion; i++) {
        const migration = this.migrations[i];
        if (migration) {
          this._log("debug", `Running migration ${i + 1}: ${migration.name}`);
          migration.upgrade(db, transaction);
        }
      }
      this._log("info", `Database migration completed successfully`);
    } catch (error) {
      this._log(
        "error",
        `Migration failed at version ${oldVersion}: ${error.message}`
      );
      throw new this.StorageError(
        `Migration failed at version ${oldVersion}: ${error.message}`,
        error
      );
    }
  }

  _log(level, message) {
    if (this.logger && typeof this.logger.log === "function") {
      this.logger.log({
        line: message,
        level: level,
      });
    } else if (level === "error") {
      console.error(`[MigrationManager] ${message}`);
    }
  }

  /**
   * Define all database migrations for IndexedDB
   *
   * Each migration is an object with:
   * - name: Description of the migration
   * - upgrade: Function that takes (db, transaction) and creates/modifies object stores
   */
  _getMigrations() {
    return [
      {
        name: "Create initial object stores",
        upgrade: (db) => {
          // Settings store (key-value cache)
          if (!db.objectStoreNames.contains("settings")) {
            db.createObjectStore("settings", { keyPath: "key" });
          }

          // Payments store
          if (!db.objectStoreNames.contains("payments")) {
            const paymentStore = db.createObjectStore("payments", {
              keyPath: "id",
            });
            paymentStore.createIndex("timestamp", "timestamp", {
              unique: false,
            });
            paymentStore.createIndex("paymentType", "paymentType", {
              unique: false,
            });
            paymentStore.createIndex("status", "status", { unique: false });
          }

          // Payment metadata store
          if (!db.objectStoreNames.contains("payment_metadata")) {
            db.createObjectStore("payment_metadata", { keyPath: "paymentId" });
          }

          // Unclaimed deposits store
          if (!db.objectStoreNames.contains("unclaimed_deposits")) {
            const depositStore = db.createObjectStore("unclaimed_deposits", {
              keyPath: ["txid", "vout"],
            });
            depositStore.createIndex("txid", "txid", { unique: false });
          }
        },
      },
      {
        name: "Create invoice index",
        upgrade: (db, transaction) => {
          const paymentStore = transaction.objectStore("payments");
          if (!paymentStore.indexNames.contains("invoice")) {
            paymentStore.createIndex("invoice", "details.invoice", {
              unique: false,
            });
          }
        },
      },
      {
        name: "Add lnurl receive fields",
        upgrade: (db, transaction) => {
          // LNURL invoices store
          if (!db.objectStoreNames.contains("lnurl_invoices")) {
            const lnurlInvoicesStore = db.createObjectStore("lnurl_invoices", {
              keyPath: "invoice",
            });
          }
        },
      }
    ];
  }
}

class StorageError extends Error {
  constructor(message, cause = null) {
    super(message);
    this.name = "StorageError";
    this.cause = cause;

    // Maintain proper stack trace for where our error was thrown (only available on V8)
    if (Error.captureStackTrace) {
      Error.captureStackTrace(this, StorageError);
    }
  }
}

class IndexedDBStorage {
  constructor(dbName = "BreezSDK", logger = null) {
    this.dbName = dbName;
    this.db = null;
    this.migrationManager = null;
    this.logger = logger;
    this.dbVersion = 3; // Current schema version (aligned with the number of migrations)
  }

  /**
   * Initialize the storage - must be called before using other methods
   */
  async initialize() {
    if (this.db) {
      return this;
    }

    if (typeof window === "undefined" || !window.indexedDB) {
      throw new StorageError("IndexedDB is not available in this environment");
    }

    return new Promise((resolve, reject) => {
      const request = indexedDB.open(this.dbName, this.dbVersion);

      request.onerror = () => {
        const error = new StorageError(
          `Failed to open IndexedDB: ${
            request.error?.message || "Unknown error"
          }`,
          request.error
        );
        reject(error);
      };

      request.onsuccess = () => {
        this.db = request.result;
        this.migrationManager = new MigrationManager(
          this.db,
          StorageError,
          this.logger
        );

        // Handle unexpected version changes
        this.db.onversionchange = () => {
          this.db.close();
          this.db = null;
        };

        resolve(this);
      };

      request.onupgradeneeded = (event) => {
        this.db = event.target.result;
        this.migrationManager = new MigrationManager(
          this.db,
          StorageError,
          this.logger
        );

        try {
          this.migrationManager.handleUpgrade(
            event,
            event.oldVersion,
            event.newVersion
          );
        } catch (error) {
          reject(error);
        }
      };
    });
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

  async getCachedItem(key) {
    if (!this.db) {
      throw new StorageError("Database not initialized");
    }

    return new Promise((resolve, reject) => {
      const transaction = this.db.transaction("settings", "readonly");
      const store = transaction.objectStore("settings");
      const request = store.get(key);

      request.onsuccess = () => {
        const result = request.result;
        resolve(result ? result.value : null);
      };

      request.onerror = () => {
        reject(
          new StorageError(
            `Failed to get cached item '${key}': ${
              request.error?.message || "Unknown error"
            }`,
            request.error
          )
        );
      };
    });
  }

  async setCachedItem(key, value) {
    if (!this.db) {
      throw new StorageError("Database not initialized");
    }

    return new Promise((resolve, reject) => {
      const transaction = this.db.transaction("settings", "readwrite");
      const store = transaction.objectStore("settings");
      const request = store.put({ key, value });

      request.onsuccess = () => resolve();

      request.onerror = () => {
        reject(
          new StorageError(
            `Failed to set cached item '${key}': ${
              request.error?.message || "Unknown error"
            }`,
            request.error
          )
        );
      };
    });
  }

  async deleteCachedItem(key) {
    if (!this.db) {
      throw new StorageError("Database not initialized");
    }

    return new Promise((resolve, reject) => {
      const transaction = this.db.transaction("settings", "readwrite");
      const store = transaction.objectStore("settings");
      const request = store.delete(key);

      request.onsuccess = () => resolve();

      request.onerror = () => {
        reject(
          new StorageError(
            `Failed to delete cached item '${key}': ${
              request.error?.message || "Unknown error"
            }`,
            request.error
          )
        );
      };
    });
  }

  // ===== Payment Operations =====

  async listPayments(offset = null, limit = null) {
    if (!this.db) {
      throw new StorageError("Database not initialized");
    }

    // Handle null values by using default values
    const actualOffset = offset !== null ? offset : 0;
    const actualLimit = limit !== null ? limit : 4294967295; // u32::MAX

    try {
      const transaction = this.db.transaction(
        ["payments", "payment_metadata", "lnurl_invoices"],
        "readonly"
      );
      const paymentStore = transaction.objectStore("payments");
      const metadataStore = transaction.objectStore("payment_metadata");
      const lnurlInvoicesStore = transaction.objectStore("lnurl_invoices");
      
      // Get paginated payments using our helper
      const rawPayments = await this._getPaginatedFromCursor(
        paymentStore.index("timestamp"), 
        actualOffset, 
        actualLimit, 
        "prev"
      );
      
      // Process each payment with metadata and lnurl information
      const enrichedPayments = [];
      
      for (const payment of rawPayments) {
        try {
          if (payment.details) {
            try {
              payment.details = JSON.parse(payment.details);
            } catch (e) {
              // Skip if we can't parse details
              continue;
            }
          }

          if (payment.method) {
            try {
              payment.method = JSON.parse(payment.method);
            } catch (e) {
              // Skip if we can't parse method
              continue;
            }
          }

          // Get metadata for this payment
          const metadata = await this._getFromStore(metadataStore, payment.id).catch(() => null);
          let enrichedPayment = this._mergePaymentMetadata(payment, metadata);
          
          // If there's an invoice, get the lnurl invoice info
          if (payment.details && payment.details.invoice) {
            const lnurlInvoice = await this._getFromStore(lnurlInvoicesStore, payment.details.invoice).catch(() => null);
            if (lnurlInvoice) {
              enrichedPayment = this._mergeLnurlInvoice(enrichedPayment, lnurlInvoice);
            }
          }
          
          enrichedPayments.push(enrichedPayment);
        } catch (error) {
          // If processing a single payment fails, add it without enrichment
          enrichedPayments.push(payment);
        }
      }
      
      return enrichedPayments;
    } catch (error) {
      throw new StorageError(
        `Failed to list payments (offset: ${offset}, limit: ${limit}): ${error.message || "Unknown error"}`,
        error
      );
    }
  }

  async insertPayment(payment) {
    if (!this.db) {
      throw new StorageError("Database not initialized");
    }

    return new Promise((resolve, reject) => {
      const transaction = this.db.transaction("payments", "readwrite");
      const store = transaction.objectStore("payments");

      // Ensure details and method are serialized properly
      const paymentToStore = {
        ...payment,
        details: payment.details ? JSON.stringify(payment.details) : null,
        method: payment.method ? JSON.stringify(payment.method) : null,
      };

      const request = store.put(paymentToStore);
      request.onsuccess = () => resolve();
      request.onerror = () => {
        reject(
          new StorageError(
            `Failed to insert payment '${payment.id}': ${
              request.error?.message || "Unknown error"
            }`,
            request.error
          )
        );
      };
    });
  }

  async getPaymentById(id) {
    if (!this.db) {
      throw new StorageError("Database not initialized");
    }

    try {
      const transaction = this.db.transaction(
        ["payments", "payment_metadata", "lnurl_invoices"],
        "readonly"
      );
      const paymentStore = transaction.objectStore("payments");
      const metadataStore = transaction.objectStore("payment_metadata");
      const lnurlInvoicesStore = transaction.objectStore("lnurl_invoices");

      // Get the payment
      const payment = await this._getFromStore(paymentStore, id);
      if (!payment) {
        throw new StorageError(`Payment with id '${id}' not found`);
      }

      if (payment.details) {
        try {
          payment.details = JSON.parse(payment.details);
        } catch (e) {
          throw new StorageError(
            `Failed to parse payment details JSON for payment ${payment.id}: ${e.message}`,
            e
          );
        }
      }

      if (payment.method) {
        try {
          payment.method = JSON.parse(payment.method);
        } catch (e) {
          throw new StorageError(
            `Failed to parse payment method JSON for payment ${payment.id}: ${e.message}`,
            e
          );
        }
      }

      // Get metadata for this payment
      const metadata = await this._getFromStore(metadataStore, id).catch(() => null);
      let enrichedPayment = this._mergePaymentMetadata(payment, metadata);
      
      // If there's an invoice, get the lnurl invoice info
      if (payment.details && payment.details.invoice) {
        const lnurlInvoice = await this._getFromStore(lnurlInvoicesStore, payment.details.invoice).catch(() => null);
        if (lnurlInvoice) {
          enrichedPayment = this._mergeLnurlInvoice(enrichedPayment, lnurlInvoice);
        }
      }

      return enrichedPayment;
    } catch (error) {
      if (error.name === 'StorageError') {
        throw error;
      }
      throw new StorageError(
        `Failed to get payment by id '${id}': ${error.message || "Unknown error"}`,
        error
      );
    }
  }

  async getPaymentByInvoice(invoice) {
    if (!this.db) {
      throw new StorageError("Database not initialized");
    }

    try {
      const transaction = this.db.transaction(
        ["payments", "payment_metadata", "lnurl_invoices"],
        "readonly"
      );
      const paymentStore = transaction.objectStore("payments");
      const metadataStore = transaction.objectStore("payment_metadata");
      const lnurlInvoicesStore = transaction.objectStore("lnurl_invoices");

      // Get the payment by invoice
      const payment = await this._getFromStore(invoiceIndex, invoice).catch(() => null);
      if (!payment) {
        return null;
      }

      if (payment.details) {
        try {
          payment.details = JSON.parse(payment.details);
        } catch (e) {
          throw new StorageError(
            `Failed to parse payment details JSON for payment ${payment.id}: ${e.message}`,
            e
          );
        }
      }

      if (payment.method) {
        try {
          payment.method = JSON.parse(payment.method);
        } catch (e) {
          throw new StorageError(
            `Failed to parse payment method JSON for payment ${payment.id}: ${e.message}`,
            e
          );
        }
      }

      // Get metadata for this payment
      const metadata = await this._getFromStore(metadataStore, payment.id).catch(() => null);
      let enrichedPayment = this._mergePaymentMetadata(payment, metadata);
      
      // If there's an invoice, get the lnurl invoice info
      if (payment.details && payment.details.invoice) {
        const lnurlInvoice = await this._getFromStore(lnurlInvoicesStore, payment.details.invoice).catch(() => null);
        if (lnurlInvoice) {
          enrichedPayment = this._mergeLnurlInvoice(enrichedPayment, lnurlInvoice);
        }
      }
      
      return enrichedPayment;
    } catch (error) {
      throw new StorageError(
        `Failed to get payment by invoice '${invoice}': ${error.message || "Unknown error"}`,
        error
      );
    }
  }

  async setPaymentMetadata(paymentId, metadata) {
    if (!this.db) {
      throw new StorageError("Database not initialized");
    }

    return new Promise((resolve, reject) => {
      const transaction = this.db.transaction("payment_metadata", "readwrite");
      const store = transaction.objectStore("payment_metadata");

      const metadataToStore = {
        paymentId,
        lnurlPayInfo: metadata.lnurlPayInfo
          ? JSON.stringify(metadata.lnurlPayInfo)
          : null,
        lnurlDescription: metadata.lnurlDescription,
      };

      const request = store.put(metadataToStore);
      request.onsuccess = () => resolve();
      request.onerror = () => {
        reject(
          new StorageError(
            `Failed to set payment metadata for '${paymentId}': ${
              request.error?.message || "Unknown error"
            }`,
            request.error
          )
        );
      };
    });
  }

  // ===== Deposit Operations =====

  async addDeposit(txid, vout, amountSats) {
    if (!this.db) {
      throw new StorageError("Database not initialized");
    }

    return new Promise((resolve, reject) => {
      const transaction = this.db.transaction(
        "unclaimed_deposits",
        "readwrite"
      );
      const store = transaction.objectStore("unclaimed_deposits");

      const depositToStore = {
        txid,
        vout,
        amountSats,
        claimError: null,
        refundTx: null,
        refundTxId: null,
      };

      const request = store.put(depositToStore);
      request.onsuccess = () => resolve();
      request.onerror = () => {
        reject(
          new StorageError(
            `Failed to add deposit '${txid}:${vout}': ${
              request.error?.message || "Unknown error"
            }`,
            request.error
          )
        );
      };
    });
  }

  async deleteDeposit(txid, vout) {
    if (!this.db) {
      throw new StorageError("Database not initialized");
    }

    return new Promise((resolve, reject) => {
      const transaction = this.db.transaction(
        "unclaimed_deposits",
        "readwrite"
      );
      const store = transaction.objectStore("unclaimed_deposits");
      const request = store.delete([txid, vout]);

      request.onsuccess = () => resolve();
      request.onerror = () => {
        reject(
          new StorageError(
            `Failed to delete deposit '${txid}:${vout}': ${
              request.error?.message || "Unknown error"
            }`,
            request.error
          )
        );
      };
    });
  }

  async listDeposits() {
    if (!this.db) {
      throw new StorageError("Database not initialized");
    }

    return new Promise((resolve, reject) => {
      const transaction = this.db.transaction("unclaimed_deposits", "readonly");
      const store = transaction.objectStore("unclaimed_deposits");
      const request = store.getAll();

      request.onsuccess = () => {
        const deposits = request.result.map((row) => ({
          txid: row.txid,
          vout: row.vout,
          amountSats: row.amountSats,
          claimError: row.claimError ? JSON.parse(row.claimError) : null,
          refundTx: row.refundTx,
          refundTxId: row.refundTxId,
        }));
        resolve(deposits);
      };

      request.onerror = () => {
        reject(
          new StorageError(
            `Failed to list deposits: ${
              request.error?.message || "Unknown error"
            }`,
            request.error
          )
        );
      };
    });
  }

  async updateDeposit(txid, vout, payload) {
    if (!this.db) {
      throw new StorageError("Database not initialized");
    }

    return new Promise((resolve, reject) => {
      const transaction = this.db.transaction(
        "unclaimed_deposits",
        "readwrite"
      );
      const store = transaction.objectStore("unclaimed_deposits");

      // First get the existing deposit
      const getRequest = store.get([txid, vout]);

      getRequest.onsuccess = () => {
        const existingDeposit = getRequest.result;
        if (!existingDeposit) {
          // Deposit doesn't exist, just resolve (matches SQLite behavior)
          resolve();
          return;
        }

        let updatedDeposit = { ...existingDeposit };

        if (payload.type === "claimError") {
          updatedDeposit.claimError = JSON.stringify(payload.error);
          updatedDeposit.refundTx = null;
          updatedDeposit.refundTxId = null;
        } else if (payload.type === "refund") {
          updatedDeposit.refundTx = payload.refundTx;
          updatedDeposit.refundTxId = payload.refundTxid;
          updatedDeposit.claimError = null;
        } else {
          reject(new StorageError(`Unknown payload type: ${payload.type}`));
          return;
        }

        const putRequest = store.put(updatedDeposit);
        putRequest.onsuccess = () => resolve();
        putRequest.onerror = () => {
          reject(
            new StorageError(
              `Failed to update deposit '${txid}:${vout}': ${
                putRequest.error?.message || "Unknown error"
              }`,
              putRequest.error
            )
          );
        };
      };

      getRequest.onerror = () => {
        reject(
          new StorageError(
            `Failed to get deposit '${txid}:${vout}' for update: ${
              getRequest.error?.message || "Unknown error"
            }`,
            getRequest.error
          )
        );
      };
    });
  }
  
  async addLnurlInvoices(invoices) {
    if (!this.db) {
      throw new StorageError("Database not initialized");
    }
    
    if (!Array.isArray(invoices) || invoices.length === 0) {
      return; // Nothing to add
    }
    
    return new Promise((resolve, reject) => {
      const transaction = this.db.transaction("lnurl_invoices", "readwrite");
      const store = transaction.objectStore("lnurl_invoices");
      
      let completed = 0;
      let errors = [];
      
      // Process each invoice
      invoices.forEach(invoiceInfo => {
        const request = store.put({
          invoice: invoiceInfo.invoice,
          nostrZapRequest: invoiceInfo.nostrZapRequest || null,
          senderComment: invoiceInfo.senderComment || null
        });
        
        request.onsuccess = () => {
          completed++;
          if (completed === invoices.length) {
            if (errors.length > 0) {
              reject(
                new StorageError(
                  `Failed to add some LNURL invoices: ${errors.join(", ")}`
                )
              );
            } else {
              resolve();
            }
          }
        };
        
        request.onerror = () => {
          completed++;
          errors.push(
            `${invoiceInfo.invoice}: ${request.error?.message || "Unknown error"}`
          );
          
          if (completed === invoices.length) {
            reject(
              new StorageError(
                `Failed to add some LNURL invoices: ${errors.join(", ")}`
              )
            );
          }
        };
      });
      
      transaction.onerror = () => {
        reject(
          new StorageError(
            `Transaction failed while adding LNURL invoices: ${
              transaction.error?.message || "Unknown error"
            }`,
            transaction.error
          )
        );
      };
    });
  }

  // ===== Private Helper Methods =====
  
  // Helper function to wrap IndexedDB get request in a Promise
  _getFromStore(store, key) {
    return new Promise((resolve, reject) => {
      const request = store.get(key);
      
      request.onsuccess = () => {
        resolve(request.result);
      };
      
      request.onerror = () => {
        reject(new StorageError(
          `Failed to get item from store: ${request.error?.message || "Unknown error"}`,
          request.error
        ));
      };
    });
  }
  
  // Helper function for cursor with pagination
  _getPaginatedFromCursor(index, offset = 0, limit = 4294967295, direction = "next", query = null) {
    return new Promise((resolve, reject) => {
      const results = [];
      let count = 0;
      let skipped = 0;
      const request = index.openCursor(query, direction);
      
      request.onsuccess = (event) => {
        const cursor = event.target.result;
        if (!cursor || count >= limit) {
          resolve(results);
          return;
        }
        
        if (skipped < offset) {
          skipped++;
          cursor.continue();
          return;
        }
        
        results.push(cursor.value);
        count++;
        cursor.continue();
      };
      
      request.onerror = () => {
        reject(new StorageError(
          `Failed to iterate cursor with pagination: ${request.error?.message || "Unknown error"}`,
          request.error
        ));
      };
    });
  }

  _mergePaymentMetadata(payment, metadata) {
    if (!metadata || !payment || !payment.details || payment.details.type !== 'lightning') {
      return payment;
    }

    // If this is a Lightning payment and we have lnurl_pay_info, add it to details
    try {
      payment.details.lnurlPayInfo = JSON.parse(metadata.lnurlPayInfo);
      if (metadata.lnurlDescription && !details.description) {
        payment.details.description = metadata.lnurlDescription;
      }
    } catch (e) {
      throw new StorageError(
        `Failed to parse lnurl_pay_info JSON for payment ${payment.id}: ${e.message}`,
        e
      );
    }

    return payment;
  }
  
  _mergeLnurlInvoice(payment, lnurlInvoice) {
    if (!lnurlInvoice || !payment || !payment.details || payment.details.type !== 'lightning') {
      return payment;
    }
    

    // Add LNURL invoice information to payment details
    if (lnurlInvoice.nostrZapRequest || lnurlInvoice.senderComment) {
      payment.details.lnurlReceiveInfo = {};

      payment.details.lnurlReceiveInfo.nostrZapRequest = lnurlInvoice.nostrZapRequest ?? null;
      payment.details.lnurlReceiveInfo.senderComment = lnurlInvoice.senderComment ?? null;
    }

    return payment;
  }
}

export async function createDefaultStorage(
  dbName = "BreezSdkSpark",
  logger = null
) {
  const storage = new IndexedDBStorage(dbName, logger);
  await storage.initialize();
  return storage;
}

export { IndexedDBStorage, StorageError };
