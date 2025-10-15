/**
 * Test helpers for IndexedDB storage tests
 * This file is ONLY used by wasm tests, not production code
 */

/**
 * Creates an old v2 format IndexedDB database for migration testing
 * This simulates the database state before the v2→v3 migration
 */
export async function createOldV2Database(dbName) {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(dbName, 2);

    request.onupgradeneeded = (event) => {
      const db = event.target.result;
      const transaction = event.target.transaction;

      // Create payments store if needed (v1 migration)
      if (!db.objectStoreNames.contains("payments")) {
        const paymentStore = db.createObjectStore("payments", {
          keyPath: "id",
        });
        paymentStore.createIndex("timestamp", "timestamp", { unique: false });
        paymentStore.createIndex("paymentType", "paymentType", {
          unique: false,
        });
        paymentStore.createIndex("status", "status", { unique: false });
      }

      // Create payment_metadata store
      if (!db.objectStoreNames.contains("payment_metadata")) {
        db.createObjectStore("payment_metadata", { keyPath: "paymentId" });
      }

      // Create unclaimed_deposits store
      if (!db.objectStoreNames.contains("unclaimed_deposits")) {
        const depositStore = db.createObjectStore("unclaimed_deposits", {
          keyPath: ["txid", "vout"],
        });
        depositStore.createIndex("txid", "txid", { unique: false });
      }

      // Create invoice index (v2 migration)
      const paymentStore = transaction.objectStore("payments");
      if (!paymentStore.indexNames.contains("invoice")) {
        paymentStore.createIndex("invoice", "details.invoice", {
          unique: false,
        });
      }

      // Insert test payment with OLD Number format
      paymentStore.put({
        id: "migration-test-payment",
        paymentType: "send",
        status: "completed",
        amount: 1000.0, // OLD: Number (will have .0)
        fees: 50.0, // OLD: Number (will have .0)
        timestamp: 1234567890,
        details: JSON.stringify({ type: "spark" }),
        method: JSON.stringify("spark"),
      });
    };

    request.onsuccess = () => {
      request.result.close();
      resolve();
    };

    request.onerror = () => {
      reject(
        new Error(`Failed to create old database: ${request.error?.message}`)
      );
    };
  });
}
