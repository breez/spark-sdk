// TZ-mismatch regression test for mysql-tree-store. Runs the spent-leaf
// scenario under several host timezones to catch any JS-side timezone bug
// in the tree-store, mirroring the protection that mysql-token-store has
// via its own timezone.test.cjs.
//
// A regression here would otherwise pass on UTC-only CI runners; this test
// forces a positive- and a negative-offset TZ regardless of the host's clock.

const { describe, test } = require("node:test");
const assert = require("node:assert/strict");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const { createTestConnectionString } = require(
  "../../mysql-test-helpers.cjs"
);

const SCENARIO_PATH = path.join(__dirname, "timezone-scenario.cjs");
const TIMEZONES = [
  "UTC",
  "Pacific/Kiritimati", // UTC+14: largest positive offset.
  "Pacific/Pago_Pago", // UTC-11: large negative offset.
  "Europe/Berlin", // UTC+1/+2: the original repro environment.
];

describe("mysql-tree-store timezone regression", () => {
  for (const tz of TIMEZONES) {
    test(
      `spent marker survives setLeaves replay under TZ=${tz}`,
      { timeout: 120_000 },
      async () => {
        const connectionString = await createTestConnectionString(
          `tz_tree_${tz.replace(/[^a-zA-Z0-9]/g, "_")}`
        );

        const result = spawnSync(
          process.execPath,
          [SCENARIO_PATH],
          {
            env: {
              ...process.env,
              TZ: tz,
              MYSQL_URI: connectionString,
            },
            stdio: "inherit",
          }
        );

        assert.equal(
          result.status,
          0,
          `scenario failed under TZ=${tz} (exit ${result.status})`
        );
      }
    );
  }
});
