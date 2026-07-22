'use strict'

// Runs the shared behavioral scenarios from
// crates/breez-sdk/cli/tests/scenarios/ against the wasm CLI port. Mirrors
// the gating of the Rust runner: the whole suite skips unless
// FAUCET_USERNAME is set, and docker-requiring scenarios skip when the
// docker daemon is unreachable.

require('dotenv').config()

const fs = require('fs')
const path = require('path')
const test = require('node:test')

const { WASM_CLI_DIR, loadScenarios, runScenario } = require('./helpers/scenario')
const { dockerAvailable } = require('./helpers/lnurl-fixture')

const portReady = fs.existsSync(
  path.join(WASM_CLI_DIR, 'node_modules/@breeztech/breez-sdk-spark')
)
const faucetConfigured = Boolean(process.env.FAUCET_USERNAME)
const haveDocker = dockerAvailable()

for (const [name, scenario] of loadScenarios()) {
  test(`scenario ${name}`, async (t) => {
    // A typo'd name must fail, not silently un-gate the scenario. 'faucet'
    // itself is enforced by the suite-wide gate below; 'docker' is probed.
    const unknown = (scenario.requires ?? []).find((r) => r !== 'faucet' && r !== 'docker')
    if (unknown !== undefined) {
      throw new Error(`unknown requirement '${unknown}'`)
    }
    if (!faucetConfigured) {
      t.skip('FAUCET_USERNAME not set')
      return
    }
    if ((scenario.requires ?? []).includes('docker') && !haveDocker) {
      t.skip('docker not available')
      return
    }
    if (!portReady) {
      throw new Error(
        `wasm CLI port has no installed bindings at ${WASM_CLI_DIR}; run 'make wasm-itest'`
      )
    }
    await runScenario(scenario)
  })
}
