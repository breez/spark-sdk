'use strict'

require('dotenv').config()
require('./serialization') // Sets up BigInt.prototype.toJSON

const fs = require('fs')
const path = require('path')
const readline = require('readline')
const { parse: parseShell } = require('shell-quote')

const {
  SdkBuilder,
  defaultConfig,
  defaultPostgresStorageConfig,
  initLogging,
  getSparkStatus
} = require('@breeztech/breez-sdk-spark/nodejs')

const { CliPersistence } = require('./persistence')
const { buildProgram, COMMAND_NAMES } = require('./commands')
const { parsePasskeyProvider, buildPrfProvider, resolvePasskeySeed } = require('./passkey')

// ---------------------------------------------------------------------------
// CLI argument parsing (manual, since commander is used inside the REPL)
// ---------------------------------------------------------------------------

function parseCliArgs() {
  const args = process.argv.slice(2)
  const opts = {
    dataDir: './.data',
    network: 'regtest',
    accountNumber: undefined,
    postgresConnectionString: undefined,
    stableBalanceTokenIdentifier: undefined,
    stableBalanceThreshold: undefined,
    passkey: undefined,
    label: undefined,
    listLabels: false,
    storeLabel: false,
    rpid: undefined
  }

  for (let i = 0; i < args.length; i++) {
    switch (args[i]) {
      case '-d':
      case '--data-dir':
        opts.dataDir = args[++i]
        break
      case '--network':
        opts.network = args[++i]
        break
      case '--account-number':
        opts.accountNumber = parseInt(args[++i], 10)
        break
      case '--postgres-connection-string':
        opts.postgresConnectionString = args[++i]
        break
      case '--stable-balance-token-identifier':
        opts.stableBalanceTokenIdentifier = args[++i]
        break
      case '--stable-balance-threshold':
        opts.stableBalanceThreshold = parseInt(args[++i], 10)
        break
      case '--passkey':
        opts.passkey = args[++i]
        break
      case '--label':
        opts.label = args[++i]
        break
      case '--list-labels':
        opts.listLabels = true
        break
      case '--store-label':
        opts.storeLabel = true
        break
      case '--rpid':
        opts.rpid = args[++i]
        break
      case '-h':
      case '--help':
        console.log('Usage: node src/main.js [OPTIONS]')
        console.log('')
        console.log('Options:')
        console.log('  -d, --data-dir <path>                       Path to the data directory (default: ./.data)')
        console.log('  --network <network>                          Network to use: regtest or mainnet (default: regtest)')
        console.log('  --account-number <number>                    Account number for the Spark signer')
        console.log('  --postgres-connection-string <string>        PostgreSQL connection string')
        console.log('  --stable-balance-token-identifier <string>   Stable balance token identifier')
        console.log('  --stable-balance-threshold <number>          Stable balance threshold in sats')
        console.log('  --passkey <provider>                         Use passkey with PRF provider (file, yubikey, or fido2)')
        console.log('  --label <name>                               Label for seed derivation (requires --passkey)')
        console.log('  --list-labels                                List and select from labels on Nostr (requires --passkey)')
        console.log('  --store-label                                Publish the label to Nostr (requires --passkey and --label)')
        console.log('  --rpid <id>                                  Relying party ID for FIDO2 provider (requires --passkey)')
        console.log('  -h, --help                                   Show this help message')
        process.exit(0)
        break
      default:
        console.error(`Unknown option: ${args[i]}`)
        process.exit(1)
    }
  }

  // Validate passkey-related flag constraints
  if (!opts.passkey) {
    if (opts.label) {
      console.error('--label requires --passkey')
      process.exit(1)
    }
    if (opts.listLabels) {
      console.error('--list-labels requires --passkey')
      process.exit(1)
    }
    if (opts.storeLabel) {
      console.error('--store-label requires --passkey')
      process.exit(1)
    }
    if (opts.rpid) {
      console.error('--rpid requires --passkey')
      process.exit(1)
    }
  }

  if (opts.storeLabel && !opts.label) {
    console.error('--store-label requires --label')
    process.exit(1)
  }

  if (opts.listLabels && (opts.label || opts.storeLabel)) {
    console.error('--list-labels conflicts with --label and --store-label')
    process.exit(1)
  }

  return opts
}

/**
 * Expand ~ to home directory in a path.
 */
function expandPath(p) {
  if (p.startsWith('~/')) {
    return path.join(require('os').homedir(), p.slice(2))
  }
  return p
}

// ---------------------------------------------------------------------------
// Event listener
// ---------------------------------------------------------------------------

class CliEventListener {
  onEvent = (event) => {
    try {
      console.log(`\nEvent: ${JSON.stringify(event)}`)
    } catch {
      console.log('\nEvent: [failed to serialize]')
    }
  }
}

// ---------------------------------------------------------------------------
// Logger
// ---------------------------------------------------------------------------

class CliFileLogger {
  constructor(logStream) {
    this.logStream = logStream
  }

