import {
  type BreezSdk,
  connect,
  defaultConfig,
  getSparkStatus,
  initLogging,
  type LogEntry,
  type SdkEvent,
  SdkBuilder,
  type Seed
} from '@breeztech/breez-sdk-spark'

// Init stub
const init = async () => {}

const exampleGettingStarted = async () => {
  // ANCHOR: init-sdk
  // Call init to load the WASM module before calling any other SDK methods.
  // This is not needed when using the SDK via require() in Node.js.
  //
  // For SSR frameworks (Next.js, SvelteKit, Nuxt), use the /ssr subpath:
  //   import init, { connect } from '@breeztech/breez-sdk-spark/ssr'
  // The /ssr import is safe during server-side rendering. Call init() on the
  // client only (e.g., inside useEffect or onMount).
  //
  // import init from '@breeztech/breez-sdk-spark'
  await init()

  // Construct the seed using a mnemonic, entropy or passkey
  const mnemonic = '<mnemonic words>'
  const seed: Seed = { type: 'mnemonic', mnemonic, passphrase: undefined }

  // Create the default config
  const config = defaultConfig('mainnet')
  config.apiKey = '<breez api key>'

  // Connect to the SDK using the simplified connect method
  const sdk = await connect({
    config,
    seed,
    storageDir: './.data'
  })
  // ANCHOR_END: init-sdk
}

const exampleFetchNodeInfo = async (sdk: BreezSdk) => {
  // ANCHOR: fetch-balance
  const info = await sdk.getInfo({
    // ensureSynced: true will ensure the SDK is synced with the Spark network
    // before returning the balance
    ensureSynced: false
  })
  const identityPubkey = info.identityPubkey
  const balanceSats = info.balanceSats
  // ANCHOR_END: fetch-balance
}

const exampleLogging = async () => {
  // ANCHOR: logging
  class JsLogger {
    log = (l: LogEntry) => {
      console.log(`[${l.level}]: ${l.line}`)
    }
  }

  const logger = new JsLogger()
  await initLogging(logger)
  // ANCHOR_END: logging
}

const exampleAddEventListener = async (sdk: BreezSdk) => {
  // ANCHOR: add-event-listener
  class JsEventListener {
    onEvent = async (event: SdkEvent) => {
      switch (event.type) {
        case 'synced': {
          // Data has been synchronized with the network. When this event is received,
          // it is recommended to refresh the payment list and wallet balance.
          break
        }
        case 'newDeposits': {
          // New deposits were detected (may be pending or confirmed)
          const newDeposits = event.newDeposits
          break
        }
        case 'unclaimedDeposits': {
          // SDK was unable to claim some deposits automatically
          const unclaimedDeposits = event.unclaimedDeposits
          break
        }
        case 'claimedDeposits': {
          // Deposits were successfully claimed
          const claimedDeposits = event.claimedDeposits
          break
        }
        case 'paymentSucceeded': {
          // A payment completed successfully
          const payment = event.payment
          break
        }
        case 'paymentPending': {
          // A payment is pending (waiting for confirmation)
          const pendingPayment = event.payment
          break
        }
        case 'paymentFailed': {
          // A payment failed
          const failedPayment = event.payment
          break
        }
        case 'optimization': {
          // An optimization event occurred
          const optimizationEvent = event.optimizationEvent
          break
        }
        case 'lightningAddressChanged': {
          // The lightning address has changed
          const lightningAddress = event.lightningAddress
          break
        }
        default: {
          // Handle any future event types
          break
        }
      }
    }
  }

  const eventListener = new JsEventListener()

  const listenerId = await sdk.addEventListener(eventListener)
  // ANCHOR_END: add-event-listener
}

const exampleRemoveEventListener = async (sdk: BreezSdk, listenerId: string) => {
  // ANCHOR: remove-event-listener
  await sdk.removeEventListener(listenerId)
  // ANCHOR_END: remove-event-listener
}

const exampleGetSparkStatus = async () => {
  // ANCHOR: spark-status
  const sparkStatus = await getSparkStatus()

  switch (sparkStatus.status) {
    case 'operational': {
      console.log('Spark is fully operational')
      break
    }
    case 'degraded': {
      console.log('Spark is experiencing degraded performance')
      break
    }
    case 'partial': {
      console.log('Spark is partially unavailable')
      break
    }
    case 'major': {
      console.log('Spark is experiencing a major outage')
      break
    }
    case 'unknown': {
      console.log('Spark status is unknown')
      break
    }
  }

  console.log(`Last updated: ${sparkStatus.lastUpdated}`)
  // ANCHOR_END: spark-status
}

const exampleDisconnect = async (sdk: BreezSdk) => {
  // ANCHOR: disconnect
  await sdk.disconnect()
  // ANCHOR_END: disconnect
}

const exampleConnectWithRecovery = async () => {
  // ANCHOR: corrupt-storage-error
  const storageDir = './.data'

  const seed: Seed = { type: 'mnemonic', mnemonic: '<mnemonic words>', passphrase: undefined }
  const connectRequest = {
    config: (() => {
      const config = defaultConfig('mainnet')
      config.apiKey = '<breez api key>'
      return config
    })(),
    seed,
    storageDir
  }

  let sdk: BreezSdk
  try {
    sdk = await connect(connectRequest)
  } catch (error) {
    if (error instanceof Error && error.message.startsWith('Corrupt storage')) {
      // The SDK storage is corrupted and cannot be recovered by retrying.
      // Clear the storage directory and reconnect with fresh storage.
      // (Platform-specific: delete the storageDir directory or clear IndexedDB)
      sdk = await connect(connectRequest)
    } else {
      throw error
    }
  }
  // ANCHOR_END: corrupt-storage-error
}
