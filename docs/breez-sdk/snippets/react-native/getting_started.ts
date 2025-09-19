import {
  defaultConfig,
  connect,
  Network,
  defaultStorage,
  SdkBuilder,
  BreezSdk,
  initLogging,
  LogEntry,
  SdkEvent
} from '@breeztech/breez-sdk-spark-react-native'
import RNFS from 'react-native-fs'

const exampleGettingStarted = async () => {
  // ANCHOR: init-sdk
  const mnemonic = '<mnemonics words>'

  // Create the default config
  let config = defaultConfig(Network.Mainnet)
  config.apiKey = '<breez api key>'

  const sdk = await connect({
    config,
    mnemonic,
    storageDir: `${RNFS.DocumentDirectoryPath}/data`
  })
  // ANCHOR_END: init-sdk
}

const exampleGettingStartedAdvanced = async () => {
  // ANCHOR: init-sdk-advanced
  const mnemonic = '<mnemonics words>'

  // Create the default config
  let config = defaultConfig(Network.Mainnet)
  config.apiKey = '<breez api key>'

  // Create the default storage
  const storage = await defaultStorage(`${RNFS.DocumentDirectoryPath}/data`)

  const builder = new SdkBuilder(config, mnemonic, storage)
  // You can also pass your custom implementations:
  // builder.withRestChainService("https://custom.chain.service", {
  //   username: "service-username",
  //   password: "service-password",
  // });
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
  initLogging(undefined, logger, undefined)
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
