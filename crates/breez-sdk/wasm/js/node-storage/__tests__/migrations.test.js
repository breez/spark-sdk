/**
 * Tests for Migration Manager
 */

const {
  describe,
  test,
  expect,
  beforeEach,
  afterEach,
} = require("@jest/globals");
const { tmpdir } = require("os");
const { join } = require("path");
const { unlinkSync, existsSync } = require("fs");
const Database = require("better-sqlite3");
const { MigrationManager } = require("../migrations.cjs");
const { StorageError } = require("../errors.cjs");

describe("MigrationManager", () => {
  let db;
  let dbPath;
  let migrationManager;

  beforeEach(() => {
    dbPath = join(
      tmpdir(),
      `test-migrations-${Date.now()}-${Math.random()
        .toString(36)
        .substr(2, 9)}.db`
    );
    db = new Database(dbPath);
    migrationManager = new MigrationManager(db, StorageError);
  });

  afterEach(() => {
    if (db) {
      db.close();
    }
    if (existsSync(dbPath)) {
      try {
        unlinkSync(dbPath);
      } catch (error) {
        console.warn("Failed to clean up test database:", error.message);
      }
    }
  });

  describe("Fresh Database", () => {
    test("should start with version 0", () => {
      const version = migrationManager._getCurrentVersion();
      expect(version).toBe(0);
    });

    test("should run all migrations on fresh database", () => {
      migrationManager.migrate();

      const info = migrationManager.getMigrationInfo();
      expect(info.isUpToDate).toBe(true);
      expect(info.pendingMigrations).toBe(0);
      expect(info.currentVersion).toBe(info.totalMigrations);
    });

    test("should create all expected tables", () => {
      migrationManager.migrate();

      const tables = db
        .prepare(
          `
                SELECT name FROM sqlite_master 
                WHERE type='table' AND name NOT LIKE 'sqlite_%'
                ORDER BY name
            `
        )
        .all()
        .map((t) => t.name);

      expect(tables).toContain("payments");
      expect(tables).toContain("settings");
      expect(tables).toContain("unclaimed_deposits");
      expect(tables).toContain("payment_metadata");
    });
  });

  describe("Schema Validation", () => {
    test("should validate schema after migration", () => {
      migrationManager.migrate();

      const schemaInfo = migrationManager.validateSchema();
      expect(schemaInfo.isValid).toBe(true);
      expect(schemaInfo.tables).toHaveLength(4);
    });

    test("should detect missing tables", () => {
      // Run only first migration
      migrationManager._setVersion(1);

      expect(() => {
        migrationManager.validateSchema();
      }).toThrow("Missing tables:");
    });
  });

  describe("Incremental Migrations", () => {
    test("should run only pending migrations", () => {
      // Simulate partially migrated database
      const initialMigrations = 2;

      // Run first 2 migrations manually
      const migrations = migrationManager._getMigrations();
      for (let i = 0; i < initialMigrations; i++) {
        const migration = migrations[i];
        if (Array.isArray(migration.sql)) {
          migration.sql.forEach((sql) => db.exec(sql));
        } else {
          db.exec(migration.sql);
        }
      }
      migrationManager._setVersion(initialMigrations);

      // Verify current state
      expect(migrationManager._getCurrentVersion()).toBe(initialMigrations);

      // Run remaining migrations
      migrationManager.migrate();

      // Should now be fully up to date
      const info = migrationManager.getMigrationInfo();
      expect(info.isUpToDate).toBe(true);
      expect(info.currentVersion).toBe(info.totalMigrations);
    });

    test("should not run migrations on up-to-date database", () => {
      // First migration
      migrationManager.migrate();
      const initialVersion = migrationManager._getCurrentVersion();

      // Try to migrate again
      migrationManager.migrate();
      const finalVersion = migrationManager._getCurrentVersion();

      expect(finalVersion).toBe(initialVersion);
    });
  });

  describe("Migration Info", () => {
    test("should provide accurate migration info for fresh database", () => {
      const info = migrationManager.getMigrationInfo();

      expect(info.currentVersion).toBe(0);
      expect(info.totalMigrations).toBeGreaterThan(0);
      expect(info.isUpToDate).toBe(false);
      expect(info.pendingMigrations).toBe(info.totalMigrations);
    });

    test("should provide accurate migration info for up-to-date database", () => {
      migrationManager.migrate();
      const info = migrationManager.getMigrationInfo();

      expect(info.currentVersion).toBe(info.totalMigrations);
      expect(info.isUpToDate).toBe(true);
      expect(info.pendingMigrations).toBe(0);
    });
  });

  describe("Migration Content", () => {
    test("should create payments table with correct schema", () => {
      migrationManager.migrate();

      const tableInfo = db.prepare(`PRAGMA table_info(payments)`).all();
      const columnNames = tableInfo.map((col) => col.name);

      expect(columnNames).toContain("id");
      expect(columnNames).toContain("payment_type");
      expect(columnNames).toContain("status");
      expect(columnNames).toContain("amount");
      expect(columnNames).toContain("fees");
      expect(columnNames).toContain("timestamp");
      expect(columnNames).toContain("details");
      expect(columnNames).toContain("method");
    });

    test("should create settings table with correct schema", () => {
      migrationManager.migrate();

      const tableInfo = db.prepare(`PRAGMA table_info(settings)`).all();
      const columnNames = tableInfo.map((col) => col.name);

      expect(columnNames).toContain("key");
      expect(columnNames).toContain("value");
    });

    test("should create unclaimed_deposits table with correct schema", () => {
      migrationManager.migrate();

      const tableInfo = db
        .prepare(`PRAGMA table_info(unclaimed_deposits)`)
        .all();
      const columnNames = tableInfo.map((col) => col.name);

      expect(columnNames).toContain("txid");
      expect(columnNames).toContain("vout");
      expect(columnNames).toContain("amount_sats");
      expect(columnNames).toContain("claim_error");
      expect(columnNames).toContain("refund_tx");
      expect(columnNames).toContain("refund_tx_id");
    });

    test("should create payment_metadata table with correct schema", () => {
      migrationManager.migrate();

      const tableInfo = db.prepare(`PRAGMA table_info(payment_metadata)`).all();
      const columnNames = tableInfo.map((col) => col.name);

      expect(columnNames).toContain("payment_id");
      expect(columnNames).toContain("lnurl_pay_info");
    });

    test("should create appropriate indexes", () => {
      migrationManager.migrate();

      const indexes = db
        .prepare(
          `
                SELECT name FROM sqlite_master 
                WHERE type='index' AND name NOT LIKE 'sqlite_%'
            `
        )
        .all()
        .map((idx) => idx.name);

      expect(indexes).toContain("idx_payments_timestamp");
      expect(indexes).toContain("idx_unclaimed_deposits_txid");
      expect(indexes).toContain("idx_payment_metadata_payment_id");
    });
  });

  describe("Error Handling", () => {
    test("should handle malformed SQL gracefully", () => {
      // Create a migration manager with bad SQL
      const badMigrationManager = new MigrationManager(db, StorageError);
      badMigrationManager.migrations = [
        {
          name: "Bad migration",
          sql: "INVALID SQL STATEMENT",
        },
      ];

      expect(() => {
        badMigrationManager.migrate();
      }).toThrow();
    });

    test("should handle transaction rollback on migration failure", () => {
      // Create migration that will fail partway through
      const badMigrationManager = new MigrationManager(db, StorageError);
      badMigrationManager.migrations = [
        {
          name: "Good migration",
          sql: "CREATE TABLE test_table (id INTEGER PRIMARY KEY)",
        },
        {
          name: "Bad migration",
          sql: "INVALID SQL STATEMENT",
        },
      ];

      expect(() => {
        badMigrationManager.migrate();
      }).toThrow();

      // Database should remain unchanged
      const tables = db
        .prepare(
          `
                SELECT name FROM sqlite_master 
                WHERE type='table' AND name = 'test_table'
            `
        )
        .all();
      expect(tables).toHaveLength(0);
    });
  });

  describe("Version Management", () => {
    test("should correctly set and get version", () => {
      expect(migrationManager._getCurrentVersion()).toBe(0);

      migrationManager._setVersion(5);
      expect(migrationManager._getCurrentVersion()).toBe(5);

      migrationManager._setVersion(0);
      expect(migrationManager._getCurrentVersion()).toBe(0);
    });

    test("should persist version across database reopens", () => {
      migrationManager._setVersion(3);
      db.close();

      // Reopen database
      db = new Database(dbPath);
      const newMigrationManager = new MigrationManager(db, StorageError);

      expect(newMigrationManager._getCurrentVersion()).toBe(3);
    });
  });
});
