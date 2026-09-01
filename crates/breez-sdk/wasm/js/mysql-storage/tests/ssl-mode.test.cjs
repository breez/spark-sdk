// Regression test for the `ssl-mode` URL parameter translation in
// createMysqlPool. mysql2 does not understand `ssl-mode`, so a pool built
// straight from the URI would silently drop it and connect in plaintext;
// createMysqlPool must translate it to mysql2's `ssl` option and fail closed
// on unrecognized values. No database connection is made here: the assertions
// read the config mysql2 derived for the pool.

const { describe, test } = require("node:test");
const assert = require("node:assert/strict");
const path = require("node:path");
const fs = require("node:fs");
const os = require("node:os");

const { createMysqlPool, StorageError } = require("../index.cjs");

function sslConfigFor(connectionString) {
  const pool = createMysqlPool({ connectionString, maxPoolSize: 1 });
  const ssl = pool.pool.config.connectionConfig.ssl;
  pool.end();
  return ssl;
}

describe("createMysqlPool ssl-mode translation", () => {
  test("required verifies chain and hostname", () => {
    assert.deepEqual(
      sslConfigFor("mysql://u:p@h:3306/db?ssl-mode=required"),
      // verifyIdentity is required: mysql2 checks the hostname only when it
      // is set; rejectUnauthorized alone verifies just the chain.
      { rejectUnauthorized: true, verifyIdentity: true }
    );
  });

  test("verify_identity verifies chain and hostname", () => {
    assert.deepEqual(
      sslConfigFor("mysql://u:p@h:3306/db?ssl-mode=verify_identity"),
      { rejectUnauthorized: true, verifyIdentity: true }
    );
  });

  test("verify_ca without ssl-ca throws", () => {
    assert.throws(
      () =>
        createMysqlPool({
          connectionString: "mysql://u:p@h:3306/db?ssl-mode=verify_ca",
          maxPoolSize: 1,
        }),
      StorageError
    );
  });

  test("verify_ca pins the CA from ssl-ca", () => {
    const caPath = path.join(os.tmpdir(), `mysql-ssl-ca-${process.pid}.pem`);
    fs.writeFileSync(
      caPath,
      "-----BEGIN CERTIFICATE-----\ndummy\n-----END CERTIFICATE-----\n"
    );
    try {
      const ssl = sslConfigFor(
        `mysql://u:p@h:3306/db?ssl-mode=verify_ca&ssl-ca=${encodeURIComponent(caPath)}`
      );
      assert.equal(ssl.rejectUnauthorized, true);
      assert.equal(ssl.verifyIdentity, undefined);
      assert.match(ssl.ca, /BEGIN CERTIFICATE/);
    } finally {
      fs.unlinkSync(caPath);
    }
  });

  test("no-verify encrypts without verification", () => {
    assert.deepEqual(sslConfigFor("mysql://u:p@h:3306/db?ssl-mode=no-verify"), {
      rejectUnauthorized: false,
    });
  });

  test("disabled and absent mean no TLS", () => {
    assert.equal(sslConfigFor("mysql://u:p@h:3306/db?ssl-mode=disabled"), false);
    assert.equal(sslConfigFor("mysql://u:p@h:3306/db"), false);
  });

  test("other query parameters are preserved", () => {
    const pool = createMysqlPool({
      connectionString:
        "mysql://u:p@h:3306/db?charset=utf8mb4&ssl-mode=required",
      maxPoolSize: 1,
    });
    const cfg = pool.pool.config.connectionConfig;
    pool.end();
    assert.deepEqual(cfg.ssl, { rejectUnauthorized: true, verifyIdentity: true });
    assert.equal(cfg.charsetNumber > 0, true);
  });

  test("an unrecognized value throws instead of downgrading to plaintext", () => {
    assert.throws(
      () =>
        createMysqlPool({
          connectionString: "mysql://u:p@h:3306/db?ssl-mode=verify_identtiy",
          maxPoolSize: 1,
        }),
      StorageError
    );
  });
});
