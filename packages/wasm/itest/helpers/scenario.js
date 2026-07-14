'use strict'

// JS runner for the shared behavioral scenarios in
// crates/breez-sdk/cli/tests/scenarios/. It drives the wasm CLI port (which
// consumes the locally built npm package) the same way the Rust harness
// drives the Rust CLI: piped stdin, a bogus-command marker after every step,
// and casing/tag-tolerant JSON assertions. Mirror of
// crates/breez-sdk/cli/tests/harness/{session,assert,engine}.rs.

const fs = require('fs')
const os = require('os')
const path = require('path')
const { spawn } = require('child_process')

const { fundAddress } = require('./faucet')
const { LnurlFixture } = require('./lnurl-fixture')

const WASM_CLI_DIR = path.join(
  __dirname,
  '../../../../crates/breez-sdk/bindings/examples/cli/langs/wasm'
)
const SCENARIOS_DIR = path.join(__dirname, '../../../../crates/breez-sdk/cli/tests/scenarios')

// Marker-wait ceiling for a single command; longer waits use scenario-level
// retry. Same values as the Rust harness.
const STEP_TIMEOUT_MS = 120_000
const QUIESCE_MS = 200
const POLL_MS = 100

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

// --------------------------------------------------------------------------
// Tolerant JSON assertions (mirror of harness/assert.rs)
// --------------------------------------------------------------------------

/**
 * Extract the JSON documents a transcript chunk contains. A document starts
 * at a line whose first column is `{` or `[` and ends when the accumulated
 * lines parse.
 *
 * @param {string} chunk
 * @returns {any[]}
 */
function extractJsonDocs(chunk) {
  const docs = []
  let acc = null
  for (const line of chunk.split('\n')) {
    if (acc !== null) {
      acc += line
      try {
        docs.push(JSON.parse(acc))
        acc = null
      } catch {
        acc += '\n'
      }
    } else if (line.startsWith('{') || line.startsWith('[')) {
      try {
        docs.push(JSON.parse(line))
      } catch {
        acc = line + '\n'
      }
    }
  }
  return docs
}

/** Lowercase and strip underscores so all casings compare equal. */
function normalize(s) {
  return s.toLowerCase().replaceAll('_', '')
}

/**
 * Resolve a dot-separated path against a JSON value. Object keys match after
 * normalization; numeric segments index arrays. An enum-tag segment matches
 * either an externally tagged wrapper key (descends) or a `type` field on
 * the current object (stays in place).
 *
 * @param {any} root
 * @param {string} pathExpr
 * @returns {any} the value, or undefined when the path does not resolve
 */
function lookupPath(root, pathExpr) {
  let current = root
  for (const segment of pathExpr.split('.')) {
    const wanted = normalize(segment)
    if (Array.isArray(current)) {
      const index = Number.parseInt(segment, 10)
      if (Number.isNaN(index) || index >= current.length) {
        return undefined
      }
      current = current[index]
    } else if (current !== null && typeof current === 'object') {
      const key = Object.keys(current).find((k) => normalize(k) === wanted)
      if (key !== undefined) {
        current = current[key]
      } else if (typeof current.type === 'string' && normalize(current.type) === wanted) {
        // Tag-style enum: the segment names the variant of the object
        // itself; the fields live alongside the tag.
      } else {
        return undefined
      }
    } else {
      return undefined
    }
  }
  return current
}

/** Render a JSON value the way a scenario writes it. */
function valueToString(value) {
  return typeof value === 'string' ? value : JSON.stringify(value)
}

function asNumber(value) {
  if (typeof value === 'number') {
    return value
  }
  if (typeof value === 'string' && value.trim() !== '') {
    const n = Number(value)
    return Number.isNaN(n) ? undefined : n
  }
  return undefined
}

/**
 * Check one expect_json matcher against the value found at its path.
 * Matcher forms: bare value (tolerant equality), {gte: n}, {exists: bool}.
 *
 * @param {any} matcher
 * @param {any} found - undefined when the path did not resolve
 */
