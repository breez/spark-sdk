'use strict'

// LNURL server fixture driven through the docker CLI, mirroring the Rust
// harness (crates/breez-sdk/cli/tests/harness/lnurl.rs). Shares the
// breez-lnurl-built image tag: whichever runner builds it first wins. To
// force a rebuild: `docker image rm breez-lnurl-built:latest`.

const net = require('net')
const path = require('path')
const { execFile, execFileSync } = require('child_process')
const { promisify } = require('util')

const execFileAsync = promisify(execFile)

const IMAGE = 'breez-lnurl-built:latest'
const POSTGRES_IMAGE = 'postgres:16-alpine'
const HTTP_PORT = 8080
const POSTGRES_PORT = 5432
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

/** @returns {Promise<number>} host port the container port was published to */
async function publishedPort(containerId, containerPort) {
  const portOutput = await docker(['port', containerId, `${containerPort}/tcp`])
  return Number(portOutput.split('\n')[0].trim().split(':').pop())
}

/**
 * Resolves once something answers HTTP on `port`. Any status answers the
 * question, so this only checks that a response came back. HTTP/1.0 so the
 * server closes the connection when it is done.
 */
function httpResponds(port) {
  return new Promise((resolve, reject) => {
    const socket = net.connect(port, '127.0.0.1')
    let response = ''
    socket.setTimeout(5000)
    socket.on('connect', () => socket.write('GET /health HTTP/1.0\r\nHost: localhost\r\n\r\n'))
    socket.on('data', (chunk) => {
      response += chunk
    })
    socket.on('end', () =>
      response.startsWith('HTTP/') ? resolve() : reject(new Error('no HTTP response'))
    )
    socket.on('timeout', () => socket.destroy(new Error('timed out')))
    socket.on('error', reject)
  })
}

class LnurlFixture {
  /** @param {string} containerId @param {string} httpUrl @param {string} postgresContainerId */
  constructor(containerId, httpUrl, postgresContainerId) {
    this.containerId = containerId
    this.httpUrl = httpUrl
    // Backing database. Postgres is the server's only supported backend.
    this.postgresContainerId = postgresContainerId
  }

  static async start() {
    await buildImageIfMissing()
    const fixture = new LnurlFixture('', '', '')
    try {
      const postgresOutput = await docker([
        'run',
        '-d',
        // All interfaces rather than 127.0.0.1: the lnurl server reaches this
        // from inside its own container through the host gateway, which on
        // Linux is the bridge address and not loopback.
        '-p',
        '0.0.0.0:0:5432',
        '-e', 'POSTGRES_PASSWORD=postgres',
        POSTGRES_IMAGE
      ])
      fixture.postgresContainerId = postgresOutput.trim()
      await fixture.waitForPostgres()
      const dbPort = await publishedPort(fixture.postgresContainerId, POSTGRES_PORT)

      const runOutput = await docker([
        'run',
        '-d',
        '-p',
        '127.0.0.1:0:8080',
        '--add-host',
        'host.docker.internal:host-gateway',
        '-e', 'BREEZ_LNURL_NETWORK=regtest',
        '-e', 'BREEZ_LNURL_AUTO_MIGRATE=true',
        '-e', `BREEZ_LNURL_DB_URL=postgres://postgres:postgres@host.docker.internal:${dbPort}/postgres`,
        '-e', 'BREEZ_LNURL_LOG_LEVEL=lnurl=trace,info',
        '-e', 'BREEZ_LNURL_DOMAINS=',
        '-e', 'BREEZ_LNURL_SCHEME=http',
        '-e', 'BREEZ_LNURL_NSEC=nsec1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqsmhltgl',
        '-e', 'BREEZ_LNURL_MIN_SENDABLE=1000',
        '-e', 'BREEZ_LNURL_MAX_SENDABLE=1000000000',
        '-e', 'BREEZ_LNURL_DEV_DONT_USE_LNURL_INCLUDE_SPARK_ADDRESS=false',
        IMAGE
      ])
      fixture.containerId = runOutput.trim()

      const port = await publishedPort(fixture.containerId, HTTP_PORT)
      await fixture.waitForHttp(port)
      fixture.httpUrl = `http://127.0.0.1:${port}`
      return fixture
    } catch (e) {
      fixture.stop()
      throw e
    }
  }

  /**
   * Readiness cannot be taken from the logs: the server's startup line is
   * printed before it migrates the database and binds, and it logs nothing
   * afterwards. Nor is reaching the port enough, since docker's proxy accepts
   * connections there from the moment the container starts and closes them
   * again while nothing is listening inside.
   */
  async waitForHttp(port) {
    const deadline = Date.now() + START_TIMEOUT_MS
    for (;;) {
      try {
        await httpResponds(port)
        return
      } catch {
        // Not serving yet.
      }
      await this.ensureRunning(this.containerId, 'lnurl server')
      if (Date.now() >= deadline) {
        const logs = await this.logs(this.containerId)
        throw new Error(`lnurl server did not serve HTTP within ${START_TIMEOUT_MS}ms:\n${logs}`)
      }
      await new Promise((resolve) => setTimeout(resolve, 250))
    }
  }

  async waitForPostgres() {
    const deadline = Date.now() + START_TIMEOUT_MS
    for (;;) {
      try {
        // Probe TCP explicitly: while initialising, the image's entrypoint
        // runs a temporary socket-only server that the default `pg_isready`
        // check would accept as ready.
        await docker([
          'exec', this.postgresContainerId,
          'pg_isready', '-h', '127.0.0.1', '-U', 'postgres'
        ])
        return
      } catch {
        // Not accepting connections yet.
      }
      await this.ensureRunning(this.postgresContainerId, 'postgres')
      if (Date.now() >= deadline) {
        throw new Error(`postgres did not accept connections within ${START_TIMEOUT_MS}ms`)
      }
      await new Promise((resolve) => setTimeout(resolve, 250))
    }
  }

  /**
   * Fail with the container's logs once it has exited. A container that dies
   * on startup otherwise surfaces only as a readiness timeout, or as an
   * unrelated docker error, neither of which names the real cause.
   */
  async ensureRunning(containerId, name) {
    const state = await docker(['inspect', '-f', '{{.State.Running}}', containerId])
    if (state.trim() === 'true') {
      return
    }
    throw new Error(`${name} container exited before it was ready:\n${await this.logs(containerId)}`)
  }

  async logs(containerId) {
    try {
      // docker logs writes to both stdout and stderr; check both.
      const { stdout, stderr } = await execFileAsync('docker', ['logs', containerId])
      return stdout + stderr
    } catch (e) {
      return `failed to read container logs: ${e.message}`
    }
  }

  stop() {
    for (const containerId of [this.containerId, this.postgresContainerId]) {
      if (!containerId) {
        continue
      }
      try {
        execFileSync('docker', ['rm', '-f', containerId], { stdio: 'ignore' })
      } catch {
        // Container may already be gone.
      }
    }
  }
}

module.exports = { LnurlFixture, dockerAvailable }
