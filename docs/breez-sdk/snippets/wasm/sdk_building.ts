import {
  SdkBuilder,
  defaultConfig,
  defaultPostgresStorageConfig,
  createPostgresConnectionPool,
  defaultMysqlStorageConfig,
  createMysqlConnectionPool,
  newRestChainService
} from '@breeztech/breez-sdk-spark'
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
const init = async () => { }

const exampleGettingStartedAdvanced = async () => {
  // ANCHOR: init-sdk-advanced
  // Call init when using the SDK in a web environment before calling any other SDK
  // methods. This is not needed when using the SDK in a Node.js/Deno environment.
  await init()

  // Construct the seed using a mnemonic, entropy or passkey
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
  // builder = builder.withChainService(<your chain service implementation>)
  // builder = builder.withRestClient(<your rest client implementation>)
  // builder = builder.withKeySet({ keySetType: <your key set type>, useAddressIndex: <use address index>, accountNumber: <account number> })
  // builder = builder.withPaymentObserver(<your payment observer implementation>)
  const sdk = await builder.build()
  // ANCHOR_END: init-sdk-advanced
}

const exampleWithPostgresStorage = async () => {
  // ANCHOR: init-sdk-postgres
  // Construct the seed using a mnemonic, entropy or passkey
  const mnemonic = '<mnemonic words>'
  const seed: Seed = { type: 'mnemonic', mnemonic, passphrase: undefined }

  // Create the default config
  const config = defaultConfig('mainnet')
  config.apiKey = '<breez api key>'

  // Configure PostgreSQL backend
  // Connection string format: "host=localhost user=postgres password=secret dbname=spark"
  // Or URI format: "postgres://user:password@host:port/dbname?sslmode=require"
  const pgConfig = defaultPostgresStorageConfig('host=localhost user=postgres dbname=spark')
  // Optionally pool settings can be adjusted. Some examples:
  pgConfig.maxPoolSize = 8 // Max connections in pool
  pgConfig.createTimeoutSecs = 30 // Timeout for establishing a new connection
  pgConfig.recycleTimeoutSecs = 30 // Timeout for recycling an idle connection
  // If your service owns SDK-compatible schema migrations:
  pgConfig.runMigration = false

  // Construct the connection pool. The same pool handle can be passed to
  // multiple SdkBuilders to share connections across SDKs; per-tenant
  // scoping (rows isolated by seed identity) is preserved.
  const pool = createPostgresConnectionPool(pgConfig)

  // Build the SDK with PostgreSQL backend (storage, tree store, and token store)
  let builder = SdkBuilder.new(config, seed)
  builder = builder.withPostgresConnectionPool(pool)
  const sdk = await builder.build()
  // ANCHOR_END: init-sdk-postgres
}

const exampleWithMysqlStorage = async () => {
  // ANCHOR: init-sdk-mysql
  // Construct the seed using a mnemonic, entropy or passkey
  const mnemonic = '<mnemonic words>'
  const seed: Seed = { type: 'mnemonic', mnemonic, passphrase: undefined }

  // Create the default config
  const config = defaultConfig('mainnet')
  config.apiKey = '<breez api key>'

  // Configure MySQL backend (MySQL 8.0+).
  // Connection string format (URL only):
  //   "mysql://user:password@host:3306/dbname?ssl-mode=required"
  const mysqlConfig = defaultMysqlStorageConfig('mysql://user:password@localhost:3306/spark')
  // Optionally pool settings can be adjusted. Some examples:
  mysqlConfig.maxPoolSize = 8 // Max connections in pool
  mysqlConfig.createTimeoutSecs = 30 // Timeout for establishing a new connection
  mysqlConfig.recycleTimeoutSecs = 60 // Recycle idle connections after this many seconds

  // Construct the connection pool. The same pool handle can be passed to
  // multiple SdkBuilders to share connections across SDKs; per-tenant
  // scoping (rows isolated by seed identity) is preserved.
  const pool = createMysqlConnectionPool(mysqlConfig)

  // Build the SDK with MySQL backend (storage, tree store, and token store)
  let builder = SdkBuilder.new(config, seed)
  builder = builder.withMysqlConnectionPool(pool)
  const sdk = await builder.build()
  // ANCHOR_END: init-sdk-mysql
}

const exampleWithRestChainService = async (builder: SdkBuilder) => {
  // ANCHOR: with-rest-chain-service
  const url = '<your REST chain service URL>'
  const chainApiType = 'mempoolSpace'
  const optionalCredentials: Credentials = {
    username: '<username>',
    password: '<password>'
  }
  builder = builder.withRestChainService(url, chainApiType, optionalCredentials)
  // ANCHOR_END: with-rest-chain-service
}

const exampleWithSharedRestChainService = async (builder: SdkBuilder) => {
  // ANCHOR: with-shared-rest-chain-service
  // Construct one chain service and reuse it across every SdkBuilder —
  // they share a single pooled HTTP client.
  const url = '<your REST chain service URL>'
  const chainApiType = 'mempoolSpace'
  const optionalCredentials: Credentials = {
    username: '<username>',
    password: '<password>'
  }
  const chainService = newRestChainService(url, 'mainnet', chainApiType, optionalCredentials)
  builder = builder.withChainService(chainService)
  // ANCHOR_END: with-shared-rest-chain-service
}

const exampleWithKeySet = async (builder: SdkBuilder) => {
  // ANCHOR: with-key-set
  builder = builder.withKeySet({
    keySetType: 'default',
    useAddressIndex: false,
    accountNumber: 21
  })
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
