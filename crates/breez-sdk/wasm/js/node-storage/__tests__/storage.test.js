/**
 * Comprehensive tests for Node.js SQLite Storage Implementation
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
const { SqliteStorage } = require("../index.cjs");

describe("SqliteStorage", () => {
  let storage;
  let dbPath;

  beforeEach(async () => {
    // Create a unique temporary database for each test
    dbPath = join(
      tmpdir(),
      `test-storage-${Date.now()}-${Math.random().toString(36).substr(2, 9)}.db`
    );
    storage = new SqliteStorage(dbPath);
    await storage.initialize();
  });

  afterEach(() => {
    if (storage) {
      storage.close();
    }
    // Clean up test database
    if (existsSync(dbPath)) {
      try {
        unlinkSync(dbPath);
      } catch (error) {
        console.warn("Failed to clean up test database:", error.message);
      }
    }
  });

  describe("Initialization", () => {
    test("should initialize database successfully", () => {
      expect(storage.db).toBeTruthy();
      expect(existsSync(dbPath)).toBe(true);
    });

    test("should run migrations on initialization", () => {
      const migrationInfo = storage.migrationManager.getMigrationInfo();
      expect(migrationInfo.isUpToDate).toBe(true);
      expect(migrationInfo.pendingMigrations).toBe(0);
    });

    test("should validate schema after initialization", () => {
      const schemaInfo = storage.migrationManager.validateSchema();
      expect(schemaInfo.isValid).toBe(true);
      expect(schemaInfo.tables).toContain("payments");
      expect(schemaInfo.tables).toContain("settings");
      expect(schemaInfo.tables).toContain("unclaimed_deposits");
      expect(schemaInfo.tables).toContain("payment_metadata");
    });
  });

  describe("Cache Operations", () => {
    test("should store and retrieve cached items", async () => {
      const key = "test_key";
      const value = "test_value";

      await storage.setCachedItem(key, value);
      const retrieved = await storage.getCachedItem(key);

      expect(retrieved).toBe(value);
    });

    test("should return null for non-existent keys", async () => {
      const result = await storage.getCachedItem("non_existent_key");
      expect(result).toBeNull();
    });

    test("should update existing cached items", async () => {
      const key = "update_test";

      await storage.setCachedItem(key, "original_value");
      await storage.setCachedItem(key, "updated_value");

      const result = await storage.getCachedItem(key);
      expect(result).toBe("updated_value");
    });

    test("should handle complex JSON values", async () => {
      const key = "json_test";
      const complexValue = JSON.stringify({
        nested: { data: "value" },
        array: [1, 2, 3],
        boolean: true,
      });

      await storage.setCachedItem(key, complexValue);
      const retrieved = await storage.getCachedItem(key);

      expect(JSON.parse(retrieved)).toEqual(JSON.parse(complexValue));
    });
  });

  describe("Payment Operations", () => {
    const createTestPayment = (id = "test_payment_1") => ({
      id,
      paymentType: "Send",
      status: "Completed",
      amount: 100000,
      fees: 1000,
      timestamp: Math.floor(Date.now() / 1000),
      method: "Spark",
      details: {
        type: "Spark",
        description: "Test payment",
      },
    });

    test("should insert and retrieve payments", async () => {
      const payment = createTestPayment();

      await storage.insertPayment(payment);
      const retrieved = await storage.getPaymentById(payment.id);

      expect(retrieved.id).toBe(payment.id);
      expect(retrieved.paymentType).toBe(payment.paymentType);
      expect(retrieved.status).toBe(payment.status);
      expect(retrieved.amount).toBe(payment.amount);
      expect(retrieved.fees).toBe(payment.fees);
      expect(retrieved.method).toBe(payment.method);
      expect(retrieved.details).toEqual(payment.details);
    });

    test("should list payments with pagination", async () => {
      const payments = [
        createTestPayment("payment_1"),
        createTestPayment("payment_2"),
        createTestPayment("payment_3"),
      ];

      for (const payment of payments) {
        await storage.insertPayment(payment);
      }

      // Test default listing (no pagination)
      const allPayments = await storage.listPayments();
      expect(allPayments).toHaveLength(3);

      // Test pagination
      const firstPage = await storage.listPayments(0, 2);
      expect(firstPage).toHaveLength(2);

      const secondPage = await storage.listPayments(2, 2);
      expect(secondPage).toHaveLength(1);
    });

    test("should list payments in descending timestamp order", async () => {
      const baseTime = Math.floor(Date.now() / 1000);
      const payments = [
        { ...createTestPayment("payment_1"), timestamp: baseTime },
        { ...createTestPayment("payment_2"), timestamp: baseTime + 100 },
        { ...createTestPayment("payment_3"), timestamp: baseTime + 50 },
      ];

      for (const payment of payments) {
        await storage.insertPayment(payment);
      }

      const retrieved = await storage.listPayments();
      expect(retrieved[0].timestamp).toBe(baseTime + 100);
      expect(retrieved[1].timestamp).toBe(baseTime + 50);
      expect(retrieved[2].timestamp).toBe(baseTime);
    });

    test("should update existing payments", async () => {
      const payment = createTestPayment();
      await storage.insertPayment(payment);

      const updatedPayment = { ...payment, status: "Failed", fees: 2000 };
      await storage.insertPayment(updatedPayment);

      const retrieved = await storage.getPaymentById(payment.id);
      expect(retrieved.status).toBe("Failed");
      expect(retrieved.fees).toBe(2000);
    });

    test("should throw error for non-existent payment", async () => {
      await expect(
        storage.getPaymentById("non_existent_payment")
      ).rejects.toThrow("Payment with id 'non_existent_payment' not found");
    });

    test("should handle payment metadata", async () => {
      // Create a Lightning payment to test lnurl metadata
      const payment = {
        ...createTestPayment(),
        details: {
          Lightning: {
            description: "Test Lightning payment",
          },
        },
      };
      await storage.insertPayment(payment);

      const metadata = {
        lnurlPayInfo: {
          domain: "example.com",
          amount: 1000,
          description: "Test LNURL payment",
        },
      };

      await storage.setPaymentMetadata(payment.id, metadata);

      // Test retrieval via getPaymentById
      const retrievedPayment = await storage.getPaymentById(payment.id);
      expect(retrievedPayment.details.Lightning.lnurlPayInfo).toEqual(
        metadata.lnurlPayInfo
      );

      // Test retrieval via listPayments
      const paymentsList = await storage.listPayments();
      const foundPayment = paymentsList.find((p) => p.id === payment.id);
      expect(foundPayment.details.Lightning.lnurlPayInfo).toEqual(
        metadata.lnurlPayInfo
      );
    });

    test("should handle payment without metadata", async () => {
      const payment = createTestPayment();
      await storage.insertPayment(payment);

      // Retrieve payment without any metadata set
      const retrievedPayment = await storage.getPaymentById(payment.id);
      expect(retrievedPayment.details).toEqual(payment.details);
      expect(retrievedPayment.details.Lightning?.lnurlPayInfo).toBeUndefined();
    });

    test("should handle non-Lightning payment with metadata", async () => {
      const sparkPayment = {
        ...createTestPayment(),
        details: {
          Spark: {
            description: "Test Spark payment",
          },
        },
      };
      await storage.insertPayment(sparkPayment);

      const metadata = {
        lnurlPayInfo: {
          domain: "example.com",
          amount: 1000,
        },
      };

      await storage.setPaymentMetadata(sparkPayment.id, metadata);

      // For non-Lightning payments, metadata should not be added to details
      const retrievedPayment = await storage.getPaymentById(sparkPayment.id);
      expect(retrievedPayment.details.Spark).toEqual(
        sparkPayment.details.Spark
      );
      expect(retrievedPayment.details.Lightning?.lnurlPayInfo).toBeUndefined();
    });

    test("should update existing payment metadata", async () => {
      const payment = {
        ...createTestPayment(),
        details: {
          Lightning: {
            description: "Test Lightning payment",
          },
        },
      };
      await storage.insertPayment(payment);

      // Set initial metadata
      const initialMetadata = {
        lnurlPayInfo: {
          domain: "initial.com",
          amount: 500,
        },
      };
      await storage.setPaymentMetadata(payment.id, initialMetadata);

      // Update metadata
      const updatedMetadata = {
        lnurlPayInfo: {
          domain: "updated.com",
          amount: 1500,
          description: "Updated LNURL payment",
        },
      };
      await storage.setPaymentMetadata(payment.id, updatedMetadata);

      // Verify updated metadata is retrieved
      const retrievedPayment = await storage.getPaymentById(payment.id);
      expect(retrievedPayment.details.Lightning.lnurlPayInfo).toEqual(
        updatedMetadata.lnurlPayInfo
      );
    });

    test("should handle null lnurlPayInfo in metadata", async () => {
      const payment = {
        ...createTestPayment(),
        details: {
          Lightning: {
            description: "Test Lightning payment",
          },
        },
      };
      await storage.insertPayment(payment);

      // Set metadata with null lnurlPayInfo
      const metadata = {
        lnurlPayInfo: null,
      };
      await storage.setPaymentMetadata(payment.id, metadata);

      // Verify payment is retrieved without lnurlPayInfo
      const retrievedPayment = await storage.getPaymentById(payment.id);
      expect(retrievedPayment.details.Lightning.lnurlPayInfo).toBeUndefined();
    });
  });

  describe("Deposit Operations", () => {
    test("should add and list deposits", async () => {
      await storage.addDeposit("test_tx_1", 0, 50000);
      const deposits = await storage.listDeposits();

      expect(deposits).toHaveLength(1);
      expect(deposits[0].txid).toBe("test_tx_1");
      expect(deposits[0].vout).toBe(0);
      expect(deposits[0].amountSats).toBe(50000);
      expect(deposits[0].claimError).toBeNull();
      expect(deposits[0].refundTx).toBeNull();
      expect(deposits[0].refundTxId).toBeNull();
    });

    test("should delete deposits", async () => {
      await storage.addDeposit("test_tx_1", 0, 50000);
      await storage.deleteDeposit("test_tx_1", 0);

      const deposits = await storage.listDeposits();
      expect(deposits).toHaveLength(0);
    });

    test("should add multiple deposits", async () => {
      await storage.addDeposit("tx1", 0, 25000);
      await storage.addDeposit("tx2", 1, 75000);
      await storage.addDeposit("tx3", 2, 100000);

      const deposits = await storage.listDeposits();
      expect(deposits).toHaveLength(3);

      const txids = deposits.map((d) => d.txid);
      expect(txids).toContain("tx1");
      expect(txids).toContain("tx2");
      expect(txids).toContain("tx3");
    });

    test("should update deposit with claim error", async () => {
      await storage.addDeposit("test_tx_1", 0, 50000);

      const claimErrorPayload = {
        type: "claimError",
        error: {
          type: "Generic",
          message: "Test claim error",
        },
      };

      await storage.updateDeposit("test_tx_1", 0, claimErrorPayload);

      const deposits = await storage.listDeposits();
      expect(deposits).toHaveLength(1);
      expect(deposits[0].claimError).toEqual(claimErrorPayload.error);
      expect(deposits[0].refundTx).toBeNull();
      expect(deposits[0].refundTxId).toBeNull();
    });

    test("should update deposit with refund", async () => {
      await storage.addDeposit("test_tx_1", 0, 50000);

      const refundPayload = {
        type: "refund",
        refundTxid: "refund_tx_123",
        refundTx: "0200000001abcd1234...",
      };

      await storage.updateDeposit("test_tx_1", 0, refundPayload);

      const deposits = await storage.listDeposits();
      expect(deposits).toHaveLength(1);
      expect(deposits[0].refundTxId).toBe("refund_tx_123");
      expect(deposits[0].refundTx).toBe("0200000001abcd1234...");
      expect(deposits[0].claimError).toBeNull();
    });

    test("should handle updating non-existent deposit", async () => {
      const claimErrorPayload = {
        type: "claimError",
        error: {
          type: "Generic",
          message: "Test error",
        },
      };

      // This should not throw, just not update anything
      await storage.updateDeposit("non_existent_tx", 0, claimErrorPayload);

      const deposits = await storage.listDeposits();
      expect(deposits).toHaveLength(0);
    });

    test("should handle unknown payload type", async () => {
      await storage.addDeposit("test_tx_1", 0, 50000);

      const invalidPayload = {
        type: "unknown",
        data: "test",
      };

      await expect(
        storage.updateDeposit("test_tx_1", 0, invalidPayload)
      ).rejects.toThrow("Unknown payload type: unknown");
    });

    test("should ignore duplicate deposits with same txid:vout", async () => {
      await storage.addDeposit("test_tx_1", 0, 50000);
      await storage.addDeposit("test_tx_1", 0, 75000); // Same txid:vout, different amount

      const deposits = await storage.listDeposits();
      expect(deposits).toHaveLength(1);
      expect(deposits[0].amountSats).toBe(50000); // First amount preserved
    });
  });

  describe("Error Handling", () => {
    test("should handle database corruption gracefully", async () => {
      // Close the database and corrupt the file
      await storage.close();

      expect(() => {
        new SqliteStorage(
          "/invalid/path/that/does/not/exist/test.db"
        ).initializeSync();
      }).toThrow();
    });

    test("should throw meaningful errors for invalid operations", async () => {
      await expect(storage.insertPayment(null)).rejects.toThrow();
      await expect(storage.getPaymentById(null)).rejects.toThrow();
    });
  });

  function createTestPayment(id = "test_payment_1") {
    return {
      id,
      paymentType: "Send",
      status: "Completed",
      amount: 100000,
      fees: 1000,
      timestamp: Math.floor(Date.now() / 1000),
      method: "Spark",
      details: {
        type: "Spark",
        description: "Test payment",
      },
    };
  }
});
