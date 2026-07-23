'use strict'

// LNURL server fixture driven through the docker CLI, mirroring the Rust
// harness (crates/breez-sdk/cli/tests/harness/lnurl.rs). Shares the
// breez-lnurl-built image tag: whichever runner builds it first wins. To
// force a rebuild: `docker image rm breez-lnurl-built:latest`.

const path = require('path')
const { execFile, execFileSync } = require('child_process')
const { promisify } = require('util')

const execFileAsync = promisify(execFile)

const IMAGE = 'breez-lnurl-built:latest'
const HTTP_PORT = 8080
const START_TIMEOUT_MS = 120_000
const WORKSPACE_ROOT = path.join(__dirname, '../../../..')

/** @returns {boolean} whether the docker daemon is reachable */
function dockerAvailable() {
  try {
    execFileSync('docker', ['version'], { stdio: 'ignore' })
    return true
  } catch {
    return false
  }
}

async function docker(args, opts = {}) {
  const { stdout } = await execFileAsync('docker', args, { maxBuffer: 64 * 1024 * 1024, ...opts })
  return stdout
}

async function buildImageIfMissing() {
  try {
    await docker(['image', 'inspect', IMAGE])
    return
  } catch {
    // Image missing: build it.
  }
  console.error(`building ${IMAGE} (first run only, this can take several minutes)`)
  await docker([
    'build',
    '-f',
    path.join(WORKSPACE_ROOT, 'crates/breez-sdk/lnurl/Dockerfile'),
    '-t',
    IMAGE,
    '--build-arg',
    'CARGO_FEATURES=dev',
    WORKSPACE_ROOT
  ])
}

class LnurlFixture {
  /** @param {string} containerId @param {string} httpUrl */
  constructor(containerId, httpUrl) {
    this.containerId = containerId
    this.httpUrl = httpUrl
  }

  static async start() {
    await buildImageIfMissing()
    const runOutput = await docker([
      'run',
      '-d',
      '--rm',
      '-p',
      '127.0.0.1:0:8080',
      '--add-host',
      'host.docker.internal:host-gateway',
      '-e', 'BREEZ_LNURL_NETWORK=regtest',
      '-e', 'BREEZ_LNURL_AUTO_MIGRATE=true',
      '-e', 'BREEZ_LNURL_DB_URL=:memory:',
      '-e', 'BREEZ_LNURL_LOG_LEVEL=lnurl=trace,info',
      '-e', 'BREEZ_LNURL_DOMAINS=',
      '-e', 'BREEZ_LNURL_SCHEME=http',
      '-e', 'BREEZ_LNURL_NSEC=nsec1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqsmhltgl',
      '-e', 'BREEZ_LNURL_MIN_SENDABLE=1000',
      '-e', 'BREEZ_LNURL_MAX_SENDABLE=1000000000',
      '-e', 'BREEZ_LNURL_DEV_DONT_USE_LNURL_INCLUDE_SPARK_ADDRESS=false',
      IMAGE
    ])
    const containerId = runOutput.trim()
    const fixture = new LnurlFixture(containerId, '')
    try {
      await fixture.waitForLog('starting lnurl server')
      const portOutput = await docker(['port', containerId, `${HTTP_PORT}/tcp`])
      const port = portOutput.split('\n')[0].trim().split(':').pop()
      fixture.httpUrl = `http://127.0.0.1:${port}`
      return fixture
    } catch (e) {
      fixture.stop()
      throw e
    }
  }

  async waitForLog(needle) {
    const deadline = Date.now() + START_TIMEOUT_MS
    for (;;) {
      // docker logs writes to both stdout and stderr; check both.
      let logs
      try {
        const { stdout, stderr } = await execFileAsync('docker', ['logs', this.containerId])
        logs = stdout + stderr
      } catch (e) {
        throw new Error(`failed to read lnurl container logs: ${e.message}`)
      }
      if (logs.includes(needle)) {
        return
      }
      if (Date.now() >= deadline) {
        throw new Error(`lnurl server did not log '${needle}' within ${START_TIMEOUT_MS}ms:\n${logs}`)
      }
      await new Promise((resolve) => setTimeout(resolve, 1000))
    }
  }

  stop() {
    try {
      execFileSync('docker', ['rm', '-f', this.containerId], { stdio: 'ignore' })
    } catch {
      // Container may already be gone.
    }
  }
}

module.exports = { LnurlFixture, dockerAvailable }
