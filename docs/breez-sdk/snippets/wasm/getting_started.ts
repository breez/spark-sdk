import {
  type BreezSdk,
  defaultConfig,
  defaultStorage,
  initLogging,
  type LogEntry,
  type SdkEvent,
  SdkBuilder
} from '@breeztech/breez-sdk-spark'

// Init stub
const init = async () => {}

const exampleGettingStarted = async () => {
  // ANCHOR: init-sdk
  // Call init when using the SDK in a web environment before calling any other SDK
  // methods. This is not needed when using the SDK in a Node.js/Deno environment.
  //
  // import init, { defaultConfig, connect } from '@breeztech/breez-sdk-spark'
  await init()

  const mnemonic = '<mnemonic words>'
  // Create the default config
  let config = defaultConfig('mainnet')
  config.apiKey = '<breez api key>'

  // Create the default storage
  const storage = await defaultStorage('./.data')

  // Build the SDK using the config, mnemonic and storage
  const builder = SdkBuilder.new(config, mnemonic, storage)
  const sdk = await builder.build()
  // ANCHOR_END: init-sdk
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

const exampleAddEventListener = (sdk: BreezSdk) => {
  // ANCHOR: add-event-listener
  class JsEventListener {
    onEvent = (event: SdkEvent) => {
      console.log(`Received event: ${JSON.stringify(event)}`)
    }
  }

  const eventListener = new JsEventListener()

  const listenerId = sdk.addEventListener(eventListener)
  // ANCHOR_END: add-event-listener
}

const exampleRemoveEventListener = (sdk: BreezSdk, listenerId: string) => {
  // ANCHOR: remove-event-listener
  sdk.removeEventListener(listenerId)
  // ANCHOR_END: remove-event-listener
}

const exampleDisconnect = (sdk: BreezSdk) => {
  // ANCHOR: disconnect
  sdk.disconnect()
  // ANCHOR_END: disconnect
}
