// Spent-leaf scenario that demonstrates the timezone-mismatch bug fixed in
// commit 141bcf1a ("Pin all MySQL timestamps to UTC to fix host-TZ
// mismatch"). Runs in a child process so the test runner can set
// `process.env.TZ` per invocation — Node's `Date` formatter latches the TZ at
// startup, so this test cannot mutate it inside a single process.
//
// Mirrors mysql-token-store/tests/timezone-scenario.cjs but exercises the
// tree-store's setLeaves/finalizeReservation spent-marker path.
//
// Expects `MYSQL_URI` in the environment. Exits 0 on success, 1 with a
// printed error on failure.

const { createMysqlTreeStore } = require("../index.cjs");

// Same shape as the wasm-bindgen-test fixture identity; 33 bytes is required.
const TEST_IDENTITY = Buffer.from([
  0x02, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c,
  0x0d, 0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19,
  0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f, 0x20,
]);

// The tree-store persists each leaf as JSON in the `data` column, so the
// scenario only needs the fields touched by selection / spent-marker logic:
// `id`, `status`, `value`. Everything else round-trips via JSON.stringify.
function buildLeaf(id, value) {
  return { id, status: "Available", value };
}

async function main() {
  const connectionString = process.env.MYSQL_URI;
  if (!connectionString) {
    throw new Error("MYSQL_URI must be set");
  }

  const store = await createMysqlTreeStore(
    { connectionString, maxPoolSize: 4 },
    TEST_IDENTITY,
    null
  );

  try {
    const leafA = buildLeaf("leaf-100", 100);
    const leafB = buildLeaf("leaf-200", 200);

    // 1. Populate the tree under a future refresh window so neither leaf
    //    is treated as aged out.
    await store.setLeaves([leafA, leafB], [], Date.now() + 10_000);

    // 2. Reserve leaf-100 by exact amount.
    const reserve = await store.tryReserveLeaves(
      { type: "amountAndFee", amountSats: 100, feeSats: null },
      true,
      "Payment"
    );
    if (reserve.type !== "success") {
      throw new Error(
        `expected tryReserveLeaves success, got type=${reserve.type}`
      );
    }
    const reservedIds = reserve.reservation.leaves.map((l) => l.id);
    if (reservedIds.length !== 1 || reservedIds[0] !== "leaf-100") {
      throw new Error(
        `expected reservation to hold leaf-100, got [${reservedIds.join(",")}]`
      );
    }

    // 3. Finalize → spent marker for leaf-100 (no replacement leaves).
    await store.finalizeReservation(reserve.reservation.id, null);

    // 4. Replay setLeaves with a refresh start in the past. The spent
    //    marker (timestamp ≈ now) is newer than the refresh start, so
    //    leaf-100 must not be re-added. This is the comparison that was
    //    silently inverted on positive-offset hosts before the fix.
    await store.setLeaves([leafA, leafB], [], Date.now() - 60_000);

    const result = await store.getLeaves();
    if (result.available.length !== 1) {
      throw new Error(
        `expected 1 available leaf, got ${result.available.length} (TZ=${
          process.env.TZ
        }, ids=${result.available.map((l) => l.id).join(",")})`
      );
    }
    if (result.available[0].id !== "leaf-200") {
      throw new Error(
        `expected surviving leaf to be leaf-200, got ${result.available[0].id} (TZ=${process.env.TZ})`
      );
    }

    console.log(`PASS (TZ=${process.env.TZ ?? "unset"})`);
  } finally {
    await store.close();
  }
}

main().catch((err) => {
  console.error("FAIL:", err.message ?? err);
  process.exit(1);
});
