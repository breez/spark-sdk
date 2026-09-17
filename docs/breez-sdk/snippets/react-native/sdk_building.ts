import {
  type BreezSdk,
  SdkBuilder,
  Seed,
  defaultConfig,
  defaultSessionStore,
  defaultStorage,
  Network,
  ChainApiType,
  type PaymentIdUpdate,
  type ProvisionalPayment,
  type Credentials,
  type Session,
  type SessionStore
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
  // await builder.withAccountNumber(<account number>)
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

const exampleWithAccountNumber = async (builder: SdkBuilder) => {
  // ANCHOR: with-account-number
  await builder.withAccountNumber(21)
  // ANCHOR_END: with-account-number
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
      console.log(`Token tx broadcast: ${update.provisionalPaymentId} -> ${update.finalPaymentId}`)
    }
  }
}

const exampleWithPaymentObserver = async (builder: SdkBuilder) => {
  const paymentObserver = new ExamplePaymentObserver()
  await builder.withPaymentObserver(paymentObserver)
}
// ANCHOR_END: with-payment-observer

// ANCHOR: with-session-store
class EncryptingSessionStore implements SessionStore {
  constructor (private readonly inner: SessionStore) {}

  getSession = async (serviceIdentityKey: string): Promise<Session> => {
    const session = await this.inner.getSession(serviceIdentityKey)
    // Decrypt session.token here before returning it.
    return session
  }

  setSession = async (serviceIdentityKey: string, session: Session): Promise<void> => {
    // Encrypt session.token here before persisting it.
    await this.inner.setSession(serviceIdentityKey, session)
  }
}

// `identity` is the wallet identity public key bytes, used to scope the store.
const exampleWithSessionStore = async (identity: ArrayBuffer): Promise<SdkBuilder> => {
  // Construct the seed using a mnemonic, entropy or passkey
  const mnemonic = '<mnemonic words>'
  const seed = new Seed.Mnemonic({ mnemonic, passphrase: undefined })

  // Create the default config
  const config = defaultConfig(Network.Mainnet)
  config.apiKey = '<breez api key>'

  // Reuse one storage backend for both the SDK storage and the session store.
  const backend = defaultStorage('./.data')
  // Get the session store the backend provides, then wrap it to add encryption.
  const inner = await defaultSessionStore(backend, config.network, identity)
  const sessionStore = new EncryptingSessionStore(inner)

  const builder = new SdkBuilder(config, seed)
  await builder.withStorageBackend(backend)
  await builder.withSessionStore(sessionStore)
  return builder
}
// ANCHOR_END: with-session-store

const exampleRefundPendingConversions = async (sdk: BreezSdk) => {
  // ANCHOR: refund-pending-conversions
  // The flashnet conversion refunder doesn't run in the background in server
  // mode. Call this from your own scheduler (e.g. once per minute) to issue
  // pending refunds for failed conversions.
  await sdk.refundPendingConversions()
  // ANCHOR_END: refund-pending-conversions
}
