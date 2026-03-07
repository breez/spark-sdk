/**
 * Test helpers for PostgreSQL storage tests.
 * This file is ONLY used by wasm tests, not production code.
 *
 * Requires Docker to be running — spins up a PostgreSQL container via testcontainers.
 */

const path = require("path");

// Resolve dependencies from the postgres-storage package where they're installed
let GenericContainer, Wait, Pool;
try {
  const pgStoragePath = path.join(__dirname, "postgres-storage", "node_modules");
  const tc = require(path.join(pgStoragePath, "testcontainers"));
  GenericContainer = tc.GenericContainer;
  Wait = tc.Wait;
  Pool = require(path.join(pgStoragePath, "pg")).Pool;
} catch (error) {
  try {
    const mainModule = require.main;
    if (mainModule) {
      const tc = mainModule.require("testcontainers");
      GenericContainer = tc.GenericContainer;
      Wait = tc.Wait;
      Pool = mainModule.require("pg").Pool;
    } else {
      const tc = require("testcontainers");
      GenericContainer = tc.GenericContainer;
      Wait = tc.Wait;
      Pool = require("pg").Pool;
    }
  } catch (fallbackError) {
    throw new Error(
      `testcontainers or pg not found. Please install them in postgres-storage: cd js/postgres-storage && npm install\n` +
        `Original error: ${error.message}\nFallback error: ${fallbackError.message}`
    );
  }
}

const PG_IMAGE = "postgres:16-alpine";
const PG_USER = "test";
const PG_PASSWORD = "test";
const PG_PORT = 5432;

let _container = null;
let _containerHost = null;
let _containerPort = null;

/**
 * Ensure a shared PostgreSQL container is running.
 * Returns { host, port } for connection.
 */
async function ensureContainer() {
  if (_container) {
    return { host: _containerHost, port: _containerPort };
  }

  _container = await new GenericContainer(PG_IMAGE)
    .withEnvironment({
      POSTGRES_USER: PG_USER,
      POSTGRES_PASSWORD: PG_PASSWORD,
      POSTGRES_DB: "template_test",
    })
    .withExposedPorts(PG_PORT)
    .withWaitStrategy(
      Wait.forLogMessage(/database system is ready to accept connections/, 2)
    )
    .start();

  _containerHost = _container.getHost();
  _containerPort = _container.getMappedPort(PG_PORT);

  return { host: _containerHost, port: _containerPort };
}

let _dbCounter = 0;

/**
 * Create a fresh database for a single test, returning a connection string.
 * Each database name is unique to avoid cross-test interference.
 *
 * @param {string} testName - Used as part of the database name (sanitised)
 * @returns {Promise<string>} PostgreSQL connection string
 */
async function createTestConnectionString(testName) {
  const { host, port } = await ensureContainer();

  // Sanitise into a valid PG identifier
  const safeName = testName.replace(/[^a-z0-9_]/gi, "_").toLowerCase();
  _dbCounter++;
  const dbName = `test_${safeName}_${_dbCounter}`;

  // Connect to the default database to CREATE DATABASE
  const adminPool = new Pool({
    host,
    port,
    user: PG_USER,
    password: PG_PASSWORD,
    database: "template_test",
    max: 1,
  });

  try {
    await adminPool.query(`CREATE DATABASE "${dbName}"`);
  } finally {
    await adminPool.end();
  }

  return `postgres://${PG_USER}:${PG_PASSWORD}@${host}:${port}/${dbName}`;
}

/**
 * Creates a PostgreSQL database seeded with old-format (untagged) u128 data
 * at migration version 1, for BigInt tagging migration testing.
 * Returns the connection string for the test to open storage (triggering migration 2).
 */
async function createOldV1Database(testName) {
  const connString = await createTestConnectionString(testName);

  const pool = new Pool({ connectionString: connString, max: 1 });
  const client = await pool.connect();
  try {
    await client.query("BEGIN");

    // Create migrations tracking table and run migration 1 SQL
    await client.query(`
      CREATE TABLE IF NOT EXISTS schema_migrations (
        version INTEGER PRIMARY KEY,
        applied_at TIMESTAMPTZ DEFAULT NOW()
      )
    `);

    const { PostgresMigrationManager } = require("./postgres-storage/migrations.cjs");
    const mgr = new PostgresMigrationManager();
    const migrations = mgr._getMigrations();

    // Run only migration 1 (index 0)
    for (const sql of migrations[0].sql) {
      await client.query(sql);
    }
    await client.query("INSERT INTO schema_migrations (version) VALUES (1)");

    // Insert token payments with old-format (untagged) u128 values

    // Large values payment (maxSupply = u128::MAX, fee > u64::MAX)
    await client.query(
      `INSERT INTO payments (id, payment_type, status, amount, fees, timestamp, method)
       VALUES ($1, $2, $3, $4, $5, $6, $7)`,
      ["bigint-token-payment", "send", "completed", "5000", "10", 1700000001, JSON.stringify("token")]
    );

    await client.query(
      `INSERT INTO payment_details_token (payment_id, metadata, tx_hash, tx_type)
       VALUES ($1, $2, $3, $4)`,
      [
        "bigint-token-payment",
        JSON.stringify({
          identifier: "test-token-id",
          issuerPublicKey: "02" + "a".repeat(64),
          name: "Test Token",
          ticker: "TST",
          decimals: 8,
          maxSupply: "340282366920938463463374607431768211455",
          isFreezable: false,
        }),
        "0xabcdef1234567890",
        "transfer",
      ]
    );

    await client.query(
      `INSERT INTO payment_metadata (payment_id, conversion_info)
       VALUES ($1, $2)`,
      [
        "bigint-token-payment",
        JSON.stringify({
          poolId: "pool-1",
          conversionId: "conv-1",
          status: "completed",
          fee: "18446744073709551616",
          purpose: null,
        }),
      ]
    );

    // Small values payment (maxSupply < u64::MAX, fee = 500)
    await client.query(
      `INSERT INTO payments (id, payment_type, status, amount, fees, timestamp, method)
       VALUES ($1, $2, $3, $4, $5, $6, $7)`,
      ["bigint-token-payment-small", "send", "completed", "1000", "5", 1700000002, JSON.stringify("token")]
    );

    await client.query(
      `INSERT INTO payment_details_token (payment_id, metadata, tx_hash, tx_type)
       VALUES ($1, $2, $3, $4)`,
      [
        "bigint-token-payment-small",
        JSON.stringify({
          identifier: "test-token-id-small",
          issuerPublicKey: "02" + "b".repeat(64),
          name: "Small Token",
          ticker: "SML",
          decimals: 6,
          maxSupply: "1000000",
          isFreezable: false,
        }),
        "0x1234567890abcdef",
        "transfer",
      ]
    );

    await client.query(
      `INSERT INTO payment_metadata (payment_id, conversion_info)
       VALUES ($1, $2)`,
      [
        "bigint-token-payment-small",
        JSON.stringify({
          poolId: "pool-2",
          conversionId: "conv-2",
          status: "completed",
          fee: "500",
          purpose: null,
        }),
      ]
    );

    await client.query("COMMIT");
  } catch (error) {
    await client.query("ROLLBACK").catch(() => {});
    throw new Error(`Failed to create old v1 database: ${error.message}`);
  } finally {
    client.release();
    await pool.end();
  }

  return connString;
}

module.exports = { createTestConnectionString, createOldV1Database };
