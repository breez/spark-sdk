/**
 * Test helpers for Node.js SQLite storage tests
 * This file is ONLY used by wasm tests, not production code
 */

const path = require("path");
const { MigrationManager } = require("./node-storage/migrations.cjs");

// Require better-sqlite3 from the node-storage package where it's installed
let Database;
try {
  // Try to require from node-storage directory (where it's installed as a dependency)
  const nodeStoragePath = path.join(
    __dirname,
    "node-storage",
    "node_modules",
    "better-sqlite3"
  );
  Database = require(nodeStoragePath);
} catch (error) {
  // Fallback: try from the main module's context
  try {
    const mainModule = require.main;
    if (mainModule) {
      Database = mainModule.require("better-sqlite3");
    } else {
      Database = require("better-sqlite3");
    }
  } catch (fallbackError) {
    throw new Error(
      `better-sqlite3 not found. Please install it in node-storage: cd js/node-storage && npm install\n` +
        `Original error: ${error.message}\nFallback error: ${fallbackError.message}`
    );
  }
}

/**
 * Creates an old v17 format SQLite database for migration testing
 * This simulates the database state before the v17→v18 migration
 * The v18 migration (index 17) adds tx_type column to payment_details_token
 */
function createOldV17Database(dbPath) {
  const db = new Database(dbPath);

  try {
    // Run real migrations 0-16 to build the schema at version 17
    const mgr = new MigrationManager(db, Error);
    mgr.migrate(17);

    // Insert test token payment WITHOUT tx_type (pre-v18 format)
    const insertPayment = db.prepare(`
      INSERT INTO payments (id, payment_type, status, amount, fees, timestamp, method, withdraw_tx_id, deposit_tx_id, spark)
      VALUES (?, ?, ?, ?, ?, ?, ?, NULL, NULL, NULL)
    `);

    const insertToken = db.prepare(`
      INSERT INTO payment_details_token (payment_id, metadata, tx_hash, invoice_details)
      VALUES (?, ?, ?, NULL)
    `);

    const tokenMetadata = JSON.stringify({
      identifier: "test-token-id",
      issuerPublicKey: "02" + "a".repeat(64),
      name: "Test Token",
      ticker: "TST",
      decimals: 8,
      maxSupply: "1000000",
      isFreezable: false,
    });

    insertPayment.run(
      "token-migration-test-payment",
      "send",
      "completed",
      "5000",
      "10",
      1234567892,
      JSON.stringify("token")
    );

    insertToken.run(
      "token-migration-test-payment",
      tokenMetadata,
      "0xabcdef1234567890"
    );

    db.close();
    return Promise.resolve();
  } catch (error) {
    db.close();
    return Promise.reject(
      new Error(`Failed to create old v17 database: ${error.message}`)
    );
  }
}

/**
 * Creates an old v20 format SQLite database for migration testing.
 * This simulates the database state before the v20→v21 migration.
 * The v21 migration (index 20) backfills htlc_details for Lightning payments.
 * The v20 migration (index 19) added the htlc_details column but left it NULL.
 */
function createOldV20Database(dbPath) {
  const db = new Database(dbPath);

  try {
    // Run real migrations 0-19 to build the schema at version 20
    const mgr = new MigrationManager(db, Error);
    mgr.migrate(20);

    // Insert test Lightning payments with different statuses
    const insertPayment = db.prepare(`
      INSERT INTO payments (id, payment_type, status, amount, fees, timestamp, method, withdraw_tx_id, deposit_tx_id, spark)
      VALUES (?, ?, ?, ?, ?, ?, ?, NULL, NULL, NULL)
    `);

    const insertLightning = db.prepare(`
      INSERT INTO payment_details_lightning (payment_id, invoice, payment_hash, destination_pubkey, preimage)
      VALUES (?, ?, ?, ?, ?)
    `);

    // Completed Lightning payment
    insertPayment.run(
      "ln-completed",
      "send",
      "completed",
      "1000",
      "10",
      1700000001,
      JSON.stringify("lightning")
    );
    insertLightning.run(
      "ln-completed",
      "lnbc_completed",
      "hash_completed_0123456789abcdef",
      "03pubkey1",
      "preimage_completed"
    );

    // Pending Lightning payment
    insertPayment.run(
      "ln-pending",
      "receive",
      "pending",
      "2000",
      "0",
      1700000002,
      JSON.stringify("lightning")
    );
    insertLightning.run(
      "ln-pending",
      "lnbc_pending",
      "hash_pending_0123456789abcdef0",
      "03pubkey2",
      null
    );

    // Failed Lightning payment
    insertPayment.run(
      "ln-failed",
      "send",
      "failed",
      "3000",
      "5",
      1700000003,
      JSON.stringify("lightning")
    );
    insertLightning.run(
      "ln-failed",
      "lnbc_failed",
      "hash_failed_0123456789abcdef01",
      "03pubkey3",
      null
    );

    db.close();
    return Promise.resolve();
  } catch (error) {
    db.close();
    return Promise.reject(
      new Error(`Failed to create old v20 database: ${error.message}`)
    );
  }
}