function checkMatcher(matcher, found) {
  if (matcher !== null && typeof matcher === 'object' && !Array.isArray(matcher)) {
    if (typeof matcher.exists === 'boolean') {
      const exists = found !== undefined && found !== null
      if (exists !== matcher.exists) {
        throw new Error(`expected exists=${matcher.exists}, value was ${JSON.stringify(found)}`)
      }
      return
    }
    if (matcher.gte !== undefined) {
      const floor = asNumber(matcher.gte)
      if (floor === undefined) {
        throw new Error(`gte bound is not numeric: ${matcher.gte}`)
      }
      const actual = asNumber(found)
      if (actual === undefined) {
        throw new Error(`expected a number >= ${floor}, value was ${JSON.stringify(found)}`)
      }
      if (actual < floor) {
        throw new Error(`expected >= ${floor}, got ${actual}`)
      }
      return
    }
  }

  if (found === undefined) {
    throw new Error('path not found in output')
  }
  const expected = valueToString(matcher).toLowerCase()
  const actual = valueToString(found).toLowerCase()
  if (expected !== actual) {
    throw new Error(`expected '${expected}', got '${actual}'`)
  }
}

// --------------------------------------------------------------------------
// Variable interpolation (mirror of harness/mod.rs)
// --------------------------------------------------------------------------

/** Replace every ${name} in input; unknown variables fail loudly. */
function interpolate(input, vars) {
  return input.replaceAll(/\$\{([^}]*)\}/g, (_, name) => {
    if (!(name in vars)) {
      throw new Error(`unknown variable '\${${name}}' in '${input}'`)
    }
    return vars[name]
  })
}

// --------------------------------------------------------------------------
// CLI session (mirror of harness/session.rs)
// --------------------------------------------------------------------------

class CliSession {
  /**
   * @param {string} dataDir
   * @param {string[]} extraArgs
   */
  constructor(dataDir, extraArgs) {
    this.child = spawn(
      process.execPath,
      ['src/main.js', '--data-dir', dataDir, ...extraArgs],
      { cwd: WASM_CLI_DIR, stdio: ['pipe', 'pipe', 'pipe'] }
    )
    this.transcript = ''
    this.cursor = 0
    this.stepCounter = 0
    for (const stream of [this.child.stdout, this.child.stderr]) {
      stream.setEncoding('utf8')
      stream.on('data', (data) => {
        this.transcript += data
      })
    }
  }

  /**
   * Run one command with its scripted stdin answers and return the
   * transcript chunk it produced.
   *
   * @param {string} cmd
   * @param {string[]} stdinLines
   * @returns {Promise<string>}
   */
  async runStep(cmd, stdinLines) {
    this.stepCounter += 1
    const marker = `__step_end_${this.stepCounter}__`
    const input = [cmd, ...stdinLines, marker].join('\n') + '\n'
    this.child.stdin.write(input)

    const deadline = Date.now() + STEP_TIMEOUT_MS
    let markerPos
    for (;;) {
      markerPos = this.transcript.indexOf(marker, this.cursor)
      if (markerPos !== -1) {
        break
      }
      if (Date.now() >= deadline) {
        throw new Error(
          `timed out after ${STEP_TIMEOUT_MS}ms waiting for step to finish; ` +
            `output so far:\n${this.transcript.slice(this.cursor)}`
        )
      }
      await sleep(POLL_MS)
    }

    // Wait for the marker error's trailing lines to land, then consume
    // everything up to the current end of transcript.
    let stableLength = this.transcript.length
    for (;;) {
      await sleep(QUIESCE_MS)
      if (this.transcript.length === stableLength) {
        break
      }
      stableLength = this.transcript.length
    }

    // The chunk ends at the start of the line carrying the marker error.
    const chunkEnd = this.transcript.lastIndexOf('\n', markerPos)
    const chunk = this.transcript.slice(this.cursor, Math.max(chunkEnd, this.cursor))
    this.cursor = stableLength
    return chunk
  }

  /** Ask the CLI to exit and wait for the process to finish. */
  async close() {
    this.child.stdin.write('exit\n')
    const exited = await Promise.race([
      new Promise((resolve) => this.child.once('exit', () => resolve(true))),
      sleep(30_000).then(() => false)
    ])
    if (!exited) {
      this.child.kill('SIGKILL')
      throw new Error("CLI did not exit within 30s of 'exit'")
    }
  }

  kill() {
    this.child.kill('SIGKILL')
  }
}

// --------------------------------------------------------------------------
// Scenario engine (mirror of harness/engine.rs)
// --------------------------------------------------------------------------

