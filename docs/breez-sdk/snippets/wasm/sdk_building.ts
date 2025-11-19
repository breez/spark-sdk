import {
  type ProvisionalPayment,
  SdkBuilder,
  type Seed,
  type TxStatus,
  type Utxo,
  defaultConfig,
  type RestResponse,
  type FiatCurrency,
  type Rate,
  type Payment,
  type PaymentMetadata,
  type ListPaymentsRequest,
  type DepositInfo,
  type UpdateDepositPayload,
  type Record,
  type UnversionedRecordChange,
  type OutgoingChange,
  type IncomingChange
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

// ANCHOR: with-storage
export interface Storage {
  getCachedItem: (key: string) => Promise<string | null>
  setCachedItem: (key: string, value: string) => Promise<void>
  deleteCachedItem: (key: string) => Promise<void>
  listPayments: (request: ListPaymentsRequest) => Promise<Payment[]>
  insertPayment: (payment: Payment) => Promise<void>
  setPaymentMetadata: (paymentId: string, metadata: PaymentMetadata) => Promise<void>
  getPaymentById: (id: string) => Promise<Payment>
  getPaymentByInvoice: (invoice: string) => Promise<Payment>
  addDeposit: (txid: string, vout: number, amount_sats: number) => Promise<void>
  deleteDeposit: (txid: string, vout: number) => Promise<void>
  listDeposits: () => Promise<DepositInfo[]>
  updateDeposit: (txid: string, vout: number, payload: UpdateDepositPayload) => Promise<void>
  syncAddOutgoingChange: (record: UnversionedRecordChange) => Promise<number>
  syncCompleteOutgoingSync: (record: Record) => Promise<void>
  syncGetPendingOutgoingChanges: (limit: number) => Promise<OutgoingChange[]>
  syncGetLastRevision: () => Promise<number>
  syncInsertIncomingRecords: (records: Record[]) => Promise<void>
  syncDeleteIncomingRecord: (record: Record) => Promise<void>
  syncRebasePendingOutgoingRecords: (revision: number) => Promise<void>
  syncGetIncomingRecords: (limit: number) => Promise<IncomingChange[]>
  syncGetLatestOutgoingChange: () => Promise<OutgoingChange | null>
  syncUpdateRecordFromIncoming: (record: Record) => Promise<void>
}
// ANCHOR_END: with-storage

// ANCHOR: with-sync-storage
// ANCHOR_END: with-sync-storage

// ANCHOR: with-bitcoin-chain-service
interface BitcoinChainService {
  getAddressUtxos: (address: string) => Promise<Utxo[]>
  getTransactionStatus: (txid: string) => Promise<TxStatus>
  getTransactionHex: (txid: string) => Promise<string>
  broadcastTransaction: (tx: string) => Promise<void>
}
// ANCHOR_END: with-bitcoin-chain-service

// ANCHOR: with-rest-client
interface RestClient {
  getRequest: (url: string, headers?: any) => Promise<RestResponse>
  postRequest: (url: string, headers?: any, body?: string) => Promise<RestResponse>
  deleteRequest: (url: string, headers?: any, body?: string) => Promise<RestResponse>
}
// ANCHOR_END: with-rest-client

// ANCHOR: with-fiat-service
export interface FiatService {
  fetchFiatCurrencies: () => Promise<FiatCurrency[]>
  fetchFiatRates: () => Promise<Rate[]>
}
// ANCHOR_END: with-fiat-service

// ANCHOR: with-payment-observer
interface PaymentObserver {
  beforeSend: (payments: ProvisionalPayment[]) => Promise<void>
}
// ANCHOR_END: with-payment-observer