/**
 * Creates an old v24 format SQLite database for BigInt tagging migration testing.
 * This simulates the database state before the v24→v25 migration.
 * The v25 migration (index 24) tags u128 string values with "$BI:" prefix.
 */
function createOldV24Database(dbPath) {
  const db = new Database(dbPath);

  try {
    // Run real migrations 0-23 to build the schema at version 24
    const mgr = new MigrationManager(db, Error);
    mgr.migrate(24);

    // Insert a token payment with old-format (untagged) u128 values
    const insertPayment = db.prepare(`
      INSERT INTO payments (id, payment_type, status, amount, fees, timestamp, method, withdraw_tx_id, deposit_tx_id, spark)
      VALUES (?, ?, ?, ?, ?, ?, ?, NULL, NULL, NULL)
    `);

    const insertToken = db.prepare(`
      INSERT INTO payment_details_token (payment_id, metadata, tx_hash, tx_type, invoice_details)
      VALUES (?, ?, ?, ?, NULL)
    `);

    const insertMetadata = db.prepare(`
      INSERT INTO payment_metadata (payment_id, parent_payment_id, lnurl_pay_info, lnurl_withdraw_info, lnurl_description, conversion_info)
      VALUES (?, NULL, NULL, NULL, NULL, ?)
    `);

    // Token metadata with old-format maxSupply > u64::MAX (plain string, no $BI: prefix)
    const tokenMetadataLarge = JSON.stringify({
      identifier: "test-token-id",
      issuerPublicKey: "02" + "a".repeat(64),
      name: "Test Token",
      ticker: "TST",
      decimals: 8,
      maxSupply: "340282366920938463463374607431768211455",
      isFreezable: false,
    });

    // Token metadata with small maxSupply < u64::MAX
    const tokenMetadataSmall = JSON.stringify({
      identifier: "test-token-id-small",
      issuerPublicKey: "02" + "b".repeat(64),
      name: "Small Token",
      ticker: "SML",
      decimals: 6,
      maxSupply: "1000000",
      isFreezable: false,
    });

    // ConversionInfo with fee > u64::MAX (plain string, no $BI: prefix)
    const conversionInfoLarge = JSON.stringify({
      poolId: "pool-1",
      conversionId: "conv-1",
      status: "completed",
      fee: "18446744073709551616",
      purpose: null,
    });

    // ConversionInfo with small fee < u64::MAX
    const conversionInfoSmall = JSON.stringify({
      poolId: "pool-2",
      conversionId: "conv-2",
      status: "completed",
      fee: "500",
      purpose: null,
    });

    // Large values payment
    insertPayment.run(
      "bigint-token-payment",
      "send",
      "completed",
      "5000",
      "10",
      1700000001,
      JSON.stringify("token")
    );

    insertToken.run(
      "bigint-token-payment",
      tokenMetadataLarge,
      "0xabcdef1234567890",
      "transfer"
    );

    insertMetadata.run(
      "bigint-token-payment",
      conversionInfoLarge
    );

    // Small values payment
    insertPayment.run(
      "bigint-token-payment-small",
      "send",
      "completed",
      "1000",
      "5",
      1700000002,
      JSON.stringify("token")
    );

    insertToken.run(
      "bigint-token-payment-small",
      tokenMetadataSmall,
      "0x1234567890abcdef",
      "transfer"
    );

    insertMetadata.run(
      "bigint-token-payment-small",
      conversionInfoSmall
    );

    db.close();
    return Promise.resolve();
  } catch (error) {
    db.close();
    return Promise.reject(
      new Error(`Failed to create old v24 database: ${error.message}`)
    );
  }
}

module.exports = { createOldV17Database, createOldV20Database, createOldV24Database };
