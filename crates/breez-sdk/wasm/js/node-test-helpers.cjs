/**
 * Test helpers for Node.js SQLite storage tests
 * This file is ONLY used by wasm tests, not production code
 */

const path = require("path");

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
    // Create the exact schema that should exist at v18 (before tx_type migration)
    // Need all payment detail tables because queries JOIN against them
    const transaction = db.transaction(() => {
      // Payments table (with TEXT amount/fees from migration 7)
      db.exec(`
        CREATE TABLE payments (
          id TEXT PRIMARY KEY,
          payment_type TEXT NOT NULL,
          status TEXT NOT NULL,
          amount TEXT NOT NULL,
          fees TEXT NOT NULL,
          timestamp INTEGER NOT NULL,
          method TEXT,
          withdraw_tx_id TEXT,
          deposit_tx_id TEXT,
          spark INTEGER
        )
      `);
      db.exec(
        `CREATE INDEX idx_payments_timestamp ON payments(timestamp DESC)`
      );

      // Payment details token table WITHOUT tx_type column (pre-v18)
      db.exec(`
        CREATE TABLE payment_details_token (
          payment_id TEXT PRIMARY KEY,
          metadata TEXT,
          tx_hash TEXT,
          invoice_details TEXT,
          FOREIGN KEY (payment_id) REFERENCES payments(id) ON DELETE CASCADE
        )
      `);

      // Settings table
      db.exec(`
        CREATE TABLE settings (
          key TEXT PRIMARY KEY,
          value TEXT NOT NULL
        )
      `);

      // Payment metadata table (with conversion_info from migration 16)
      db.exec(`
        CREATE TABLE payment_metadata (
          payment_id TEXT PRIMARY KEY,
          lnurl_pay_info TEXT,
          lnurl_description TEXT,
          lnurl_withdraw_info TEXT,
          parent_payment_id TEXT,
          conversion_info TEXT,
          FOREIGN KEY (payment_id) REFERENCES payments(id) ON DELETE CASCADE
        )
      `);

      // Payment details lightning table (needed for JOINs in queries)
      db.exec(`
        CREATE TABLE payment_details_lightning (
          payment_id TEXT PRIMARY KEY,
          invoice TEXT NOT NULL,
          payment_hash TEXT NOT NULL,
          destination_pubkey TEXT NOT NULL,
          description TEXT,
          preimage TEXT,
          FOREIGN KEY (payment_id) REFERENCES payments(id) ON DELETE CASCADE
        )
      `);
      db.exec(
        `CREATE INDEX idx_payment_details_lightning_invoice ON payment_details_lightning(invoice)`
      );

      // Payment details spark table (needed for JOINs in queries)
      db.exec(`
        CREATE TABLE payment_details_spark (
          payment_id TEXT NOT NULL PRIMARY KEY,
          invoice_details TEXT,
          htlc_details TEXT,
          FOREIGN KEY (payment_id) REFERENCES payments(id) ON DELETE CASCADE
        )
      `);

      // LNURL receive metadata table (needed for JOINs in queries)
      db.exec(`
        CREATE TABLE lnurl_receive_metadata (
          payment_hash TEXT NOT NULL PRIMARY KEY,
          nostr_zap_request TEXT,
          nostr_zap_receipt TEXT,
          sender_comment TEXT
        )
      `);

      // Sync tables (created by migration 9, needed by migrations 18-19)
      db.exec(`
        CREATE TABLE sync_revision (
          revision INTEGER NOT NULL DEFAULT 0
        )
      `);
      db.exec(`INSERT INTO sync_revision (revision) VALUES (0)`);
      db.exec(`
        CREATE TABLE sync_outgoing (
          record_type TEXT NOT NULL,
          data_id TEXT NOT NULL,
          schema_version TEXT NOT NULL,
          commit_time INTEGER NOT NULL,
          updated_fields_json TEXT NOT NULL,
          revision INTEGER NOT NULL
        )
      `);
      db.exec(`CREATE INDEX idx_sync_outgoing_data_id_record_type ON sync_outgoing(record_type, data_id)`);
      db.exec(`
        CREATE TABLE sync_state (
          record_type TEXT NOT NULL,
          data_id TEXT NOT NULL,
          schema_version TEXT NOT NULL,
          commit_time INTEGER NOT NULL,
          data TEXT NOT NULL,
          revision INTEGER NOT NULL,
          PRIMARY KEY (record_type, data_id)
        )
      `);
      db.exec(`
        CREATE TABLE sync_incoming (
          record_type TEXT NOT NULL,
          data_id TEXT NOT NULL,
          schema_version TEXT NOT NULL,
          commit_time INTEGER NOT NULL,
          data TEXT NOT NULL,
          revision INTEGER NOT NULL,
          PRIMARY KEY (record_type, data_id, revision)
        )
      `);
      db.exec(`CREATE INDEX idx_sync_incoming_revision ON sync_incoming(revision)`);

      // Set database version to 17 (before tx_type migration at index 17)
      db.pragma("user_version = 17");
    });

    transaction();

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

module.exports = { createOldV17Database };
