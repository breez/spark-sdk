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
  // Construct the seed using a mnemonic, entropy or passkey
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
      } else if (event.tag === SdkEvent_Tags.NewDeposits) {
        // Detected deposits, as DepositInfo. Only those with isMature set
        // have enough confirmations to be claimed. Show the rest as pending.
        const newDeposits = event.inner.newDeposits
      } else if (event.tag === SdkEvent_Tags.UnclaimedDeposits) {
        // Deposits the SDK could not claim. Each claimError says why,
        // most often the fee exceeded the configured maximum.
        const unclaimedDeposits = event.inner.unclaimedDeposits
      } else if (event.tag === SdkEvent_Tags.ClaimedDeposits) {
        // Deposits claimed into the wallet. The resulting payment
        // arrives separately as its own event.
        const claimedDeposits = event.inner.claimedDeposits
      } else if (event.tag === SdkEvent_Tags.PaymentSucceeded) {
        // A payment completed. The cached balance is already refreshed,
        // so getInfo returns the new value.
        const payment = event.inner.payment
      } else if (event.tag === SdkEvent_Tags.PaymentPending) {
        // A payment is awaiting confirmation. It arrives again as
        // succeeded or failed once it settles.
        const pendingPayment = event.inner.payment
      } else if (event.tag === SdkEvent_Tags.PaymentFailed) {
        // A payment failed. payment.details carries the method-specific
        // context to show the user.
        const failedPayment = event.inner.payment
      } else if (event.tag === SdkEvent_Tags.AutoOptimization) {
        // Background optimizer progress: started, round completed, or a
        // terminal outcome. Manual optimizeLeaves calls do not emit these.
        const optimizationEvent = event.inner.optimizationEvent
      } else if (event.tag === SdkEvent_Tags.LightningAddressChanged) {
        // The lightning address changed on another device. Unset when the
        // address was deleted.
        const lightningAddress = event.inner.lightningAddress
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
    case ServiceStatus.Unknown:
      console.log('Spark status is unknown')
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