/** Check a step's expectations against its chunk; returns captured vars. */
function evaluate(chunk, step, vars) {
  const docs = extractJsonDocs(chunk)
  const last = docs.length > 0 ? docs[docs.length - 1] : undefined

  for (const [pathExpr, matcher] of Object.entries(step.expect_json ?? {})) {
    const resolved = typeof matcher === 'string' ? interpolate(matcher, vars) : matcher
    try {
      checkMatcher(resolved, last === undefined ? undefined : lookupPath(last, pathExpr))
    } catch (e) {
      throw new Error(`expect_json '${pathExpr}' failed: ${e.message}`)
    }
  }

  for (const needle of step.expect_contains ?? []) {
    const resolved = interpolate(needle, vars)
    if (!chunk.includes(resolved)) {
      throw new Error(`expect_contains '${resolved}' not found in step output`)
    }
  }

  const captured = {}
  for (const [name, pathExpr] of Object.entries(step.capture ?? {})) {
    const value = last === undefined ? undefined : lookupPath(last, pathExpr)
    if (value === undefined) {
      throw new Error(`capture '${name}': path '${pathExpr}' not found`)
    }
    captured[name] = valueToString(value)
  }
  return captured
}

async function runCmdStep(cli, step, vars) {
  const cmd = interpolate(step.cmd, vars)
  const stdinLines = (step.stdin ?? []).map((l) => interpolate(l, vars))
  const deadline = step.retry ? Date.now() + step.retry.timeout_secs * 1000 : undefined
  const intervalMs = (step.retry?.interval_secs ?? 5) * 1000

  for (;;) {
    const chunk = await cli.runStep(cmd, stdinLines)
    try {
      Object.assign(vars, evaluate(chunk, step, vars))
      return
    } catch (e) {
      if (deadline !== undefined && Date.now() < deadline) {
        console.error(`step '${cmd}' not satisfied yet (${e.message}), retrying`)
        await sleep(intervalMs)
      } else {
        throw new Error(`step '${cmd}' failed: ${e.message}\nstep output:\n${chunk}`)
      }
    }
  }
}

/**
 * Run one scenario JSON (already parsed). Fixture and requirement gating is
 * the caller's job; this assumes they are met.
 *
 * @param {object} scenario
 */
async function runScenario(scenario) {
  const vars = {}
  const fixtures = []
  const walletDirs = new Map()
  try {
    for (const fixture of scenario.fixtures ?? []) {
      if (fixture === 'lnurl') {
        const lnurl = await LnurlFixture.start()
        vars.lnurl_url = lnurl.httpUrl
        fixtures.push(lnurl)
      } else {
        throw new Error(`unknown fixture '${fixture}'`)
      }
    }

    for (const [sessionIndex, session] of scenario.sessions.entries()) {
      let dir = walletDirs.get(session.wallet)
      if (dir === undefined) {
        dir = fs.mkdtempSync(path.join(os.tmpdir(), 'breez-cli-scenario-'))
        walletDirs.set(session.wallet, dir)
      }
      const extraArgs = (session.extra_args ?? []).map((a) => interpolate(a, vars))
      const cli = new CliSession(dir, extraArgs)
      try {
        for (const [stepIndex, step] of session.steps.entries()) {
          try {
            if (step.faucet_fund) {
              const address = interpolate(step.faucet_fund.address, vars)
              const txid = await fundAddress(address, step.faucet_fund.amount_sats)
              console.error(
                `faucet funded ${address} with ${step.faucet_fund.amount_sats} sats: ${txid}`
              )
            } else {
              await runCmdStep(cli, step, vars)
            }
          } catch (e) {
            e.message =
              `scenario '${scenario.name}', session ${sessionIndex} ` +
              `(wallet '${session.wallet}'), step ${stepIndex}: ${e.message}`
            throw e
          }
        }
      } catch (e) {
        cli.kill()
        throw e
      }
      await cli.close()
    }
  } finally {
    for (const fixture of fixtures) {
      fixture.stop()
    }
    for (const dir of walletDirs.values()) {
      fs.rmSync(dir, { recursive: true, force: true })
    }
  }
}

/** List scenario files as [name, parsed] pairs, sorted by name. */
function loadScenarios() {
  return fs
    .readdirSync(SCENARIOS_DIR)
    .filter((f) => f.endsWith('.json'))
    .sort()
    .map((f) => [
      path.basename(f, '.json'),
      JSON.parse(fs.readFileSync(path.join(SCENARIOS_DIR, f), 'utf8'))
    ])
}

module.exports = {
  WASM_CLI_DIR,
  checkMatcher,
  extractJsonDocs,
  interpolate,
  loadScenarios,
  lookupPath,
  runScenario
}
