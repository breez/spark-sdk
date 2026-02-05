import {
  defaultConfig,
  connect,
  Network,
  SdkBuilder,
  type BreezSdk,
  initLogging,
  type LogEntry,
  type SdkEvent,
  SdkEvent_Tags,
  Seed,
  getSparkStatus,
  ServiceStatus
} from '@breeztech/breez-sdk-spark-react-native'
import RNFS from 'react-native-fs'

const exampleGettingStarted = async () => {
  // ANCHOR: init-sdk
  // Construct the seed using mnemonic words or entropy bytes
  const mnemonic = '<mnemonics words>'
  const seed = new Seed.Mnemonic({ mnemonic, passphrase: undefined })

  // Create the default config
  const config = defaultConfig(Network.Mainnet)
  config.apiKey = '<breez api key>'

  const sdk = await connect({
    config,
    seed,
    storageDir: `${RNFS.DocumentDirectoryPath}/data`
  })
  // ANCHOR_END: init-sdk
}

const exampleFetchNodeInfo = async (sdk: BreezSdk) => {
  // ANCHOR: fetch-balance
  // ensureSynced: true will ensure the SDK is synced with the Spark network
  // before returning the balance
  const info = await sdk.getInfo({
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
  initLogging(undefined, logger, undefined)
  // ANCHOR_END: logging
}

const exampleAddEventListener = async (sdk: BreezSdk) => {
  // ANCHOR: add-event-listener
  class JsEventListener {
    onEvent = async (event: SdkEvent) => {
      if (event.tag === SdkEvent_Tags.Synced) {
        // Data has been synchronized with the network. When this event is received,
        // it is recommended to refresh the payment list and wallet balance.
      } else if (event.tag === SdkEvent_Tags.UnclaimedDeposits) {
        // SDK was unable to claim some deposits automatically
        const unclaimedDeposits = event.inner.unclaimedDeposits
      } else if (event.tag === SdkEvent_Tags.ClaimedDeposits) {
        // Deposits were successfully claimed
        const claimedDeposits = event.inner.claimedDeposits
      } else if (event.tag === SdkEvent_Tags.PaymentSucceeded) {
        // A payment completed successfully
        const payment = event.inner.payment
      } else if (event.tag === SdkEvent_Tags.PaymentPending) {
        // A payment is pending (waiting for confirmation)
        const pendingPayment = event.inner.payment
      } else if (event.tag === SdkEvent_Tags.PaymentFailed) {
        // A payment failed
        const failedPayment = event.inner.payment
      } else if (event.tag === SdkEvent_Tags.Optimization) {
        // An optimization event occurred
        const optimizationEvent = event.inner.optimizationEvent
      } else {
        // Handle any future event types
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
    case ServiceStatus.Operational:
      console.log('Spark is fully operational')
      break
    case ServiceStatus.Degraded:
      console.log('Spark is experiencing degraded performance')
      break
    case ServiceStatus.Partial:
      console.log('Spark is partially unavailable')
      break
    case ServiceStatus.Major:
      console.log('Spark is experiencing a major outage')
      break
  }

  console.log(`Last updated: ${sparkStatus.lastUpdated}`)
  // ANCHOR_END: spark-status
}

const exampleDisconnect = async (sdk: BreezSdk) => {
  // ANCHOR: disconnect
  await sdk.disconnect()
  // ANCHOR_END: disconnect
}
