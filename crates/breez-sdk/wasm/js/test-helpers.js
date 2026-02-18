/**
 * Test helpers for IndexedDB storage tests
 * This file is ONLY used by wasm tests, not production code
 */

import { MigrationManager, StorageError } from "./web-storage/index.js";

/**
 * Opens an IndexedDB database at a specific version, running real migrations
 * up to that version. Returns the open database handle.
 */
function openDatabaseAtVersion(dbName, version) {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(dbName, version);

    request.onupgradeneeded = (event) => {
      const mgr = new MigrationManager(null, StorageError);
      mgr.handleUpgrade(event, event.oldVersion, event.newVersion);
    };

    request.onsuccess = () => resolve(request.result);

    request.onerror = () => {
      reject(
        new Error(`Failed to open database at v${version}: ${request.error?.message}`)
      );
    };
  });
}

/**
 * Creates an old v2 format IndexedDB database for migration testing
 * This simulates the database state before the v2→v3 migration
 */
export async function createOldV2Database(dbName) {
  // Run real migrations 0-1 to build schema at version 2
  const db = await openDatabaseAtVersion(dbName, 2);

  return new Promise((resolve, reject) => {
    const tx = db.transaction("payments", "readwrite");
    const paymentStore = tx.objectStore("payments");

    // Insert test payment with OLD Number format (migration 2 will convert to BigInt)
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

    tx.oncomplete = () => { db.close(); resolve(); };
    tx.onerror = () => { db.close(); reject(new Error(`Failed to create old v2 database: ${tx.error?.message}`)); };
  });
}

/**
 * Creates an old v8 format IndexedDB database for migration testing
 * This simulates the database state before the v8→v9 migration
 * The v9 migration adds txType field to token payments
 */
export async function createOldV8Database(dbName) {
  // Run real migrations 0-7 to build schema at version 8
  const db = await openDatabaseAtVersion(dbName, 8);

  return new Promise((resolve, reject) => {
    const tx = db.transaction("payments", "readwrite");
    const paymentStore = tx.objectStore("payments");

    // Insert test token payment WITHOUT txType (pre-v9 format)
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

    tx.oncomplete = () => { db.close(); resolve(); };
    tx.onerror = () => { db.close(); reject(new Error(`Failed to create old v8 database: ${tx.error?.message}`)); };
  });
}

/**
 * Creates an old v10 format IndexedDB database for migration testing
 * This simulates the database state before the v10→v11 migration
 * The v11 migration backfills htlcDetails for Lightning payments
 */
export async function createOldV10Database(dbName) {
  // Run real migrations 0-9 to build schema at version 10
  const db = await openDatabaseAtVersion(dbName, 10);

  return new Promise((resolve, reject) => {
    const tx = db.transaction("payments", "readwrite");
    const paymentStore = tx.objectStore("payments");

    // Insert test Lightning payments WITHOUT htlcDetails (pre-v11 format)

    // Completed Lightning payment
    paymentStore.put({
      id: "ln-completed",
      paymentType: "send",
      status: "completed",
      amount: BigInt(1000),
      fees: BigInt(10),
      timestamp: 1700000001,
      details: JSON.stringify({
        type: "lightning",
        invoice: "lnbc_completed",
        paymentHash: "hash_completed_0123456789abcdef",
        destinationPubkey: "03pubkey1",
        preimage: "preimage_completed",
        // NOTE: htlcDetails is missing - this is what migration 10 will add
      }),
      method: JSON.stringify("lightning"),
    });

    // Pending Lightning payment
    paymentStore.put({
      id: "ln-pending",
      paymentType: "receive",
      status: "pending",
      amount: BigInt(2000),
      fees: BigInt(0),
      timestamp: 1700000002,
      details: JSON.stringify({
        type: "lightning",
        invoice: "lnbc_pending",
        paymentHash: "hash_pending_0123456789abcdef0",
        destinationPubkey: "03pubkey2",
        preimage: null,
      }),
      method: JSON.stringify("lightning"),
    });

    // Failed Lightning payment
    paymentStore.put({
      id: "ln-failed",
      paymentType: "send",
      status: "failed",
      amount: BigInt(3000),
      fees: BigInt(5),
      timestamp: 1700000003,
      details: JSON.stringify({
        type: "lightning",
        invoice: "lnbc_failed",
        paymentHash: "hash_failed_0123456789abcdef01",
        destinationPubkey: "03pubkey3",
        preimage: null,
      }),
      method: JSON.stringify("lightning"),
    });

    tx.oncomplete = () => { db.close(); resolve(); };
    tx.onerror = () => { db.close(); reject(new Error(`Failed to create old v10 database: ${tx.error?.message}`)); };
  });
}
