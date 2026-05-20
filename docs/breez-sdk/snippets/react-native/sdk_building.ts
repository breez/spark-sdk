import {
  type BreezSdk,
  SdkBuilder,
  Seed,
  defaultConfig,
  defaultServerConfig,
  Network,
  ChainApiType,
  KeySetType,
  type KeySetConfig,
  type PaymentIdUpdate,
  type ProvisionalPayment,
  type Credentials,
  ReceivePaymentMethod
} from '@breeztech/breez-sdk-spark-react-native'
import RNFS from 'react-native-fs'

const exampleGettingStartedAdvanced = async () => {
  // ANCHOR: init-sdk-advanced
  // Construct the seed using a mnemonic, entropy or passkey
  const mnemonic = '<mnemonics words>'
  const seed = new Seed.Mnemonic({ mnemonic, passphrase: undefined })

  // Create the default config
  const config = defaultConfig(Network.Mainnet)
  config.apiKey = '<breez api key>'

  // Build the SDK using the config, seed and default storage
  const builder = new SdkBuilder(config, seed)
  await builder.withDefaultStorage(`${RNFS.DocumentDirectoryPath}/data`)
  // You can also pass your custom implementations:
  // await builder.withStorage(<your storage implementation>)
  // await builder.withChainService(<your chain service implementation>)
  // await builder.withRestClient(<your rest client implementation>)
  // await builder.withKeySet({ keySetType: <your key set type>, useAddressIndex: <use address index>, accountNumber: <account number> })
  // await builder.withPaymentObserver(<your payment observer implementation>)
  const sdk = await builder.build()
  // ANCHOR_END: init-sdk-advanced
}

const exampleWithRestChainService = async (builder: SdkBuilder) => {
  // ANCHOR: with-rest-chain-service
  const url = '<your REST chain service URL>'
  const chainApiType = ChainApiType.MempoolSpace
  const optionalCredentials: Credentials = {
    username: '<username>',
    password: '<password>'
  }
  await builder.withRestChainService(url, chainApiType, optionalCredentials)
  // ANCHOR_END: with-rest-chain-service
}

const exampleWithKeySet = async (builder: SdkBuilder) => {
  // ANCHOR: with-key-set
  const keySetConfig: KeySetConfig = {
    keySetType: KeySetType.Default,
    useAddressIndex: false,
    accountNumber: 21
  }
  await builder.withKeySet(keySetConfig)
  // ANCHOR_END: with-key-set
}

// ANCHOR: with-payment-observer
class ExamplePaymentObserver {
  beforeSend = async (payments: ProvisionalPayment[]) => {
    for (const payment of payments) {
      console.log(`About to send payment: ${payment.paymentId} of amount ${payment.amount}`)
    }
  }

  afterSend = async (updates: PaymentIdUpdate[]) => {
    for (const update of updates) {
      console.log(`Token tx broadcast: ${update.partialTxId} -> ${update.finalTxId}`)
    }
  }
}

const exampleWithPaymentObserver = async (builder: SdkBuilder) => {
  const paymentObserver = new ExamplePaymentObserver()
  await builder.withPaymentObserver(paymentObserver)
}
// ANCHOR_END: with-payment-observer

const exampleInitSdkServer = async () => {
  // ANCHOR: init-sdk-server
  // Construct the seed using a mnemonic, entropy or passkey
  const mnemonic = '<mnemonics words>'
  const seed = new Seed.Mnemonic({ mnemonic, passphrase: undefined })

  // Build a server-mode config: same as defaultConfig(network) with
  // backgroundTasksEnabled = false. No periodic sync, no real-time sync
  // client, no leaf/token optimizer, no flashnet refunder, no lightning-
  // address recovery, no spark private-mode init.
  const config = defaultServerConfig(Network.Mainnet)
  config.apiKey = '<breez api key>'

  // Typically server-mode SDKs are built per request and share infrastructure
  // (DB pool, REST chain service, SSP/Connection Manager) across instances.
  // Pass the shared resources via the builder.
  const builder = new SdkBuilder(config, seed)
  await builder.withDefaultStorage(`${RNFS.DocumentDirectoryPath}/data`)
  const sdk = await builder.build()
  // ANCHOR_END: init-sdk-server
  return sdk
}

const exampleServerModeRequestHandler = async (sdk: BreezSdk) => {
  // ANCHOR: server-mode-request-handler
  // User-facing request handler: do not call syncWallet here. Operations
  // that read from local storage (getInfo, listPayments, etc.) do not need
  // a defensive sync. Call syncWallet only from webhook handlers or
  // reconciliation jobs that need to observe an external state change.
  const response = await sdk.receivePayment({
    paymentMethod: new ReceivePaymentMethod.Bolt11Invoice({
      description: '<invoice description>',
      amountSats: BigInt(5_000),
      expirySecs: 3600,
      paymentHash: undefined
    })
  })

  // Always disconnect at the end of the request lifecycle to flush
  // outstanding storage writes.
  await sdk.disconnect()
  // ANCHOR_END: server-mode-request-handler
  return response.paymentRequest
}

const exampleServerModeProvisioning = async (sdk: BreezSdk) => {
  // ANCHOR: server-mode-provisioning
  // One-time setup when a wallet is first registered. The client-mode SDK
  // would normally apply the private-mode preset itself on first startup;
  // server-mode SDKs do not, so opt in once here via updateUserSettings.
  await sdk.updateUserSettings({
    sparkPrivateModeEnabled: true,
    stableBalanceActiveLabel: undefined
  })

  await sdk.disconnect()
  // ANCHOR_END: server-mode-provisioning
}

const exampleRefundPendingConversions = async (sdk: BreezSdk) => {
  // ANCHOR: refund-pending-conversions
  // The flashnet conversion refunder doesn't run in the background in server
  // mode. Call this from your own scheduler (e.g. once per minute) to issue
  // pending refunds for failed conversions.
  await sdk.refundPendingConversions()
  // ANCHOR_END: refund-pending-conversions
}
