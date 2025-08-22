/**
 * Tests for IndexedDB Storage Implementation
 */

// Set up IndexedDB polyfill for Node.js testing
import "fake-indexeddb/auto";

// Polyfill structuredClone for older Node.js versions
if (typeof global.structuredClone === "undefined") {
  global.structuredClone = (obj) => {
    return JSON.parse(JSON.stringify(obj));
  };
}

import {
  IndexedDBStorage,
  createDefaultStorage,
  StorageError,
} from "../index.js";

describe("IndexedDBStorage", () => {
  let storage;

  beforeEach(async () => {
    // Create a unique database name for each test to ensure isolation
    const dbName = `TestBreezSDK-${Date.now()}-${Math.random()
      .toString(36)
      .substr(2, 9)}`;
    storage = await createDefaultStorage(dbName);
  });

  afterEach(() => {
    if (storage) {
      storage.close();
    }
  });

  describe("Cache Operations", () => {
    test("should store and retrieve cached items", async () => {
      await storage.setCachedItem("test-key", "test-value");
      const value = await storage.getCachedItem("test-key");
      expect(value).toBe("test-value");
    });

    test("should return null for non-existent cached items", async () => {
      const value = await storage.getCachedItem("non-existent-key");
      expect(value).toBeNull();
    });

    test("should update existing cached items", async () => {
      await storage.setCachedItem("update-key", "original-value");
      await storage.setCachedItem("update-key", "updated-value");
      const value = await storage.getCachedItem("update-key");
      expect(value).toBe("updated-value");
    });
  });

  describe("Payment Operations", () => {
    const samplePayment = {
      id: "payment-123",
      paymentType: "Lightning",
      status: "Complete",
      amount: 1000,
      fees: 10,
      timestamp: Date.now(),
      method: "Lightning",
      details: { bolt11: "lnbc123..." },
    };

    test("should insert and retrieve payment by id", async () => {
      await storage.insertPayment(samplePayment);
      const payment = await storage.getPaymentById("payment-123");

      expect(payment.id).toBe(samplePayment.id);
      expect(payment.paymentType).toBe(samplePayment.paymentType);
      expect(payment.amount).toBe(samplePayment.amount);
      expect(payment.method).toBe(samplePayment.method);
      expect(typeof payment.details).toBe("object");
    });

    test("should throw error for non-existent payment", async () => {
      await expect(storage.getPaymentById("non-existent")).rejects.toThrow(
        StorageError
      );
    });

    test("should list payments with pagination", async () => {
      // Insert multiple payments with different timestamps
      const payments = [];
      for (let i = 0; i < 5; i++) {
        const payment = {
          ...samplePayment,
          id: `payment-${i}`,
          timestamp: Date.now() + i * 1000,
          method: "Lightning",
        };
        payments.push(payment);
        await storage.insertPayment(payment);
      }

      // Test listing with limit
      const listedPayments = await storage.listPayments(0, 3);
      expect(listedPayments).toHaveLength(3);

      // Should be ordered by timestamp descending (newest first)
      expect(listedPayments[0].id).toBe("payment-4");
      expect(listedPayments[1].id).toBe("payment-3");
      expect(listedPayments[2].id).toBe("payment-2");
    });

    test("should set and retrieve payment metadata", async () => {
      // Create a Lightning payment with proper structure
      const lightningPayment = {
        ...samplePayment,
        details: { Lightning: { bolt11: "lnbc123..." } },
      };

      await storage.insertPayment(lightningPayment);

      const metadata = {
        lnurlPayInfo: { domain: "example.com", description: "Test payment" },
      };

      await storage.setPaymentMetadata("payment-123", metadata);
      const payment = await storage.getPaymentById("payment-123");

      expect(payment.details.Lightning.lnurlPayInfo).toEqual(
        metadata.lnurlPayInfo
      );
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

    test("should replace existing deposits with same txid:vout", async () => {
      await storage.addDeposit("test_tx_1", 0, 50000);
      await storage.addDeposit("test_tx_1", 0, 75000); // Same txid:vout, different amount

      const deposits = await storage.listDeposits();
      expect(deposits).toHaveLength(1);
      expect(deposits[0].amountSats).toBe(75000); // Latest amount preserved (IndexedDB put replaces)
    });
  });

  describe("Error Handling", () => {
    test("should throw StorageError for invalid operations", async () => {
      storage.close(); // Close the database to trigger errors

      await expect(storage.getCachedItem("test")).rejects.toThrow(StorageError);
    });
  });

  describe("Database Creation", () => {
    test("should create storage with custom database name", async () => {
      const customStorage = await createDefaultStorage("CustomDB");
      expect(customStorage.dbName).toBe("CustomDB");
      customStorage.close();
    });

    test("should throw error if IndexedDB is not available", async () => {
      const originalIndexedDB = global.indexedDB;
      global.indexedDB = undefined;

      await expect(createDefaultStorage("TestDB")).rejects.toThrow(
        StorageError
      );

      global.indexedDB = originalIndexedDB;
    });
  });
});
