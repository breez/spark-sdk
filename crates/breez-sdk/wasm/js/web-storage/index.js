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
          // sync_revision: tracks the last committed revision (from server-acknowledged
          // or server-received records). Does NOT include pending outgoing revisions.
          if (!db.objectStoreNames.contains("sync_revision")) {
            const syncRevisionStore = db.createObjectStore("sync_revision", {
              keyPath: "id",
            });
            transaction
              .objectStore("sync_revision")
              .add({ id: 1, revision: "0" });
          }

          if (!db.objectStoreNames.contains("sync_outgoing")) {
            db.createObjectStore("sync_outgoing", {
              keyPath: ["type", "dataId", "revision"],
            });
            transaction
              .objectStore("sync_outgoing")
              .createIndex("revision", "revision");
          }

          if (!db.objectStoreNames.contains("sync_incoming")) {
            db.createObjectStore("sync_incoming", {
              keyPath: ["type", "dataId", "revision"],
            });
            transaction
              .objectStore("sync_incoming")
              .createIndex("revision", "revision");
          }

          if (!db.objectStoreNames.contains("sync_state")) {
            db.createObjectStore("sync_state", { keyPath: ["type", "dataId"] });
          }
        },
      },
      {
        name: "Create lnurl_receive_metadata store",
        upgrade: (db) => {
          if (!db.objectStoreNames.contains("lnurl_receive_metadata")) {
            db.createObjectStore("lnurl_receive_metadata", {
              keyPath: "paymentHash",
            });
          }
        },
      },
      {
        // Delete all unclaimed deposits to clear old claim_error JSON format.
        // Deposits will be recovered on next sync.
        name: "Clear unclaimed deposits for claim_error format change",
        upgrade: (db, transaction) => {
          if (db.objectStoreNames.contains("unclaimed_deposits")) {
            const store = transaction.objectStore("unclaimed_deposits");
            store.clear();
          }
        },
      },
      {
        name: "Clear sync tables for BreezSigner backward compatibility",
        upgrade: (db, transaction) => {
          // Clear all sync tables due to BreezSigner signature change.
          // This forces users to sync from scratch to the sync server.
          // Also delete the sync_initial_complete flag to force re-populating
          // all payment metadata for outgoing sync using the new key.

          // Clear sync tables (only if they exist)
          if (db.objectStoreNames.contains("sync_outgoing")) {
            const syncOutgoing = transaction.objectStore("sync_outgoing");
            syncOutgoing.clear();
          }

          if (db.objectStoreNames.contains("sync_incoming")) {
            const syncIncoming = transaction.objectStore("sync_incoming");
            syncIncoming.clear();
          }

          if (db.objectStoreNames.contains("sync_state")) {
            const syncState = transaction.objectStore("sync_state");
            syncState.clear();
          }

          // Reset revision to 0 (only if store exists)
          if (db.objectStoreNames.contains("sync_revision")) {
            const syncRevision = transaction.objectStore("sync_revision");
            syncRevision.clear();
            syncRevision.put({ id: 1, revision: "0" });
          }

          // Delete sync_initial_complete setting (only if store exists)
          if (db.objectStoreNames.contains("settings")) {
            const settings = transaction.objectStore("settings");
            settings.delete("sync_initial_complete");
          }
        }
      },
      {
        name: "Create parentPaymentId index for related payments lookup",
        upgrade: (db, transaction) => {
          if (db.objectStoreNames.contains("payment_metadata")) {
            const metadataStore = transaction.objectStore("payment_metadata");
            if (!metadataStore.indexNames.contains("parentPaymentId")) {
              metadataStore.createIndex("parentPaymentId", "parentPaymentId", { unique: false });
            }
          }
        }
      },
      {
        name: "Add tx_type to token payments and trigger token re-sync",
        upgrade: (db, transaction) => {
          // Update all existing token payments to have a default txType
          if (db.objectStoreNames.contains("payments")) {
            const paymentStore = transaction.objectStore("payments");
            const getAllRequest = paymentStore.getAll();

            getAllRequest.onsuccess = () => {
              const payments = getAllRequest.result;

              payments.forEach((payment) => {
                // Parse details if it's a string
                let details = null;
                if (payment.details && typeof payment.details === "string") {
                  try {
                    details = JSON.parse(payment.details);
                  } catch (e) {
                    return; // Skip this payment if parsing fails
                  }
                } else {
                  details = payment.details;
                }

                // Add default txType to token payments
                if (details && details.type === "token" && !details.txType) {
                  details.txType = "transfer";
                  payment.details = JSON.stringify(details);
                  paymentStore.put(payment);
                }
              });
            };
          }

          // Reset sync cache to trigger token re-sync
          if (db.objectStoreNames.contains("settings")) {
            const settingsStore = transaction.objectStore("settings");
            const getRequest = settingsStore.get("sync_offset");

            getRequest.onsuccess = () => {
              const syncCache = getRequest.result;
              if (syncCache && syncCache.value) {
                try {
                  const syncInfo = JSON.parse(syncCache.value);
                  // Reset only the token sync position, keep the bitcoin offset
                  syncInfo.last_synced_final_token_payment_id = null;
                  settingsStore.put({
                    key: "sync_offset",
                    value: JSON.stringify(syncInfo),
                  });
                } catch (e) {
                  // If parsing fails, just continue
                }
              }
            };
          }
        },
      },
      {
        name: "Clear sync tables to force re-sync",
        upgrade: (db, transaction) => {
          if (db.objectStoreNames.contains("sync_outgoing")) {
            transaction.objectStore("sync_outgoing").clear();
          }
          if (db.objectStoreNames.contains("sync_incoming")) {
            transaction.objectStore("sync_incoming").clear();
          }
          if (db.objectStoreNames.contains("sync_state")) {
            transaction.objectStore("sync_state").clear();
          }
          if (db.objectStoreNames.contains("sync_revision")) {
            const syncRevision = transaction.objectStore("sync_revision");
            syncRevision.clear();
            syncRevision.put({ id: 1, revision: "0" });
          }
          if (db.objectStoreNames.contains("settings")) {
            transaction.objectStore("settings").delete("sync_initial_complete");
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
    this.dbVersion = 10; // Current schema version
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

  /**
   * Gets the set of payment IDs that are related payments (have a parentPaymentId).
   * Uses the parentPaymentId index for efficient lookup.
   * @param {IDBObjectStore} metadataStore - The payment_metadata object store
   * @returns {Promise<Set<string>>} Set of payment IDs that are related payments
   */
  _getRelatedPaymentIds(metadataStore) {
    return new Promise((resolve) => {
      const relatedPaymentIds = new Set();

      // Check if the parentPaymentId index exists (added in migration)
      if (!metadataStore.indexNames.contains("parentPaymentId")) {
        // Index doesn't exist yet, fall back to scanning all metadata
        const cursorRequest = metadataStore.openCursor();
        cursorRequest.onsuccess = (event) => {
          const cursor = event.target.result;
          if (cursor) {
            if (cursor.value.parentPaymentId) {
              relatedPaymentIds.add(cursor.value.paymentId);
            }
            cursor.continue();
          } else {
            resolve(relatedPaymentIds);
          }
        };
        cursorRequest.onerror = () => resolve(new Set());
        return;
      }

      // Use the parentPaymentId index to find all metadata entries with a parent
      const index = metadataStore.index("parentPaymentId");
      const cursorRequest = index.openCursor();

      cursorRequest.onsuccess = (event) => {
        const cursor = event.target.result;
        if (cursor) {
          // Only add if parentPaymentId is truthy (not null/undefined)
          if (cursor.value.parentPaymentId) {
            relatedPaymentIds.add(cursor.value.paymentId);
          }
          cursor.continue();
        } else {
          resolve(relatedPaymentIds);
        }
      };

      cursorRequest.onerror = () => {
        // If index lookup fails, return empty set and fall back to per-payment lookup
        resolve(new Set());
      };
    });
  }

  async listPayments(request) {
    if (!this.db) {
      throw new StorageError("Database not initialized");
    }

    // Handle null values by using default values
    const actualOffset = request.offset !== null ? request.offset : 0;
    const actualLimit = request.limit !== null ? request.limit : 4294967295; // u32::MAX

    const transaction = this.db.transaction(
      ["payments", "payment_metadata", "lnurl_receive_metadata"],
      "readonly"
    );
    const paymentStore = transaction.objectStore("payments");
    const metadataStore = transaction.objectStore("payment_metadata");
    const lnurlReceiveMetadataStore = transaction.objectStore("lnurl_receive_metadata");

    // Build set of related payment IDs upfront for O(1) filtering
    const relatedPaymentIds = await this._getRelatedPaymentIds(metadataStore);

    return new Promise((resolve, reject) => {
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

        // Skip related payments (those with a parentPaymentId)
        if (relatedPaymentIds.has(payment.id)) {
          cursor.continue();
          return;
        }

        if (skipped < actualOffset) {
          skipped++;
          cursor.continue();
          return;
        }

        // Get metadata for this payment (now only for non-related payments)
        const metadataRequest = metadataStore.get(payment.id);
        metadataRequest.onsuccess = () => {
          const metadata = metadataRequest.result;

          const paymentWithMetadata = this._mergePaymentMetadata(
            payment,
            metadata
          );

          // Apply filters
          if (!this._matchesFilters(paymentWithMetadata, request)) {
            cursor.continue();
            return;
          }

          // Fetch lnurl receive metadata if it's a lightning payment
          this._fetchLnurlReceiveMetadata(
            paymentWithMetadata,
            lnurlReceiveMetadataStore
          )
            .then((mergedPayment) => {
              payments.push(mergedPayment);
              count++;
              cursor.continue();
            })
            .catch(() => {
              // Continue without lnurl receive metadata if fetch fails
              payments.push(paymentWithMetadata);
              count++;
              cursor.continue();
            });
        };
        metadataRequest.onerror = () => {
          // Continue without metadata if it fails
          if (this._matchesFilters(payment, request)) {
            payments.push(payment);
            count++;
          }

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
        ["payments", "payment_metadata", "lnurl_receive_metadata"],
        "readonly"
      );
      const paymentStore = transaction.objectStore("payments");
      const metadataStore = transaction.objectStore("payment_metadata");
      const lnurlReceiveMetadataStore = transaction.objectStore(
        "lnurl_receive_metadata"
      );

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

          // Fetch lnurl receive metadata if it's a lightning payment
          this._fetchLnurlReceiveMetadata(
            paymentWithMetadata,
            lnurlReceiveMetadataStore
          )
            .then(resolve)
            .catch(() => {
              // Continue without lnurl receive metadata if fetch fails
              resolve(paymentWithMetadata);
            });
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
        ["payments", "payment_metadata", "lnurl_receive_metadata"],
        "readonly"
      );
      const paymentStore = transaction.objectStore("payments");
      const invoiceIndex = paymentStore.index("invoice");
      const metadataStore = transaction.objectStore("payment_metadata");
      const lnurlReceiveMetadataStore = transaction.objectStore(
        "lnurl_receive_metadata"
      );

      const paymentRequest = invoiceIndex.get(invoice);

      paymentRequest.onsuccess = () => {
        const payment = paymentRequest.result;
        if (!payment) {
          resolve(null);
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

          // Fetch lnurl receive metadata if it's a lightning payment
          this._fetchLnurlReceiveMetadata(
            paymentWithMetadata,
            lnurlReceiveMetadataStore
          )
            .then(resolve)
            .catch(() => {
              // Continue without lnurl receive metadata if fetch fails
              resolve(paymentWithMetadata);
            });
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

  /**
   * Checks if any related payments exist (payments with a parentPaymentId).
   * Uses the parentPaymentId index for efficient lookup.
   * @param {IDBObjectStore} metadataStore - The payment_metadata object store
   * @returns {Promise<boolean>} True if any related payments exist
   */
  _hasRelatedPayments(metadataStore) {
    return new Promise((resolve) => {
      // Check if the parentPaymentId index exists (added in migration)
      if (!metadataStore.indexNames.contains("parentPaymentId")) {
        // Index doesn't exist yet, fall back to scanning all metadata
        const cursorRequest = metadataStore.openCursor();
        cursorRequest.onsuccess = (event) => {
          const cursor = event.target.result;
          if (cursor) {
            if (cursor.value.parentPaymentId) {
              resolve(true);
              return;
            }
            cursor.continue();
          } else {
            resolve(false);
          }
        };
        cursorRequest.onerror = () => resolve(true); // Assume there might be related payments on error
        return;
      }

      const index = metadataStore.index("parentPaymentId");
      const cursorRequest = index.openCursor();

      cursorRequest.onsuccess = (event) => {
        const cursor = event.target.result;
        if (cursor && cursor.value.parentPaymentId) {
          // Found at least one related payment
          resolve(true);
        } else if (cursor) {
          // Entry with null parentPaymentId, continue searching
          cursor.continue();
        } else {
          // No more entries
          resolve(false);
        }
      };

      cursorRequest.onerror = () => {
        // If index lookup fails, assume there might be related payments
        resolve(true);
      };
    });
  }

  /**
   * Gets payments that have any of the specified parent payment IDs.
   * @param {string[]} parentPaymentIds - Array of parent payment IDs
   * @returns {Promise<Object>} Map of parentPaymentId -> array of RelatedPayment objects
   */
  async getPaymentsByParentIds(parentPaymentIds) {
    if (!this.db) {
      throw new StorageError("Database not initialized");
    }

    if (!parentPaymentIds || parentPaymentIds.length === 0) {
      return {};
    }

    const transaction = this.db.transaction(
      ["payments", "payment_metadata", "lnurl_receive_metadata"],
      "readonly"
    );
    const metadataStore = transaction.objectStore("payment_metadata");

    // Early exit if no related payments exist
    const hasRelated = await this._hasRelatedPayments(metadataStore);
    if (!hasRelated) {
      return {};
    }

    const parentIdSet = new Set(parentPaymentIds);
    const paymentStore = transaction.objectStore("payments");
    const lnurlReceiveMetadataStore = transaction.objectStore("lnurl_receive_metadata");

    return new Promise((resolve, reject) => {
      const result = {};
      const fetchedMetadata = [];

      // Query all metadata records and filter by parentPaymentId
      const cursorRequest = metadataStore.openCursor();

      cursorRequest.onsuccess = (event) => {
        const cursor = event.target.result;
        if (!cursor) {
          // All metadata processed, now fetch payment details
          if (fetchedMetadata.length === 0) {
            resolve(result);
            return;
          }

          let processed = 0;
          for (const metadata of fetchedMetadata) {
            const parentId = metadata.parentPaymentId;
            const paymentRequest = paymentStore.get(metadata.paymentId);
            paymentRequest.onsuccess = () => {
              const payment = paymentRequest.result;
              if (payment) {
                const paymentWithMetadata = this._mergePaymentMetadata(payment, metadata);

                if (!result[parentId]) {
                  result[parentId] = [];
                }

                // Fetch lnurl receive metadata if applicable
                this._fetchLnurlReceiveMetadata(paymentWithMetadata, lnurlReceiveMetadataStore)
                  .then((mergedPayment) => {
                    result[parentId].push(mergedPayment);
                  })
                  .catch(() => {
                    result[parentId].push(paymentWithMetadata);
                  })
                  .finally(() => {
                    processed++;
                    if (processed === fetchedMetadata.length) {
                      // Sort each parent's children by timestamp
                      for (const parentId of Object.keys(result)) {
                        result[parentId].sort((a, b) => a.timestamp - b.timestamp);
                      }
                      resolve(result);
                    }
                  });
              } else {
                processed++;
                if (processed === fetchedMetadata.length) {
                  resolve(result);
                }
              }
            };
            paymentRequest.onerror = () => {
              processed++;
              if (processed === fetchedMetadata.length) {
                resolve(result);
              }
            };
          }
          return;
        }

        const metadata = cursor.value;
        if (metadata.parentPaymentId && parentIdSet.has(metadata.parentPaymentId)) {
          fetchedMetadata.push(metadata);
        }
        cursor.continue();
      };

      cursorRequest.onerror = () => {
        reject(
          new StorageError(
            `Failed to get payments by parent ids: ${
              cursorRequest.error?.message || "Unknown error"
            }`,
            cursorRequest.error
          )
        );
      };
    });
  }

  async insertPaymentMetadata(paymentId, metadata) {
    if (!this.db) {
      throw new StorageError("Database not initialized");
    }

    return new Promise((resolve, reject) => {
      const transaction = this.db.transaction("payment_metadata", "readwrite");
      const store = transaction.objectStore("payment_metadata");

      // First get existing record to merge with
      const getRequest = store.get(paymentId);
      getRequest.onsuccess = () => {
        const existing = getRequest.result || {};

        // Use COALESCE-like behavior: new value if non-null, otherwise keep existing
        const metadataToStore = {
          paymentId,
          parentPaymentId: metadata.parentPaymentId ?? existing.parentPaymentId ?? null,
          lnurlPayInfo: metadata.lnurlPayInfo
            ? JSON.stringify(metadata.lnurlPayInfo)
            : existing.lnurlPayInfo ?? null,
          lnurlWithdrawInfo: metadata.lnurlWithdrawInfo
            ? JSON.stringify(metadata.lnurlWithdrawInfo)
            : existing.lnurlWithdrawInfo ?? null,
          lnurlDescription: metadata.lnurlDescription ?? existing.lnurlDescription ?? null,
          conversionInfo: metadata.conversionInfo
            ? JSON.stringify(metadata.conversionInfo)
            : existing.conversionInfo ?? null,
        };

        const putRequest = store.put(metadataToStore);
        putRequest.onsuccess = () => resolve();
        putRequest.onerror = () => {
          reject(
            new StorageError(
              `Failed to set payment metadata for '${paymentId}': ${
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
            `Failed to get existing payment metadata for '${paymentId}': ${
              getRequest.error?.message || "Unknown error"
            }`,
            getRequest.error
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

  async setLnurlMetadata(metadata) {
    if (!this.db) {
      throw new StorageError("Database not initialized");
    }

    return new Promise((resolve, reject) => {
      const transaction = this.db.transaction(
        "lnurl_receive_metadata",
        "readwrite"
      );
      const store = transaction.objectStore("lnurl_receive_metadata");

      let completed = 0;
      const total = metadata.length;

      if (total === 0) {
        resolve();
        return;
      }

      for (const item of metadata) {
        const request = store.put({
          paymentHash: item.paymentHash,
          nostrZapRequest: item.nostrZapRequest || null,
          nostrZapReceipt: item.nostrZapReceipt || null,
          senderComment: item.senderComment || null,
        });

        request.onsuccess = () => {
          completed++;
          if (completed === total) {
            resolve();
          }
        };

        request.onerror = () => {
          reject(
            new StorageError(
              `Failed to add lnurl metadata for payment hash '${
                item.paymentHash
              }': ${request.error?.message || "Unknown error"}`,
              request.error
            )
          );
        };
      }
    });
  }

  async syncAddOutgoingChange(record) {
    if (!this.db) {
      throw new StorageError("Database not initialized");
    }

    return new Promise((resolve, reject) => {
      const transaction = this.db.transaction(["sync_outgoing"], "readwrite");

      // This revision is a local queue id for pending rows, not a server revision.
      const outgoingStore = transaction.objectStore("sync_outgoing");
      const getAllOutgoingRequest = outgoingStore.getAll();

      getAllOutgoingRequest.onsuccess = () => {
        const records = getAllOutgoingRequest.result;
        let maxOutgoingRevision = BigInt(0);
        for (const storeRecord of records) {
          const rev = BigInt(
            storeRecord.record.localRevision ?? storeRecord.record.revision
          );
          if (rev > maxOutgoingRevision) {
            maxOutgoingRevision = rev;
          }
        }
        const nextRevision = maxOutgoingRevision + BigInt(1);

        const storeRecord = {
          type: record.id.type,
          dataId: record.id.dataId,
          revision: Number(nextRevision),
          record: {
            ...record,
            localRevision: nextRevision,
          },
        };

        const addRequest = outgoingStore.add(storeRecord);

        addRequest.onsuccess = () => {
          transaction.oncomplete = () => {
            resolve(nextRevision);
          };
        };

        addRequest.onerror = (event) => {
          reject(
            new StorageError(
              `Failed to add outgoing change: ${event.target.error.message}`
            )
          );
        };
      };

      getAllOutgoingRequest.onerror = (event) => {
        reject(
          new StorageError(
            `Failed to get outgoing records: ${event.target.error.message}`
          )
        );
      };

      transaction.onerror = (event) => {
        reject(
          new StorageError(`Transaction failed: ${event.target.error.message}`)
        );
      };
    });
  }

  async syncCompleteOutgoingSync(record, localRevision) {
    if (!this.db) {
      throw new StorageError("Database not initialized");
    }

    return new Promise((resolve, reject) => {
      const transaction = this.db.transaction(
        ["sync_outgoing", "sync_state", "sync_revision"],
        "readwrite"
      );
      const outgoingStore = transaction.objectStore("sync_outgoing");
      const stateStore = transaction.objectStore("sync_state");
      const revisionStore = transaction.objectStore("sync_revision");

      const deleteRequest = outgoingStore.delete([
        record.id.type,
        record.id.dataId,
        Number(localRevision),
      ]);

      deleteRequest.onsuccess = () => {
        const stateRecord = {
          type: record.id.type,
          dataId: record.id.dataId,
          record: record,
        };
        stateStore.put(stateRecord);

        // Update sync_revision to track the highest known revision
        const getRevisionRequest = revisionStore.get(1);
        getRevisionRequest.onsuccess = () => {
          const current = getRevisionRequest.result || { id: 1, revision: "0" };
          const currentRevision = BigInt(current.revision);
          const recordRevision = BigInt(record.revision);
          if (recordRevision > currentRevision) {
            revisionStore.put({ id: 1, revision: recordRevision.toString() });
          }
          resolve();
        };
        getRevisionRequest.onerror = (event) => {
          reject(
            new StorageError(
              `Failed to update sync revision: ${event.target.error.message}`
            )
          );
        };
      };

      deleteRequest.onerror = (event) => {
        reject(
          new StorageError(
            `Failed to complete outgoing sync: ${event.target.error.message}`
          )
        );
      };
    });
  }

  async syncGetPendingOutgoingChanges(limit) {
    if (!this.db) {
      throw new StorageError("Database not initialized");
    }

    return new Promise((resolve, reject) => {
      const transaction = this.db.transaction(
        ["sync_outgoing", "sync_state"],
        "readonly"
      );
      const outgoingStore = transaction.objectStore("sync_outgoing");
      const stateStore = transaction.objectStore("sync_state");

      // Get pending outgoing changes (all records in this store are pending)
      // Use revision index to order by revision ascending
      const revisionIndex = outgoingStore.index("revision");
      const request = revisionIndex.openCursor(null, "next");
      const changes = [];
      let count = 0;

      request.onsuccess = (event) => {
        const cursor = event.target.result;
        if (cursor && count < limit) {
          const storeRecord = cursor.value;
          const change = {
            ...storeRecord.record,
            localRevision:
              storeRecord.record.localRevision ?? storeRecord.record.revision,
          };

          // Look up parent record if it exists
          const stateRequest = stateStore.get([
            storeRecord.type,
            storeRecord.dataId,
          ]);
          stateRequest.onsuccess = () => {
            const stateRecord = stateRequest.result;
            const parent = stateRecord ? stateRecord.record : null;

            changes.push({
              change: change,
              parent: parent,
            });

            count++;
            cursor.continue();
          };

          stateRequest.onerror = () => {
            changes.push({
              change: change,
              parent: null,
            });

            count++;
            cursor.continue();
          };
        } else {
          resolve(changes);
        }
      };

      request.onerror = (event) => {
        reject(
          new StorageError(
            `Failed to get pending outgoing changes: ${event.target.error.message}`
          )
        );
      };
    });
  }

  async syncGetLastRevision() {
    if (!this.db) {
      throw new StorageError("Database not initialized");
    }

    return new Promise((resolve, reject) => {
      const transaction = this.db.transaction("sync_revision", "readonly");
      const store = transaction.objectStore("sync_revision");
      const request = store.get(1);

      request.onsuccess = () => {
        const result = request.result || { id: 1, revision: "0" };
        resolve(BigInt(result.revision));
      };

      request.onerror = (event) => {
        reject(
          new StorageError(
            `Failed to get last revision: ${event.target.error.message}`
          )
        );
      };
    });
  }

  async syncInsertIncomingRecords(records) {
    if (!this.db) {
      throw new StorageError("Database not initialized");
    }

    return new Promise((resolve, reject) => {
      const transaction = this.db.transaction(["sync_incoming"], "readwrite");
      const store = transaction.objectStore("sync_incoming");

      // Add each record to the incoming store
      let recordsProcessed = 0;

      for (const record of records) {
        const storeRecord = {
          type: record.id.type,
          dataId: record.id.dataId,
          revision: Number(record.revision),
          record: record,
        };

        const request = store.put(storeRecord);

        request.onsuccess = () => {
          recordsProcessed++;
          if (recordsProcessed === records.length) {
            resolve();
          }
        };

        request.onerror = (event) => {
          reject(
            new StorageError(
              `Failed to insert incoming record: ${event.target.error.message}`
            )
          );
        };
      }

      // If no records were provided
      if (records.length === 0) {
        resolve();
      }
    });
  }

  async syncDeleteIncomingRecord(record) {
    if (!this.db) {
      throw new StorageError("Database not initialized");
    }

    return new Promise((resolve, reject) => {
      const transaction = this.db.transaction(["sync_incoming"], "readwrite");
      const store = transaction.objectStore("sync_incoming");

      const key = [record.id.type, record.id.dataId, Number(record.revision)];
      const request = store.delete(key);

      request.onsuccess = () => {
        resolve();
      };

      request.onerror = (event) => {
        reject(
          new StorageError(
            `Failed to delete incoming record: ${event.target.error.message}`
          )
        );
      };
    });
  }

  async syncGetIncomingRecords(limit) {
    if (!this.db) {
      throw new StorageError("Database not initialized");
    }

    return new Promise((resolve, reject) => {
      const transaction = this.db.transaction(
        ["sync_incoming", "sync_state"],
        "readonly"
      );
      const incomingStore = transaction.objectStore("sync_incoming");
      const stateStore = transaction.objectStore("sync_state");

      // Get records up to the limit, ordered by revision
      const revisionIndex = incomingStore.index("revision");
      const request = revisionIndex.openCursor(null, "next");
      const records = [];
      let count = 0;

      request.onsuccess = (event) => {
        const cursor = event.target.result;
        if (cursor && count < limit) {
          const storeRecord = cursor.value;
          const newState = storeRecord.record;

          // Look for parent record
          const stateRequest = stateStore.get([
            storeRecord.type,
            storeRecord.dataId,
          ]);

          stateRequest.onsuccess = () => {
            const stateRecord = stateRequest.result;
            const oldState = stateRecord ? stateRecord.record : null;

            records.push({
              newState: newState,
              oldState: oldState,
            });

            count++;
            cursor.continue();
          };

          stateRequest.onerror = () => {
            records.push({
              newState: newState,
              oldState: null,
            });

            count++;
            cursor.continue();
          };
        } else {
          resolve(records);
        }
      };

      request.onerror = (event) => {
        reject(
          new StorageError(
            `Failed to get incoming records: ${event.target.error.message}`
          )
        );
      };
    });
  }

  async syncGetLatestOutgoingChange() {
    if (!this.db) {
      throw new StorageError("Database not initialized");
    }

    return new Promise((resolve, reject) => {
      const transaction = this.db.transaction(
        ["sync_outgoing", "sync_state"],
        "readonly"
      );
      const outgoingStore = transaction.objectStore("sync_outgoing");
      const stateStore = transaction.objectStore("sync_state");

      // Get the highest revision record
      const index = outgoingStore.index("revision");
      const request = index.openCursor(null, "prev");

      request.onsuccess = (event) => {
        const cursor = event.target.result;
        if (cursor) {
          const storeRecord = cursor.value;
          const change = {
            ...storeRecord.record,
            localRevision:
              storeRecord.record.localRevision ?? storeRecord.record.revision,
          };

          // Get the parent record
          const stateRequest = stateStore.get([
            storeRecord.type,
            storeRecord.dataId,
          ]);

          stateRequest.onsuccess = () => {
            const stateRecord = stateRequest.result;
            const parent = stateRecord ? stateRecord.record : null;

            resolve({
              change: change,
              parent: parent,
            });
          };

          stateRequest.onerror = () => {
            resolve({
              change: change,
              parent: null,
            });
          };
        } else {
          // No records found
          resolve(null);
        }
      };

      request.onerror = (event) => {
        reject(
          new StorageError(
            `Failed to get latest outgoing change: ${event.target.error.message}`
          )
        );
      };
    });
  }

  async syncUpdateRecordFromIncoming(record) {
    if (!this.db) {
      throw new StorageError("Database not initialized");
    }

    return new Promise((resolve, reject) => {
      const transaction = this.db.transaction(["sync_state", "sync_revision"], "readwrite");
      const stateStore = transaction.objectStore("sync_state");
      const revisionStore = transaction.objectStore("sync_revision");

      const storeRecord = {
        type: record.id.type,
        dataId: record.id.dataId,
        record: record,
      };

      const request = stateStore.put(storeRecord);

      request.onsuccess = () => {
        // Update sync_revision to track the highest known revision
        const getRevisionRequest = revisionStore.get(1);
        getRevisionRequest.onsuccess = () => {
          const current = getRevisionRequest.result || { id: 1, revision: "0" };
          const currentRevision = BigInt(current.revision);
          const incomingRevision = BigInt(record.revision);
          if (incomingRevision > currentRevision) {
            revisionStore.put({ id: 1, revision: incomingRevision.toString() });
          }
          resolve();
        };
        getRevisionRequest.onerror = (event) => {
          reject(
            new StorageError(
              `Failed to update sync revision: ${event.target.error.message}`
            )
          );
        };
      };

      request.onerror = (event) => {
        reject(
          new StorageError(
            `Failed to update record from incoming: ${event.target.error.message}`
          )
        );
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

    // Filter by payment details
    if (
      request.paymentDetailsFilter &&
      request.paymentDetailsFilter.length > 0
    ) {
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

      // Filter by payment details. If any filter matches, we include the payment
      let paymentDetailsFilterMatches = false;
      for (const paymentDetailsFilter of request.paymentDetailsFilter) {
        // Filter by Spark HTLC status
        if (
          paymentDetailsFilter.type === "spark" &&
          paymentDetailsFilter.htlcStatus != null &&
          paymentDetailsFilter.htlcStatus.length > 0
        ) {
          if (
            details.type !== "spark" ||
            !details.htlcDetails ||
            !paymentDetailsFilter.htlcStatus.includes(
              details.htlcDetails.status
            )
          ) {
            continue;
          }
        }
        // Filter by token conversion info presence
        if (
          (paymentDetailsFilter.type === "spark" ||
            paymentDetailsFilter.type === "token") &&
          paymentDetailsFilter.conversionRefundNeeded != null
        ) {
          if (
            details.type !== paymentDetailsFilter.type ||
            !details.conversionInfo
          ) {
            continue;
          }

          if (
            paymentDetailsFilter.conversionRefundNeeded ===
            (details.conversionInfo.status !== "refundNeeded")
          ) {
            continue;
          }
        }
        // Filter by token transaction hash
        if (
          paymentDetailsFilter.type === "token" &&
          paymentDetailsFilter.txHash != null
        ) {
          if (
            details.type !== "token" ||
            details.txHash !== paymentDetailsFilter.txHash
          ) {
            continue;
          }
        }
        // Filter by token transaction type
        if (
          paymentDetailsFilter.type === "token" &&
          paymentDetailsFilter.txType != null
        ) {
          if (
            details.type !== "token" ||
            details.txType !== paymentDetailsFilter.txType
          ) {
            continue;
          }
        }

        paymentDetailsFilterMatches = true;
        break;
      }

      if (!paymentDetailsFilterMatches) {
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

    if (metadata && details) {
      if (details.type == "lightning") {
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
      } else if (details.type == "spark" || details.type == "token") {
        // If conversionInfo exists, parse and add to details
        if (metadata.conversionInfo) {
          try {
            details.conversionInfo = JSON.parse(metadata.conversionInfo);
          } catch (e) {
            throw new StorageError(
              `Failed to parse conversionInfo JSON for payment ${payment.id}: ${e.message}`,
              e
            );
          }
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

  _fetchLnurlReceiveMetadata(payment, lnurlReceiveMetadataStore) {
    // Only fetch for lightning payments with a payment hash
    if (
      !payment.details ||
      payment.details.type !== "lightning" ||
      !payment.details.paymentHash
    ) {
      return Promise.resolve(payment);
    }

    if (!lnurlReceiveMetadataStore) {
      return Promise.resolve(payment);
    }

    return new Promise((resolve, reject) => {
      const lnurlReceiveRequest = lnurlReceiveMetadataStore.get(
        payment.details.paymentHash
      );

      lnurlReceiveRequest.onsuccess = () => {
        const lnurlReceiveMetadata = lnurlReceiveRequest.result;
        if (
          lnurlReceiveMetadata &&
          (lnurlReceiveMetadata.nostrZapRequest ||
            lnurlReceiveMetadata.senderComment)
        ) {
          payment.details.lnurlReceiveMetadata = {
            nostrZapRequest: lnurlReceiveMetadata.nostrZapRequest || null,
            nostrZapReceipt: lnurlReceiveMetadata.nostrZapReceipt || null,
            senderComment: lnurlReceiveMetadata.senderComment || null,
          };
        }
        resolve(payment);
      };

      lnurlReceiveRequest.onerror = () => {
        // Continue without lnurlReceiveMetadata if fetch fails
        reject(new Error("Failed to fetch lnurl receive metadata"));
      };
    });
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
