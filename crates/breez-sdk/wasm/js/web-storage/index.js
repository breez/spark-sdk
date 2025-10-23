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
        name: "Convert amount and fees from Number to BigInt for u128 support",
        upgrade: (db, transaction) => {
          const store = transaction.objectStore("payments");
          const getAllRequest = store.getAll();

          getAllRequest.onsuccess = () => {
            const payments = getAllRequest.result;
            let updated = 0;

            payments.forEach((payment) => {
              // Convert amount and fees from Number to BigInt if they're numbers
              let needsUpdate = false;

              if (typeof payment.amount === "number") {
                payment.amount = BigInt(Math.round(payment.amount));
                needsUpdate = true;
              }

              if (typeof payment.fees === "number") {
                payment.fees = BigInt(Math.round(payment.fees));
                needsUpdate = true;
              }

              if (needsUpdate) {
                store.put(payment);
                updated++;
              }
            });

            console.log(`Migrated ${updated} payment records to BigInt format`);
          };
        },
      },
      {
        name: "Add sync tables",
        upgrade: (db, transaction) => {
          // Create sync_revision table if it doesn't exist
          if (!db.objectStoreNames.contains("sync_revision")) {
            const syncRevisionStore = db.createObjectStore("sync_revision", { keyPath: "id" });
            // Insert the initial revision (0)
            transaction.objectStore("sync_revision").add({ id: 1, revision: 0 });
          }

          // Create sync_outgoing table if it doesn't exist
          if (!db.objectStoreNames.contains("sync_outgoing")) {
            db.createObjectStore("sync_outgoing", { keyPath: "id" });
            // Create index on revision
            transaction.objectStore("sync_outgoing").createIndex("revision", "revision");
            // Create index on record_id 
            transaction.objectStore("sync_outgoing").createIndex("record_id", ["record_id.type", "record_id.data_id"]);
          }

          // Create sync_incoming table if it doesn't exist
          if (!db.objectStoreNames.contains("sync_incoming")) {
            db.createObjectStore("sync_incoming", { keyPath: "id" });
          }

          // Create sync_state table if it doesn't exist
          if (!db.objectStoreNames.contains("sync_state")) {
            db.createObjectStore("sync_state", { keyPath: "id" });
          }
        }
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
    this.dbVersion = 4; // Current schema version
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

  async listPayments(request) {
    if (!this.db) {
      throw new StorageError("Database not initialized");
    }

    // Handle null values by using default values
    const actualOffset = request.offset !== null ? request.offset : 0;
    const actualLimit = request.limit !== null ? request.limit : 4294967295; // u32::MAX

    return new Promise((resolve, reject) => {
      const transaction = this.db.transaction(
        ["payments", "payment_metadata"],
        "readonly"
      );
      const paymentStore = transaction.objectStore("payments");
      const metadataStore = transaction.objectStore("payment_metadata");

      const payments = [];
      let count = 0;
      let skipped = 0;

      // Determine sort order - "prev" for descending (default), "next" for ascending
      const cursorDirection = request.sortAscending ? "next" : "prev";

      // Use cursor to iterate through payments ordered by timestamp
      const cursorRequest = paymentStore
        .index("timestamp")
        .openCursor(null, cursorDirection);

      cursorRequest.onsuccess = (event) => {
        const cursor = event.target.result;

        if (!cursor || count >= actualLimit) {
          resolve(payments);
          return;
        }

        const payment = cursor.value;

        // Apply filters
        if (!this._matchesFilters(payment, request)) {
          cursor.continue();
          return;
        }

        if (skipped < actualOffset) {
          skipped++;
          cursor.continue();
          return;
        }

        // Get metadata for this payment
        const metadataRequest = metadataStore.get(payment.id);
        metadataRequest.onsuccess = () => {
          const metadata = metadataRequest.result;
          const paymentWithMetadata = this._mergePaymentMetadata(
            payment,
            metadata
          );
          payments.push(paymentWithMetadata);
          count++;
          cursor.continue();
        };
        metadataRequest.onerror = () => {
          // Continue without metadata if it fails
          payments.push(payment);
          count++;
          cursor.continue();
        };
      };

      cursorRequest.onerror = () => {
        reject(
          new StorageError(
            `Failed to list payments (request: ${JSON.stringify(request)}: ${
              cursorRequest.error?.message || "Unknown error"
            }`,
            cursorRequest.error
          )
        );
      };
    });
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

    return new Promise((resolve, reject) => {
      const transaction = this.db.transaction(
        ["payments", "payment_metadata"],
        "readonly"
      );
      const paymentStore = transaction.objectStore("payments");
      const metadataStore = transaction.objectStore("payment_metadata");

      const paymentRequest = paymentStore.get(id);

      paymentRequest.onsuccess = () => {
        const payment = paymentRequest.result;
        if (!payment) {
          reject(new StorageError(`Payment with id '${id}' not found`));
          return;
        }

        // Get metadata for this payment
        const metadataRequest = metadataStore.get(id);
        metadataRequest.onsuccess = () => {
          const metadata = metadataRequest.result;
          const paymentWithMetadata = this._mergePaymentMetadata(
            payment,
            metadata
          );
          resolve(paymentWithMetadata);
        };
        metadataRequest.onerror = () => {
          // Return payment without metadata if metadata fetch fails
          resolve(payment);
        };
      };

      paymentRequest.onerror = () => {
        reject(
          new StorageError(
            `Failed to get payment by id '${id}': ${
              paymentRequest.error?.message || "Unknown error"
            }`,
            paymentRequest.error
          )
        );
      };
    });
  }

  async getPaymentByInvoice(invoice) {
    if (!this.db) {
      throw new StorageError("Database not initialized");
    }

    return new Promise((resolve, reject) => {
      const transaction = this.db.transaction(
        ["payments", "payment_metadata"],
        "readonly"
      );
      const paymentStore = transaction.objectStore("payments");
      const invoiceIndex = paymentStore.index("invoice");
      const metadataStore = transaction.objectStore("payment_metadata");

      const paymentRequest = invoiceIndex.get(invoice);

      paymentRequest.onsuccess = () => {
        const payment = paymentRequest.result;
        if (!payment) {
          resolve(null);
          return;
        }

        // Get metadata for this payment
        const metadataRequest = metadataStore.get(invoice);
        metadataRequest.onsuccess = () => {
          const metadata = metadataRequest.result;
          const paymentWithMetadata = this._mergePaymentMetadata(
            payment,
            metadata
          );
          resolve(paymentWithMetadata);
        };
        metadataRequest.onerror = () => {
          // Return payment without metadata if metadata fetch fails
          resolve(payment);
        };
      };

      paymentRequest.onerror = () => {
        reject(
          new StorageError(
            `Failed to get payment by invoice '${invoice}': ${
              paymentRequest.error?.message || "Unknown error"
            }`,
            paymentRequest.error
          )
        );
      };
    });
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
        lnurlWithdrawInfo: metadata.lnurlWithdrawInfo
          ? JSON.stringify(metadata.lnurlWithdrawInfo)
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

  async sync_add_outgoing_change(record) {
    if (!this.db) {
      throw new StorageError("Database not initialized");
    }

    return new Promise((resolve, reject) => {
      const transaction = this.db.transaction(["sync_outgoing", "sync_revision"], "readwrite");
      
      // Get the next revision
      const revisionStore = transaction.objectStore("sync_revision");
      const getRevisionRequest = revisionStore.get(1);
      
      getRevisionRequest.onsuccess = () => {
        const revisionData = getRevisionRequest.result || { id: 1, revision: 0 };
        const nextRevision = revisionData.revision + 1;
        
        // Update the revision
        const updateRequest = revisionStore.put({ id: 1, revision: nextRevision });
        
        updateRequest.onsuccess = () => {
          // Create the record change
          const outgoingStore = transaction.objectStore("sync_outgoing");
          const recordId = `${record.id.type}:${record.id.data_id}`;
          
          const recordChange = {
            id: `${Date.now()}-${Math.random().toString(36).substring(2, 15)}`,
            record_id: record.id,
            schema_version: record.schemaVersion,
            updated_fields: record.updatedFields,
            revision: nextRevision,
            synced: false
          };
          
          const addRequest = outgoingStore.add(recordChange);
          
          addRequest.onsuccess = () => {
            resolve(nextRevision);
          };
          
          addRequest.onerror = (event) => {
            reject(new StorageError(`Failed to add outgoing change: ${event.target.error.message}`));
          };
        };
        
        updateRequest.onerror = (event) => {
          reject(new StorageError(`Failed to update revision: ${event.target.error.message}`));
        };
      };
      
      getRevisionRequest.onerror = (event) => {
        reject(new StorageError(`Failed to get revision: ${event.target.error.message}`));
      };
      
      transaction.onerror = (event) => {
        reject(new StorageError(`Transaction failed: ${event.target.error.message}`));
      };
    });
  }

  async sync_complete_outgoing_sync(record) {
    if (!this.db) {
      throw new StorageError("Database not initialized");
    }

    return new Promise((resolve, reject) => {
      const transaction = this.db.transaction(["sync_outgoing", "sync_state"], "readwrite");
      const outgoingStore = transaction.objectStore("sync_outgoing");
      const stateStore = transaction.objectStore("sync_state");
      
      // Find the record by record_id
      const index = outgoingStore.index("record_id");
      const request = index.openCursor([record.id.type, record.id.data_id]);
      
      request.onsuccess = (event) => {
        const cursor = event.target.result;
        if (cursor) {
          // Delete the record instead of marking it as synced
          cursor.delete();
          
          // Also store the record in the sync state
          const stateRecord = {
            id: `${record.id.type}:${record.id.data_id}`,
            record: record
          };
          
          stateStore.put(stateRecord);
          resolve();
        } else {
          reject(new StorageError(`Record not found for sync completion`));
        }
      };
      
      request.onerror = (event) => {
        reject(new StorageError(`Failed to complete outgoing sync: ${event.target.error.message}`));
      };
    });
  }

  async sync_get_pending_outgoing_changes(limit) {
    if (!this.db) {
      throw new StorageError("Database not initialized");
    }

    return new Promise((resolve, reject) => {
      const transaction = this.db.transaction(["sync_outgoing", "sync_state"], "readonly");
      const outgoingStore = transaction.objectStore("sync_outgoing");
      const stateStore = transaction.objectStore("sync_state");
      
      // Get pending outgoing changes (all records in this store are pending)
      const request = outgoingStore.openCursor();
      const changes = [];
      let count = 0;
      
      request.onsuccess = (event) => {
        const cursor = event.target.result;
        if (cursor && count < limit) {
          const record = cursor.value;
          // Look up parent record if it exists
          const stateRequest = stateStore.get(`${record.record_id.type}:${record.record_id.data_id}`);
          
          stateRequest.onsuccess = () => {
            const parent = stateRequest.result ? stateRequest.result.record : null;
            
            // Create a change set
            const change = {
              id: record.record_id,
              schemaVersion: record.schema_version,
              updatedFields: record.updated_fields,
              revision: record.revision
            };
            
            changes.push({
              change: change,
              parent: parent
            });
            
            count++;
            cursor.continue();
          };
          
          stateRequest.onerror = () => {
            // Continue even if parent lookup fails
            const change = {
              id: record.record_id,
              schemaVersion: record.schema_version,
              updatedFields: record.updated_fields,
              revision: record.revision
            };
            
            changes.push({
              change: change,
              parent: null
            });
            
            count++;
            cursor.continue();
          };
        } else {
          resolve(changes);
        }
      };
      
      request.onerror = (event) => {
        reject(new StorageError(`Failed to get pending outgoing changes: ${event.target.error.message}`));
      };
    });
  }

  async sync_get_last_revision() {
    if (!this.db) {
      throw new StorageError("Database not initialized");
    }

    return new Promise((resolve, reject) => {
      const transaction = this.db.transaction("sync_revision", "readonly");
      const store = transaction.objectStore("sync_revision");
      const request = store.get(1);
      
      request.onsuccess = () => {
        const result = request.result || { id: 1, revision: 0 };
        resolve(result.revision);
      };
      
      request.onerror = (event) => {
        reject(new StorageError(`Failed to get last revision: ${event.target.error.message}`));
      };
    });
  }

  async sync_insert_incoming_records(records) {
    if (!this.db) {
      throw new StorageError("Database not initialized");
    }

    return new Promise((resolve, reject) => {
      const transaction = this.db.transaction(["sync_incoming"], "readwrite");
      const store = transaction.objectStore("sync_incoming");
      
      // Add each record to the incoming store
      let recordsProcessed = 0;
      
      for (const record of records) {
        const incomingRecord = {
          id: `${Date.now()}-${Math.random().toString(36).substring(2, 15)}`,
          record: record
        };
        
        const request = store.add(incomingRecord);
        
        request.onsuccess = () => {
          recordsProcessed++;
          if (recordsProcessed === records.length) {
            resolve();
          }
        };
        
        request.onerror = (event) => {
          reject(new StorageError(`Failed to insert incoming record: ${event.target.error.message}`));
        };
      }
      
      // If no records were provided
      if (records.length === 0) {
        resolve();
      }
    });
  }

  async sync_delete_incoming_record(record) {
    if (!this.db) {
      throw new StorageError("Database not initialized");
    }

    return new Promise((resolve, reject) => {
      const transaction = this.db.transaction(["sync_incoming"], "readwrite");
      const store = transaction.objectStore("sync_incoming");
      
      // Find the record in the incoming store
      const request = store.openCursor();
      
      request.onsuccess = (event) => {
        const cursor = event.target.result;
        if (cursor) {
          const incomingRecord = cursor.value;
          
          if (incomingRecord.record && 
              incomingRecord.record.id.type === record.id.type &&
              incomingRecord.record.id.data_id === record.id.data_id &&
              incomingRecord.record.revision === record.revision) {
            
            // Delete the record
            const deleteRequest = cursor.delete();
            
            deleteRequest.onsuccess = () => {
              resolve();
            };
            
            deleteRequest.onerror = (event) => {
              reject(new StorageError(`Failed to delete incoming record: ${event.target.error.message}`));
            };
          } else {
            cursor.continue();
          }
        } else {
          // Record not found, but we'll resolve anyway
          resolve();
        }
      };
      
      request.onerror = (event) => {
        reject(new StorageError(`Failed to search for incoming record: ${event.target.error.message}`));
      };
    });
  }

  async sync_rebase_pending_outgoing_records(revision) {
    if (!this.db) {
      throw new StorageError("Database not initialized");
    }

    return new Promise((resolve, reject) => {
      const transaction = this.db.transaction(["sync_outgoing", "sync_revision"], "readwrite");
      const outgoingStore = transaction.objectStore("sync_outgoing");
      const revisionStore = transaction.objectStore("sync_revision");
      
      // Get the current revision
      const getRevisionRequest = revisionStore.get(1);
      
      getRevisionRequest.onsuccess = () => {
        const revisionData = getRevisionRequest.result || { id: 1, revision: 0 };
        const currentRevision = revisionData.revision;
        
        if (revision > currentRevision) {
          // Update the revision
          revisionStore.put({ id: 1, revision: revision });
          
          // Get all records that need rebasing (where synced = false)
          const recordsRequest = outgoingStore.openCursor();
          const records = [];
          
          recordsRequest.onsuccess = (event) => {
            const cursor = event.target.result;
            if (cursor) {
              const record = cursor.value;
              if (!record.synced) {
                records.push({ cursor, record });
              }
              cursor.continue();
            } else {
              // Update all records that need rebasing
              let newRevision = revision;
              
              for (const { cursor, record } of records) {
                newRevision++;
                record.revision = newRevision;
                cursor.update(record);
              }
              
              // Update the final revision
              if (records.length > 0) {
                revisionStore.put({ id: 1, revision: newRevision });
              }
              
              resolve();
            }
          };
          
          recordsRequest.onerror = (event) => {
            reject(new StorageError(`Failed to rebase outgoing records: ${event.target.error.message}`));
          };
        } else {
          // No rebasing needed
          resolve();
        }
      };
      
      getRevisionRequest.onerror = (event) => {
        reject(new StorageError(`Failed to get revision for rebasing: ${event.target.error.message}`));
      };
    });
  }

  async sync_get_incoming_records(limit) {
    if (!this.db) {
      throw new StorageError("Database not initialized");
    }

    return new Promise((resolve, reject) => {
      const transaction = this.db.transaction(["sync_incoming", "sync_state"], "readwrite");
      const incomingStore = transaction.objectStore("sync_incoming");
      const stateStore = transaction.objectStore("sync_state");
      
      // Get records up to the limit
      const request = incomingStore.openCursor();
      const records = [];
      const recordsToDelete = [];
      let count = 0;
      
      request.onsuccess = (event) => {
        const cursor = event.target.result;
        if (cursor && count < limit) {
          const incomingRecord = cursor.value;
          recordsToDelete.push(incomingRecord.id);
          
          // Look for parent record
          const stateRequest = stateStore.get(`${incomingRecord.record.id.type}:${incomingRecord.record.id.data_id}`);
          
          stateRequest.onsuccess = () => {
            const parent = stateRequest.result ? stateRequest.result.record : null;
            
            records.push({
              record: incomingRecord.record,
              parent: parent
            });
            
            count++;
            cursor.continue();
          };
          
          stateRequest.onerror = () => {
            // Continue even if parent lookup fails
            records.push({
              record: incomingRecord.record,
              parent: null
            });
            
            count++;
            cursor.continue();
          };
        } else {
          // Delete the fetched records
          if (recordsToDelete.length > 0) {
            recordsToDelete.forEach(id => {
              incomingStore.delete(id);
            });
          }
          
          resolve(records);
        }
      };
      
      request.onerror = (event) => {
        reject(new StorageError(`Failed to get incoming records: ${event.target.error.message}`));
      };
    });
  }

  async sync_get_latest_outgoing_change() {
    if (!this.db) {
      throw new StorageError("Database not initialized");
    }

    return new Promise((resolve, reject) => {
      const transaction = this.db.transaction(["sync_outgoing", "sync_state"], "readonly");
      const outgoingStore = transaction.objectStore("sync_outgoing");
      const stateStore = transaction.objectStore("sync_state");
      
      // Get the highest revision record
      const index = outgoingStore.index("revision");
      const request = index.openCursor(null, "prev");
      
      request.onsuccess = (event) => {
        const cursor = event.target.result;
        if (cursor) {
          const record = cursor.value;
          
          // Get the parent record
          const stateRequest = stateStore.get(`${record.record_id.type}:${record.record_id.data_id}`);
          
          stateRequest.onsuccess = () => {
            const parent = stateRequest.result ? stateRequest.result.record : null;
            
            // Create a change set
            const change = {
              id: record.record_id,
              schemaVersion: record.schema_version,
              updatedFields: record.updated_fields,
              revision: record.revision
            };
            
            resolve({
              change: change,
              parent: parent
            });
          };
          
          stateRequest.onerror = () => {
            // Return without parent if lookup fails
            const change = {
              id: record.record_id,
              schemaVersion: record.schema_version,
              updatedFields: record.updated_fields,
              revision: record.revision
            };
            
            resolve({
              change: change,
              parent: null
            });
          };
        } else {
          // No records found
          resolve(null);
        }
      };
      
      request.onerror = (event) => {
        reject(new StorageError(`Failed to get latest outgoing change: ${event.target.error.message}`));
      };
    });
  }

  async sync_update_record_from_incoming(record) {
    if (!this.db) {
      throw new StorageError("Database not initialized");
    }

    return new Promise((resolve, reject) => {
      const transaction = this.db.transaction(["sync_state"], "readwrite");
      const stateStore = transaction.objectStore("sync_state");
      
      // Store or update the record state
      const stateRecord = {
        id: `${record.id.type}:${record.id.data_id}`,
        record: record
      };
      
      const request = stateStore.put(stateRecord);
      
      request.onsuccess = () => {
        resolve();
      };
      
      request.onerror = (event) => {
        reject(new StorageError(`Failed to update record from incoming: ${event.target.error.message}`));
      };
    });
  }

  // ===== Private Helper Methods =====

  _matchesFilters(payment, request) {
    // Filter by payment type
    if (request.typeFilter && request.typeFilter.length > 0) {
      if (!request.typeFilter.includes(payment.paymentType)) {
        return false;
      }
    }

    // Filter by status
    if (request.statusFilter && request.statusFilter.length > 0) {
      if (!request.statusFilter.includes(payment.status)) {
        return false;
      }
    }

    // Filter by timestamp range
    if (request.fromTimestamp !== null && request.fromTimestamp !== undefined) {
      if (payment.timestamp < request.fromTimestamp) {
        return false;
      }
    }

    if (request.toTimestamp !== null && request.toTimestamp !== undefined) {
      if (payment.timestamp >= request.toTimestamp) {
        return false;
      }
    }

    // Filter by payment details/method
    if (request.assetFilter) {
      const assetFilter = request.assetFilter;
      let details = null;

      // Parse details if it's a string (stored in IndexedDB)
      if (payment.details && typeof payment.details === "string") {
        try {
          details = JSON.parse(payment.details);
        } catch (e) {
          // If parsing fails, treat as no details
          details = null;
        }
      } else {
        details = payment.details;
      }

      if (!details) {
        return false;
      }

      if (assetFilter.type === "bitcoin" && details.type === "token") {
        return false;
      }

      if (assetFilter.type === "token") {
        if (details.type !== "token") {
          return false;
        }

        // Check token identifier if specified
        if (assetFilter.tokenIdentifier) {
          if (
            !details.metadata ||
            details.metadata.identifier !== assetFilter.tokenIdentifier
          ) {
            return false;
          }
        }
      }
    }

    return true;
  }

  _mergePaymentMetadata(payment, metadata) {
    let details = null;
    if (payment.details) {
      try {
        details = JSON.parse(payment.details);
      } catch (e) {
        throw new StorageError(
          `Failed to parse payment details JSON for payment ${payment.id}: ${e.message}`,
          e
        );
      }
    }

    let method = null;
    if (payment.method) {
      try {
        method = JSON.parse(payment.method);
      } catch (e) {
        throw new StorageError(
          `Failed to parse payment method JSON for payment ${payment.id}: ${e.message}`,
          e
        );
      }
    }

    // If this is a Lightning payment and we have metadata
    if (metadata && details && details.type == "lightning") {
      if (metadata.lnurlDescription && !details.description) {
        details.description = metadata.lnurlDescription;
      }
      // If lnurlPayInfo exists, parse and add to details
      if (metadata.lnurlPayInfo) {
        try {
          details.lnurlPayInfo = JSON.parse(metadata.lnurlPayInfo);
        } catch (e) {
          throw new StorageError(
            `Failed to parse lnurlPayInfo JSON for payment ${payment.id}: ${e.message}`,
            e
          );
        }
      }
      // If lnurlWithdrawInfo exists, parse and add to details
      if (metadata.lnurlWithdrawInfo) {
        try {
          details.lnurlWithdrawInfo = JSON.parse(metadata.lnurlWithdrawInfo);
        } catch (e) {
          throw new StorageError(
            `Failed to parse lnurlWithdrawInfo JSON for payment ${payment.id}: ${e.message}`,
            e
          );
        }
      }
    }

    return {
      id: payment.id,
      paymentType: payment.paymentType,
      status: payment.status,
      amount: payment.amount,
      fees: payment.fees,
      timestamp: payment.timestamp,
      method,
      details,
    };
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
