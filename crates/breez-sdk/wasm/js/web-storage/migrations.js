/**
 * IndexedDB Migration Manager for Breez SDK Web Storage
 * ES6 module version - handles database schema evolution
 */

export class MigrationManager {
  constructor(db, StorageError, logger = null) {
    this.db = db;
    this.StorageError = StorageError;
    this.logger = logger;
    this.migrations = this._getMigrations();
  }

  async migrate() {
    const currentVersion = this.db.version;
    const targetVersion = this.migrations.length;

    if (currentVersion >= targetVersion) {
      this._log("info", `Database is up to date (version ${currentVersion})`);
      return;
    }

    this._log(
      "info",
      `Database schema is at version ${currentVersion}, target is ${targetVersion}`
    );
    // For IndexedDB, migrations are handled during database opening via onupgradeneeded
    // This method is mainly for logging and validation
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
      // Fallback to console.error for errors only
      console.error(`[MigrationManager] ${message}`);
    }
    // For info/debug/warn levels, only log if logger is provided
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
    ];
  }

  /**
   * Get information about migration status
   */
  getMigrationInfo() {
    const currentVersion = this.db.version;
    const totalMigrations = this.migrations.length;

    return {
      currentVersion,
      totalMigrations,
      isUpToDate: currentVersion >= totalMigrations,
      pendingMigrations: Math.max(0, totalMigrations - currentVersion),
    };
  }

  /**
   * Validate database schema (for testing/debugging)
   */
  async validateSchema() {
    const expectedStores = [
      "settings",
      "payments",
      "payment_metadata",
      "unclaimed_deposits",
    ];

    const actualStores = Array.from(this.db.objectStoreNames);
    const missingStores = expectedStores.filter(
      (store) => !actualStores.includes(store)
    );

    if (missingStores.length > 0) {
      throw new this.StorageError(
        `Missing object stores: ${missingStores.join(", ")}`
      );
    }

    return {
      stores: actualStores,
      isValid: missingStores.length === 0,
    };
  }
}
