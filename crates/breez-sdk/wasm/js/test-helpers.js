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

/**
 * Creates an old v8 format IndexedDB database for migration testing
 * This simulates the database state before the v8→v9 migration
 * The v9 migration adds txType field to token payments
 */
export async function createOldV8Database(dbName) {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(dbName, 8);

    request.onupgradeneeded = (event) => {
      const db = event.target.result;
      const transaction = event.target.transaction;

      // Create all stores that exist in v7
      // Settings store
      if (!db.objectStoreNames.contains("settings")) {
        db.createObjectStore("settings", { keyPath: "key" });
      }

      // Payments store with indexes
      if (!db.objectStoreNames.contains("payments")) {
        const paymentStore = db.createObjectStore("payments", {
          keyPath: "id",
        });
        paymentStore.createIndex("timestamp", "timestamp", { unique: false });
        paymentStore.createIndex("paymentType", "paymentType", {
          unique: false,
        });
        paymentStore.createIndex("status", "status", { unique: false });
        paymentStore.createIndex("invoice", "details.invoice", {
          unique: false,
        });
      }

      // Payment metadata store (with parentPaymentId index added in v8)
      if (!db.objectStoreNames.contains("payment_metadata")) {
        const metadataStore = db.createObjectStore("payment_metadata", { keyPath: "paymentId" });
        metadataStore.createIndex("parentPaymentId", "parentPaymentId", { unique: false });
      }

      // Unclaimed deposits store
      if (!db.objectStoreNames.contains("unclaimed_deposits")) {
        const depositStore = db.createObjectStore("unclaimed_deposits", {
          keyPath: ["txid", "vout"],
        });
        depositStore.createIndex("txid", "txid", { unique: false });
      }

      // Sync tables (added in v4)
      if (!db.objectStoreNames.contains("sync_revision")) {
        db.createObjectStore("sync_revision", { keyPath: "id" });
        transaction.objectStore("sync_revision").add({ id: 1, revision: "0" });
      }

      if (!db.objectStoreNames.contains("sync_outgoing")) {
        const outgoingStore = db.createObjectStore("sync_outgoing", {
          keyPath: ["type", "dataId", "revision"],
        });
        outgoingStore.createIndex("revision", "revision");
      }

      if (!db.objectStoreNames.contains("sync_incoming")) {
        const incomingStore = db.createObjectStore("sync_incoming", {
          keyPath: ["type", "dataId", "revision"],
        });
        incomingStore.createIndex("revision", "revision");
      }

      if (!db.objectStoreNames.contains("sync_state")) {
        db.createObjectStore("sync_state", { keyPath: ["type", "dataId"] });
      }

      // Lnurl receive metadata store (added in v5)
      if (!db.objectStoreNames.contains("lnurl_receive_metadata")) {
        db.createObjectStore("lnurl_receive_metadata", {
          keyPath: "paymentHash",
        });
      }

      // Insert test token payment WITHOUT txType (pre-v8 format)
      const paymentStore = transaction.objectStore("payments");
      paymentStore.put({
        id: "token-migration-test-payment",
        paymentType: "send",
        status: "completed",
        amount: BigInt(5000),
        fees: BigInt(10),
        timestamp: 1234567892,
        details: JSON.stringify({
          type: "token",
          metadata: {
            identifier: "test-token-id",
            issuerPublicKey: "02" + "a".repeat(64),
            name: "Test Token",
            ticker: "TST",
            decimals: 8,
            maxSupply: "1000000",
            isFreezable: false,
          },
          txHash: "0xabcdef1234567890",
          // NOTE: txType is missing - this is what migration 8 will add
        }),
        method: JSON.stringify("token"),
      });
    };

    request.onsuccess = () => {
      request.result.close();
      resolve();
    };

    request.onerror = () => {
      reject(
        new Error(`Failed to create old v7 database: ${request.error?.message}`)
      );
    };
  });
}
