// Spent-marker scenario that demonstrates the timezone-mismatch bug fixed in
// the surrounding patch. Runs in a child process so the test runner can set
// `process.env.TZ` per invocation — Node's `Date` formatter latches the TZ at
// startup, so this test cannot mutate it inside a single process.
//
// Expects `MYSQL_URI` in the environment. Exits 0 on success, 1 with a
// printed error on failure.

const { createMysqlTokenStore } = require("../index.cjs");

// Same shape as the wasm-bindgen-test fixture identity; 33 bytes is required.
const TEST_IDENTITY = Buffer.from([
  0x02, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c,
  0x0d, 0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19,
  0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f, 0x20,
]);

// 33-byte secp256k1 compressed pubkey bytes formatted as hex strings, matching
// `create_token_outputs` in crates/spark/src/token/tests.rs. The values don't
// need to be cryptographically valid — the store only persists/echoes them.
function pubkeyHex(fillByte) {
  const buf = Buffer.alloc(33, fillByte);
  buf[0] = 2;
  return buf.toString("hex");
}

const OWNER_PUBKEY = Buffer.from([
  0x03, 0x17, 0xb7, 0xe1, 0xce, 0x1f, 0x9f, 0x94, 0xc3, 0x2a, 0x43, 0x73, 0x92,
  0x29, 0xf8, 0x8c, 0x0b, 0x03, 0x33, 0x29, 0x6f, 0xb4, 0x6e, 0x8f, 0x72, 0x86,
  0x58, 0x49, 0xc6, 0xae, 0x34, 0xb8, 0x4e,
]).toString("hex");

function buildTokenOutputs(amounts) {
  const identifier = "token-1";
  const ticker = "TK1";
  const issuerPk = pubkeyHex(1);

  return {
    metadata: {
      identifier,
      issuerPublicKey: issuerPk,
      name: `${ticker} Token`,
      ticker,
      decimals: 8,
      maxSupply: "1000000",
      isFreezable: false,
      creationEntityPublicKey: null,
    },
    outputs: amounts.map((amount, i) => ({
      output: {
        id: `output-${identifier}-${amount}`,
        ownerPublicKey: OWNER_PUBKEY,
        revocationCommitment: `commitment-${i}`,
        withdrawBondSats: 1000,
        withdrawRelativeBlockLocktime: 144,
        tokenPublicKey: issuerPk,
        tokenIdentifier: identifier,
        tokenAmount: String(amount),
      },
      prevTxHash: `tx-hash-${i}`,
      prevTxVout: i,
    })),
  };
}

async function main() {
  const connectionString = process.env.MYSQL_URI;
  if (!connectionString) {
    throw new Error("MYSQL_URI must be set");
  }

  const store = await createMysqlTokenStore(
    { connectionString, maxPoolSize: 4 },
    TEST_IDENTITY,
    null
  );

  try {
    const token = buildTokenOutputs([100, 200]);

    // 1. Initial set with a future refresh start, so the outputs aren't
    //    treated as stale.
    await store.setTokensOutputs([token], Date.now() + 10_000);

    // 2. Mark output 100 (at prev_tx_hash 'tx-hash-0', vout 0) as spent.
    await store.updateTokenOutputs([["tx-hash-0", 0]], null);

    // 3. Replay setTokensOutputs with a refresh start in the past (60s ago).
    //    The spent marker should suppress re-adding output 100.
    await store.setTokensOutputs([token], Date.now() - 60_000);

    const result = await store.getTokenOutputs({
      type: "identifier",
      identifier: "token-1",
    });

    if (result.available.length !== 1) {
      throw new Error(
        `expected 1 available output, got ${result.available.length} (TZ=${
          process.env.TZ
        }, amounts=${result.available
          .map((o) => o.output.tokenAmount)
          .join(",")})`
      );
    }
    if (result.available[0].output.tokenAmount !== "200") {
      throw new Error(
        `expected remaining output to be 200, got ${result.available[0].output.tokenAmount} (TZ=${process.env.TZ})`
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
