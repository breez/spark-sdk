import {
  type BreezSdk,
  connect,
  defaultConfig,
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
  // Call init when using the SDK in a web environment before calling any other SDK
  // methods. This is not needed when using the SDK in a Node.js/Deno environment.
  //
  // import init, { BreezSdk, defaultConfig } from '@breeztech/breez-sdk-spark'
  await init()

  // Construct the seed using mnemonic words or entropy bytes
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

const exampleDisconnect = async (sdk: BreezSdk) => {
  // ANCHOR: disconnect
  await sdk.disconnect()
  // ANCHOR_END: disconnect
}
