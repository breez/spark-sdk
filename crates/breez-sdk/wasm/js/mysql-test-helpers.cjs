/**
 * Test helpers for MySQL storage tests.
 * This file is ONLY used by wasm tests, not production code.
 *
 * Requires Docker to be running — spins up a MySQL container via testcontainers.
 */

const path = require("path");

// Resolve dependencies from the mysql-storage package where they're installed
let GenericContainer, Wait, mysql;
try {
  const mysqlStoragePath = path.join(__dirname, "mysql-storage", "node_modules");
  const tc = require(path.join(mysqlStoragePath, "testcontainers"));
  GenericContainer = tc.GenericContainer;
  Wait = tc.Wait;
  mysql = require(path.join(mysqlStoragePath, "mysql2", "promise"));
} catch (error) {
  try {
    const mainModule = require.main;
    if (mainModule) {
      const tc = mainModule.require("testcontainers");
      GenericContainer = tc.GenericContainer;
      Wait = tc.Wait;
      mysql = mainModule.require("mysql2/promise");
    } else {
      const tc = require("testcontainers");
      GenericContainer = tc.GenericContainer;
      Wait = tc.Wait;
      mysql = require("mysql2/promise");
    }
  } catch (fallbackError) {
    throw new Error(
      `testcontainers or mysql2 not found. Please install them in mysql-storage: cd js/mysql-storage && npm install\n` +
        `Original error: ${error.message}\nFallback error: ${fallbackError.message}`
    );
  }
}

const MYSQL_IMAGE = "mysql:8";
const MYSQL_USER = "root";
const MYSQL_PASSWORD = "test";
const MYSQL_PORT = 3306;

let _container = null;
let _containerHost = null;
let _containerPort = null;

/**
 * Ensure a shared MySQL container is running.
 * Returns { host, port } for connection.
 */
async function ensureContainer() {
  if (_container) {
    return { host: _containerHost, port: _containerPort };
  }

  _container = await new GenericContainer(MYSQL_IMAGE)
    .withEnvironment({
      MYSQL_ROOT_PASSWORD: MYSQL_PASSWORD,
      MYSQL_DATABASE: "template_test",
    })
    .withExposedPorts(MYSQL_PORT)
    .withWaitStrategy(
      Wait.forLogMessage(/ready for connections.*port: 3306/, 1)
    )
    .start();

  _containerHost = _container.getHost();
  _containerPort = _container.getMappedPort(MYSQL_PORT);

  return { host: _containerHost, port: _containerPort };
}

let _dbCounter = 0;

/**
 * Create a fresh database for a single test, returning a connection string.
 * Each database name is unique to avoid cross-test interference.
 */
async function createTestConnectionString(testName) {
  const { host, port } = await ensureContainer();
  _dbCounter += 1;
  const dbName = `test_${testName.replace(/[^a-zA-Z0-9_]/g, "_")}_${_dbCounter}`;

  // Connect to the default mysql admin database to issue CREATE DATABASE.
  const conn = await mysql.createConnection({
    host,
    port,
    user: MYSQL_USER,
    password: MYSQL_PASSWORD,
    database: "mysql",
  });
  try {
    await conn.query(`CREATE DATABASE IF NOT EXISTS \`${dbName}\``);
  } finally {
    await conn.end();
  }

  return `mysql://${MYSQL_USER}:${MYSQL_PASSWORD}@${host}:${port}/${dbName}`;
}

module.exports = { ensureContainer, createTestConnectionString };
