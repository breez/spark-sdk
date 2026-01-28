import {
  SdkBuilder,
  Seed,
  defaultConfig,
  Network,
  ChainApiType,
  KeySetType,
  type KeySetConfig,
  type ProvisionalPayment,
  type Credentials
} from '@breeztech/breez-sdk-spark-react-native'
import RNFS from 'react-native-fs'

const exampleGettingStartedAdvanced = async () => {
  // ANCHOR: init-sdk-advanced
  // Construct the seed using mnemonic words or entropy bytes
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
}

const exampleWithPaymentObserver = async (builder: SdkBuilder) => {
  const paymentObserver = new ExamplePaymentObserver()
  await builder.withPaymentObserver(paymentObserver)
}
// ANCHOR_END: with-payment-observer
