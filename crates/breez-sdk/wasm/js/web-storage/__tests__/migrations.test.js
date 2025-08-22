/**
 * Tests for IndexedDB Migration Manager
 */

// Set up IndexedDB polyfill for Node.js testing
import "fake-indexeddb/auto";
import { jest } from "@jest/globals";

import { MigrationManager } from "../migrations.js";
import { StorageError } from "../errors.js";

describe("MigrationManager", () => {
  let db;
  let migrationManager;

  beforeEach(async () => {
    // Create a fresh database for each test
    db = await new Promise((resolve, reject) => {
      const request = indexedDB.open("TestMigrationDB", 1);
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error);
      request.onupgradeneeded = (event) => {
        const db = event.target.result;
        migrationManager = new MigrationManager(db, StorageError);
        migrationManager.handleUpgrade(
          event,
          event.oldVersion,
          event.newVersion
        );
      };
    });
  });

  afterEach(() => {
    if (db) {
      db.close();
    }
  });

  describe("Schema Creation", () => {
    test("should create all required object stores", () => {
      const expectedStores = [
        "settings",
        "payments",
        "payment_metadata",
        "unclaimed_deposits",
      ];

      expectedStores.forEach((storeName) => {
        expect(db.objectStoreNames.contains(storeName)).toBe(true);
      });
    });

    test("should create proper indexes", () => {
      const transaction = db.transaction(
        ["payments", "unclaimed_deposits"],
        "readonly"
      );

      // Check payments indexes
      const paymentStore = transaction.objectStore("payments");
      expect(paymentStore.indexNames.contains("timestamp")).toBe(true);
      expect(paymentStore.indexNames.contains("paymentType")).toBe(true);
      expect(paymentStore.indexNames.contains("status")).toBe(true);

      // Check unclaimed_deposits indexes
      const depositStore = transaction.objectStore("unclaimed_deposits");
      expect(depositStore.indexNames.contains("txid")).toBe(true);
    });
  });

  describe("Migration Info", () => {
    test("should provide correct migration info", () => {
      const info = migrationManager.getMigrationInfo();

      expect(info.currentVersion).toBe(1);
      expect(info.totalMigrations).toBe(1);
      expect(info.isUpToDate).toBe(true);
      expect(info.pendingMigrations).toBe(0);
    });
  });

  describe("Schema Validation", () => {
    test("should validate schema successfully", async () => {
      const validation = await migrationManager.validateSchema();

      expect(validation.isValid).toBe(true);
      expect(validation.stores).toContain("settings");
      expect(validation.stores).toContain("payments");
      expect(validation.stores).toContain("payment_metadata");
      expect(validation.stores).toContain("unclaimed_deposits");
    });

    test("should detect missing stores", async () => {
      // Simulate a database with missing stores by creating a minimal version
      const badDb = await new Promise((resolve, reject) => {
        const request = indexedDB.open("BadTestDB", 1);
        request.onsuccess = () => resolve(request.result);
        request.onerror = () => reject(request.error);
        request.onupgradeneeded = (event) => {
          const db = event.target.result;
          // Only create settings store, missing others
          if (!db.objectStoreNames.contains("settings")) {
            db.createObjectStore("settings", { keyPath: "key" });
          }
        };
      });

      const badMigrationManager = new MigrationManager(badDb, StorageError);

      await expect(badMigrationManager.validateSchema()).rejects.toThrow(
        StorageError
      );

      badDb.close();
    });
  });

  describe("Error Handling", () => {
    test("should handle migration errors gracefully", () => {
      const mockEvent = {
        target: {
          result: {
            objectStoreNames: {
              contains: () => false,
            },
            createObjectStore: () => {
              throw new Error("Test migration error");
            },
          },
          transaction: {},
        },
      };

      expect(() => {
        migrationManager.handleUpgrade(mockEvent, 0, 1);
      }).toThrow(StorageError);
    });
  });

  describe("Logging", () => {
    test("should log migration progress when logger is provided", () => {
      const mockLogger = {
        log: jest.fn(),
      };

      const loggerMigrationManager = new MigrationManager(
        db,
        StorageError,
        mockLogger
      );
      loggerMigrationManager.migrate();

      expect(mockLogger.log).toHaveBeenCalled();
    });

    test("should not throw when no logger is provided", () => {
      const noLoggerMigrationManager = new MigrationManager(db, StorageError);

      expect(() => {
        noLoggerMigrationManager.migrate();
      }).not.toThrow();
    });
  });
});