  log = (logEntry) => {
    const msg = `[${new Date().toISOString()} ${logEntry.level}]: ${logEntry.line}\n`
    this.logStream.write(msg)
  }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

async function main() {
  const opts = parseCliArgs()

  // Resolve and create data directory
  const dataDir = expandPath(opts.dataDir)
  fs.mkdirSync(dataDir, { recursive: true })

  // Parse network
  const networkLower = opts.network.toLowerCase()
  if (networkLower !== 'regtest' && networkLower !== 'mainnet') {
    console.error("Invalid network. Use 'regtest' or 'mainnet'")
    process.exit(1)
  }
  const network = networkLower

  // Init logging
  const logStream = fs.createWriteStream(path.join(dataDir, 'sdk.log'), { flags: 'a' })
  const fileLogger = new CliFileLogger(logStream)
  try {
    await initLogging(fileLogger)
  } catch {
    // Logging may already be initialized
  }

  // Persistence
  const persistence = new CliPersistence(dataDir)

  // Config
  const config = defaultConfig(network)
  const breezApiKey = process.env.BREEZ_API_KEY
  if (breezApiKey) {
    config.apiKey = breezApiKey
  }

  // Stable balance config
  if (opts.stableBalanceTokenIdentifier) {
    config.stableBalanceConfig = {
      tokenIdentifier: opts.stableBalanceTokenIdentifier,
      thresholdSats: opts.stableBalanceThreshold,
      maxSlippageBps: undefined,
      reservedSats: undefined
    }
  }

  // Resolve seed: passkey or mnemonic
  let seed
  if (opts.passkey) {
    const provider = parsePasskeyProvider(opts.passkey)
    const prfProvider = buildPrfProvider(provider, dataDir, opts.rpid)
    seed = await resolvePasskeySeed(
      prfProvider,
      breezApiKey,
      opts.label,
      opts.listLabels,
      opts.storeLabel
    )
  } else {
    const mnemonic = persistence.getOrCreateMnemonic()
    seed = { type: 'mnemonic', mnemonic, passphrase: undefined }
  }

  // Build SDK using SdkBuilder
  let sdkBuilder = SdkBuilder.new(config, seed)

  if (opts.postgresConnectionString) {
    sdkBuilder = sdkBuilder.withPostgresStorage(
      defaultPostgresStorageConfig(opts.postgresConnectionString)
    )
  } else {
    sdkBuilder = await sdkBuilder.withDefaultStorage(dataDir)
  }

  if (opts.accountNumber != null) {
    sdkBuilder = sdkBuilder.withKeySet({
      keySetType: 'default',
      useAddressIndex: false,
      accountNumber: opts.accountNumber
    })
  }

  const sdk = await sdkBuilder.build()

  // Event listener
  const eventListener = new CliEventListener()
  await sdk.addEventListener(eventListener)

  // Token issuer
  const tokenIssuer = sdk.getTokenIssuer()

  // Build tab-completion list from all command names
  const allCommands = [
    ...COMMAND_NAMES,
    'exit',
    'quit',
    'help'
  ]

  // Tab-completion function
  function completer(line) {
    const hits = allCommands.filter((cmd) => cmd.startsWith(line.trim()))
    return [hits.length ? hits : allCommands, line]
  }

  // Create readline interface for the REPL with tab completion
  const rl = readline.createInterface({
    input: process.stdin,
    output: process.stdout,
    terminal: true,
    completer
  })

  // Load history
  const historyFile = persistence.historyFile()
  try {
    const historyData = fs.readFileSync(historyFile, 'utf-8')
    const lines = historyData.split('\n').filter((l) => l.trim() !== '')
    // readline history is most-recent-first in the internal array
    for (const line of lines.reverse()) {
      rl.history.push(line)
    }
  } catch {
    // No history file yet
  }

  // REPL prompt
  const promptStr = `breez-spark-cli [${network}]> `

  console.log('Breez SDK CLI Interactive Mode')
  console.log("Type 'help' for available commands or 'exit' to quit")

  const askQuestion = () => {
    rl.question(promptStr, async (line) => {
      const trimmed = (line || '').trim()
      if (!trimmed) {
        askQuestion()
        return
      }

      // Add to history
      if (rl.history && rl.history[0] !== trimmed) {
        // readline auto-adds to history, but we want to also save it
      }

      if (trimmed === 'exit' || trimmed === 'quit') {
        await shutdown()
        return
      }

      if (trimmed === 'help' || trimmed === '-h') {
        const helpProgram = buildProgram(
          () => sdk,
          () => tokenIssuer,
          () => getSparkStatus,
          rl
        )
        helpProgram.outputHelp()
        askQuestion()
        return
      }

      try {
        // Parse the command line into args using shell-quote
        const parsedArgs = parseShell(trimmed).map((entry) =>
          typeof entry === 'string' ? entry : String(entry)
        )

        const program = buildProgram(
          () => sdk,
          () => tokenIssuer,
          () => getSparkStatus,
          rl
        )
        await program.parseAsync(parsedArgs, { from: 'user' })
      } catch (e) {
        // commander's exitOverride throws for --help and parsing errors
        if (e.code !== 'commander.helpDisplayed' &&
            e.code !== 'commander.help' &&
            e.code !== 'commander.version' &&
            e.code !== 'commander.unknownCommand' &&
            e.code !== 'commander.unknownOption' &&
            e.code !== 'commander.missingArgument' &&
            e.code !== 'commander.missingMandatoryOptionValue' &&
            e.code !== 'commander.invalidArgument') {
          console.error(`Error: ${e.message || e}`)
        }
      }

      askQuestion()
    })
  }

  async function shutdown() {
    // Save history
    try {
      const historyLines = rl.history ? [...rl.history].reverse().join('\n') : ''
      fs.writeFileSync(historyFile, historyLines)
    } catch {
      // Ignore history save errors
    }

    try {
      await sdk.disconnect()
    } catch (e) {
      console.error(`Failed to gracefully stop SDK: ${e.message || e}`)
    }

    logStream.end()
    rl.close()
    console.log('Goodbye!')
    process.exit(0)
  }

  // Handle CTRL-C and CTRL-D
  rl.on('close', async () => {
    await shutdown()
  })

  process.on('SIGINT', async () => {
    console.log('\nCTRL-C')
    await shutdown()
  })

  // Start the REPL
  askQuestion()
}

main().catch((err) => {
  console.error(`Fatal error: ${err.message || err}`)
  process.exit(1)
})
