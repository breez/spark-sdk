# Shared behavioral scenarios

These JSON files define end-to-end wallet flows in terms of CLI commands and
expected output. They are the canonical behavioral tests for the SDK's CLI
surface, and they are **shared verbatim across languages**: each language CLI
port gets a thin runner that executes the same files, so a behavior is
defined once and enforced everywhere.

Current runners:

| Runner | Drives | Entry point | CI home |
|---|---|---|---|
| Rust | `crates/breez-sdk/cli` binary | `crates/breez-sdk/cli/tests/scenarios.rs` (`make cli-itest`) | step in `Breez integration tests` (shares its toolchain, faucet limits, and lnurl image) |
| JS/WASM | `bindings/examples/cli/langs/wasm` port (consumes the locally built `packages/wasm` npm package) | `packages/wasm/itest/scenarios.test.js` (`make wasm-itest`) | `WASM binding tests` job |
| Swift | `bindings/examples/cli/langs/swift` port (local uniffi bindings) | the Rust runner with `SCENARIO_CLI` pointing at the built binary (`make swift-itest`) | step in `CLI / swift` (macOS; lnurl scenarios skip: no docker) |
| Kotlin | `bindings/examples/cli/langs/kotlin-multiplatform` port's JVM target (shares the generated uniffi surface with Android) | the Rust runner with `SCENARIO_CLI="java -jar ..."` (`make kotlin-itest`) | step in `CLI / kotlin-multiplatform` |

The Rust runner is generic: `SCENARIO_CLI` (a command line) and `SCENARIO_CLI_CWD`
point it at any CLI port, so most languages need no runner code at all, just a
make target. The JS runner exists separately because it also hosts the npm-API
smoke suite.

When adding a language runner, prefer attaching its `make <lang>-itest` step
to the CI job that already has that language's toolchain and the faucet
secrets rather than adding a fresh job (the rust runner rides along with the
breez itests for exactly that reason).

## The sync contract

- **Scenarios are data, never ported.** Adding or changing behavior means
  changing the Rust CLI and the scenario in the same PR; the rust runner
  (`make cli-itest`, a step in the `Breez integration tests` CI job) proves
  the new behavior.
- **Runners are per-language and thin.** They only know how to spawn their
  CLI, feed stdin, and evaluate the assertions below. Adding a language means
  adding one runner, not porting tests.
- **A wasm runner failure on a Sync CLI PR means the port regressed** (or
  relies on behavior that was not ported): fix the port, not the scenario.
- Both suites soft-skip unless `FAUCET_USERNAME` is set, so the plain
  workspace test job stays hermetic. Scenarios that require docker also skip
  when the docker daemon is unreachable.

## How a runner executes a scenario

The runner spawns the CLI REPL with piped stdin/stdout (plus `--data-dir`
pointing at a temp dir per wallet) and, for each step, writes the command
line, then any scripted `stdin` answer lines, then a bogus marker command
like `__step_end_3__`. The CLI echoes the marker token in its
unknown-command error, which delimits the step's output without any CLI
support. Sessions with the same `wallet` share a data dir, so a later
session resumes the same wallet (the mnemonic persists in `<data-dir>/phrase`).

## Schema

```jsonc
{
  "name": "human-readable description",
  "requires": ["faucet", "docker"],   // optional; skip when unmet
  "fixtures": ["lnurl"],              // optional; provisioned by the runner
  "sessions": [
    {
      "wallet": "alice",              // data-dir key
      "extra_args": ["--lnurl-domain", "${lnurl_url}"],  // optional CLI args
      "steps": [ ... ]
    }
  ]
}
```

A step is exactly one of:

```jsonc
// A REPL command
{
  "cmd": "pay -r ${bob_address} -a 1000",
  "stdin": ["n"],                       // answers to interactive prompts, in order
  "expect_json": {                      // path -> matcher, against the LAST
    "payment.status": "completed",      //   JSON document the step printed
    "balance_sats": { "gte": 40000 },
    "payment.id": { "exists": true }
  },
  "expect_contains": ["${payment_id}"], // substring over the raw step output
  "capture": { "payment_id": "payment.id" },  // save values for later steps
  "retry": { "timeout_secs": 180, "interval_secs": 5 }  // re-run cmd until
}                                                       //   expects pass

// A harness action
{ "faucet_fund": { "address": "${alice_deposit_address}", "amount_sats": 50000 } }
```

`${name}` interpolates captured variables (and `${lnurl_url}` from the lnurl
fixture) inside `cmd`, `stdin`, `extra_args`, `expect_contains`, string
matchers, and `faucet_fund.address`.

### Tolerant matching

The Rust CLI prints `snake_case` JSON with externally tagged enums
(`{"Bolt11Invoice": {...}}`); JS ports print camelCase with
`{"type": "bolt11Invoice", ...}` tags and BigInt values as strings. So:

- Path segments match keys case-insensitively ignoring underscores, numeric
  segments index arrays, and an enum-variant segment matches either tag
  style. Write paths in `snake_case`.
- Equality compares stringified values case-insensitively (`"Completed"`
  equals `"completed"`, `1000` equals `"1000"`).
- `gte` accepts numbers and numeric strings. Use it (never exact equality)
  for balances: fees and claim timing are nondeterministic.

## Writing a new scenario

1. Add `NN_name.json` here and a matching `#[tokio::test]` entry in
   `tests/scenarios.rs` (the JS runner picks the file up automatically).
2. Script every interactive prompt the commands will ask, in order: a
   missing answer shows up as a step timeout with the transcript attached.
3. Verify with `make cli-itest` and `make wasm-itest` (needs
   `FAUCET_USERNAME`/`FAUCET_PASSWORD`; docker for lnurl scenarios).
