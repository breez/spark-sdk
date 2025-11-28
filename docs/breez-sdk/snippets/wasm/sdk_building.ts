import { SdkBuilder, defaultConfig } from '@breeztech/breez-sdk-spark'
import type {
  ProvisionalPayment,
  Seed,
  TxStatus,
  Utxo,
  RestResponse,
  FiatCurrency,
  Rate,
  Payment,
  PaymentMetadata,
  ListPaymentsRequest,
  DepositInfo,
  UpdateDepositPayload,
  Record,
  UnversionedRecordChange,
  OutgoingChange,
  IncomingChange,
  Credentials
} from '@breeztech/breez-sdk-spark'

// Init stub
const init = async () => {}

const exampleGettingStartedAdvanced = async () => {
  // ANCHOR: init-sdk-advanced
  // Call init when using the SDK in a web environment before calling any other SDK
  // methods. This is not needed when using the SDK in a Node.js/Deno environment.
  await init()

  // Construct the seed using mnemonic words or entropy bytes
  const mnemonic = '<mnemonic words>'
  const seed: Seed = { type: 'mnemonic', mnemonic, passphrase: undefined }

  // Create the default config
  const config = defaultConfig('mainnet')
  config.apiKey = '<breez api key>'

  // Build the SDK using the config, seed and default storage
  let builder = SdkBuilder.new(config, seed)
  builder = await builder.withDefaultStorage('./.data')
  // You can also pass your custom implementations:
  // builder = builder.withStorage(<your storage implementation>)
  // builder = builder.withRealTimeSyncStorage(<your real-time sync storage implementation>)
  // builder = builder.withChainService(<your chain service implementation>)
  // builder = builder.withRestClient(<your rest client implementation>)
  // builder = builder.withKeySet(<your key set type>, <use address index>, <account number>)
  // builder = builder.withPaymentObserver(<your payment observer implementation>)
  const sdk = await builder.build()
  // ANCHOR_END: init-sdk-advanced
}

const exampleWithRestChainService = async (builder: SdkBuilder) => {
  // ANCHOR: with-rest-chain-service
  const url = '<your REST chain service URL>'
  const optionalCredentials: Credentials = {
    username: '<username>',
    password: '<password>'
  }
  builder = builder.withRestChainService(url, optionalCredentials)
  // ANCHOR_END: with-rest-chain-service
}

const exampleWithKeySet = async (builder: SdkBuilder) => {
  // ANCHOR: with-key-set
  const keySetType = 'default'
  const useAddressIndex = false
  const optionalAccountNumber = 21
  builder = builder.withKeySet(keySetType, useAddressIndex, optionalAccountNumber)
  // ANCHOR_END: with-key-set
}

// ANCHOR: with-payment-observer
class ExamplePaymentObserver {
  beforeSend = async (payments: ProvisionalPayment[]) => {
    for (const payment of payments) {
      console.log(`About to send payment: ${payment.paymentId} of amount ${payment.amount}`)
    }
  }
}

const exampleWithPaymentObserver = (builder: SdkBuilder): SdkBuilder => {
  const paymentObserver = new ExamplePaymentObserver()
  return builder.withPaymentObserver(paymentObserver)
}
// ANCHOR_END: with-payment-observer
