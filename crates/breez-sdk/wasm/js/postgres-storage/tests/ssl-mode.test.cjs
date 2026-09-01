// Regression test for the `sslmode` URI parameter translation in
// createPostgresPool. The pinned `pg` maps `require` to verified TLS but
// warns that pg v9 reverts it to libpq semantics (no verification), so
// createPostgresPool translates `sslmode` itself: the guarantees must not
// depend on the driver version. No database connection is made here: the
// assertions read the effective ssl config pg derives for a client.

const { describe, test } = require("node:test");
const assert = require("node:assert/strict");
const path = require("node:path");
const fs = require("node:fs");
const os = require("node:os");

const { createPostgresPool, StorageError } = require("../index.cjs");
const ConnectionParameters = require(
  path.resolve(__dirname, "../node_modules/pg/lib/connection-parameters.js")
);

// The ssl config a pg.Client would actually use for this pool's options:
// pg gives a connection string's ssl settings precedence over the explicit
// option, so this also proves the shim left no competing params behind.
function effectiveSsl(connectionString) {
  const pool = createPostgresPool({ connectionString, maxPoolSize: 1 });
  const params = new ConnectionParameters(pool.options);
  pool.end();
  return params.ssl;
}

describe("createPostgresPool sslmode translation", () => {
  test("require verifies the server certificate", () => {
    assert.deepEqual(effectiveSsl("postgres://u:p@h/db?sslmode=require"), {
      rejectUnauthorized: true,
    });
  });

  test("prefer and verify-full verify like require", () => {
    assert.deepEqual(effectiveSsl("postgres://u:p@h/db?sslmode=prefer"), {
      rejectUnauthorized: true,
    });
    assert.deepEqual(effectiveSsl("postgres://u:p@h/db?sslmode=verify-full"), {
      rejectUnauthorized: true,
    });
  });

  test("verify-ca without sslrootcert throws", () => {
    assert.throws(
      () =>
        createPostgresPool({
          connectionString: "postgres://u:p@h/db?sslmode=verify-ca",
          maxPoolSize: 1,
        }),
      StorageError
    );
  });

  test("no-verify encrypts without verification", () => {
    assert.deepEqual(effectiveSsl("postgres://u:p@h/db?sslmode=no-verify"), {
      rejectUnauthorized: false,
    });
  });

  test("disable and absent mean no TLS", () => {
    assert.equal(effectiveSsl("postgres://u:p@h/db?sslmode=disable"), false);
    assert.equal(effectiveSsl("postgres://u:p@h/db"), false);
  });

  test("other query parameters are preserved", () => {
    const pool = createPostgresPool({
      connectionString:
        "postgres://u:p@h/db?application_name=app&sslmode=require",
      maxPoolSize: 1,
    });
    const params = new ConnectionParameters(pool.options);
    pool.end();
    assert.deepEqual(params.ssl, { rejectUnauthorized: true });
    assert.equal(params.application_name, "app");
  });

  test("sslrootcert is folded into the ssl option", () => {
    const caPath = path.join(os.tmpdir(), `ssl-mode-test-ca-${process.pid}.pem`);
    fs.writeFileSync(caPath, "-----BEGIN CERTIFICATE-----\ndummy\n-----END CERTIFICATE-----\n");
    try {
      const ssl = effectiveSsl(
        `postgres://u:p@h/db?sslmode=verify-ca&sslrootcert=${encodeURIComponent(caPath)}`
      );
      assert.equal(ssl.rejectUnauthorized, true);
      assert.match(ssl.ca, /BEGIN CERTIFICATE/);
      assert.equal(typeof ssl.checkServerIdentity, "function");
      assert.equal(ssl.checkServerIdentity("host", {}), undefined);
    } finally {
      fs.unlinkSync(caPath);
    }
  });

  test("an unrecognized value throws instead of changing the TLS level", () => {
    assert.throws(
      () =>
        createPostgresPool({
          connectionString: "postgres://u:p@h/db?sslmode=verify-fll",
          maxPoolSize: 1,
        }),
      StorageError
    );
  });
});
