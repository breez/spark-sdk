import {
  SdkBuilder,
  Seed,
  defaultConfig,
  Network,
  ChainApiType,
  KeySetType
} from '@breeztech/breez-sdk-spark-react-native'
import type {
  ProvisionalPayment,
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
  // await builder.withRealTimeSyncStorage(<your real-time sync storage implementation>)
  // await builder.withChainService(<your chain service implementation>)
  // await builder.withRestClient(<your rest client implementation>)
  // await builder.withKeySet(<your key set type>, <use address index>, <account number>)
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
  const keySetType = KeySetType.Default
  const useAddressIndex = false
  const optionalAccountNumber = 21
  await builder.withKeySet(keySetType, useAddressIndex, optionalAccountNumber)
  // ANCHOR_END: with-key-set
}

// ANCHOR: with-storage
export interface Storage {
  deleteCachedItem: (key: string) => Promise<void>
  getCachedItem: (key: string) => Promise<string | undefined>
  setCachedItem: (key: string, value: string) => Promise<void>
  listPayments: (request: ListPaymentsRequest) => Promise<Payment[]>
  insertPayment: (payment: Payment) => Promise<void>
  setPaymentMetadata: (paymentId: string, metadata: PaymentMetadata) => Promise<void>
  getPaymentById: (id: string) => Promise<Payment>
  getPaymentByInvoice: (invoice: string) => Promise<Payment | undefined>
  addDeposit: (txid: string, vout: number, amountSats: bigint) => Promise<void>
  deleteDeposit: (txid: string, vout: number) => Promise<void>
  listDeposits: () => Promise<DepositInfo[]>
  updateDeposit: (txid: string, vout: number, payload: UpdateDepositPayload) => Promise<void>
}
// ANCHOR_END: with-storage

// ANCHOR: with-sync-storage
interface SyncStorage {
  addOutgoingChange: (record: UnversionedRecordChange) => Promise<bigint>
  completeOutgoingSync: (record: Record) => Promise<void>
  getPendingOutgoingChanges: (limit: number) => Promise<OutgoingChange[]>
  getLastRevision: () => Promise<bigint>
  insertIncomingRecords: (records: Record[]) => Promise<void>
  deleteIncomingRecord: (record: Record) => Promise<void>
  rebasePendingOutgoingRecords: (revision: bigint) => Promise<void>
  getIncomingRecords: (limit: number) => Promise<IncomingChange[]>
  getLatestOutgoingChange: () => Promise<OutgoingChange | undefined>
  updateRecordFromIncoming: (record: Record) => Promise<void>
}
// ANCHOR_END: with-sync-storage

// ANCHOR: with-chain-service
interface BitcoinChainService {
  getAddressUtxos: (address: string) => Promise<Utxo[]>
  getTransactionStatus: (txid: string) => Promise<TxStatus>
  getTransactionHex: (txid: string) => Promise<string>
  broadcastTransaction: (tx: string) => Promise<void>
}
// ANCHOR_END: with-chain-service

// ANCHOR: with-rest-client
interface RestClient {
  getRequest: (url: string, headers: Map<string, string> | undefined) => Promise<RestResponse>
  postRequest: (
    url: string,
    headers: Map<string, string> | undefined,
    body: string | undefined
  ) => Promise<RestResponse>
  deleteRequest: (
    url: string,
    headers: Map<string, string> | undefined,
    body: string | undefined
  ) => Promise<RestResponse>
}
// ANCHOR_END: with-rest-client

// ANCHOR: with-fiat-service
interface FiatService {
  fetchFiatCurrencies: () => Promise<FiatCurrency[]>
  fetchFiatRates: () => Promise<Rate[]>
}
// ANCHOR_END: with-fiat-service

// ANCHOR: with-payment-observer
interface PaymentObserver {
  beforeSend: (payments: ProvisionalPayment[]) => Promise<void>
}
// ANCHOR_END: with-payment-observer
