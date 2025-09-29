import {
  BreezSdk,
  connect,
  defaultConfig,
  defaultStorage,
  initLogging,
  type LogEntry,
  type SdkEvent,
  SdkBuilder,
  Seed
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
  let config = defaultConfig('mainnet')
  config.apiKey = '<breez api key>'

  // Connect to the SDK using the simplified connect method
  const sdk = await connect({
    config,
    seed,
    storageDir: './.data'
  })
  // ANCHOR_END: init-sdk
}

const exampleGettingStartedAdvanced = async () => {
  // ANCHOR: init-sdk-advanced
  // Call init when using the SDK in a web environment before calling any other SDK
  // methods. This is not needed when using the SDK in a Node.js/Deno environment.
  await init()

  // Construct the seed using mnemonic words or entropy bytes
  const mnemonic = '<mnemonic words>'
  const seed: Seed = { type: 'mnemonic', mnemonic, passphrase: undefined }

  // Create the default config
  let config = defaultConfig('mainnet')
  config.apiKey = '<breez api key>'

  // Create the default storage
  const storage = await defaultStorage('./.data')

  // Build the SDK using the config, seed and storage
  const builder = SdkBuilder.new(config, seed, storage)

  // You can also pass your custom implementations:
  // builder = builder.withChainService(<your chain service implementation>)
  // builder = builder.withRestClient(<your rest client implementation>)
  // builder = builder.withKeySet(<your key set type>, <use address index>)
  const sdk = await builder.build()
  // ANCHOR_END: init-sdk-advanced
}

const exampleFetchNodeInfo = async (sdk: BreezSdk) => {
  // ANCHOR: fetch-balance
  const info = await sdk.getInfo({})
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
  initLogging(logger)
  // ANCHOR_END: logging
}

const exampleAddEventListener = async (sdk: BreezSdk) => {
  // ANCHOR: add-event-listener
  class JsEventListener {
    onEvent = async (event: SdkEvent) => {
      console.log(`Received event: ${JSON.stringify(event)}`)
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

const exampleDisconnect = (sdk: BreezSdk) => {
  // ANCHOR: disconnect
  sdk.disconnect()
  // ANCHOR_END: disconnect
}
