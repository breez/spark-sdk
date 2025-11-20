import BreezSdkSpark

func initSdkAdvanced() async throws -> BreezSdk {
    // ANCHOR: init-sdk-advanced
    // Construct the seed using mnemonic words or entropy bytes
    let mnemonic = "<mnemonic words>"
    let seed = Seed.mnemonic(mnemonic: mnemonic, passphrase: nil)

    // Create the default config
    var config = defaultConfig(network: Network.mainnet)
    config.apiKey = "<breez api key>"

    // Build the SDK using the config, seed and default storage
    let builder = SdkBuilder(config: config, seed: seed)
    await builder.withDefaultStorage(storageDir: "./.data")
    // You can also pass your custom implementations:
    // await builder.withStorage(<your storage implementation>)
    // await builder.withRealTimeSyncStorage(<your real-time sync storage implementation>)
    // await builder.withChainService(<your chain service implementation>)
    // await builder.withRestClient(<your rest client implementation>)
    // await builder.withKeySet(<your key set type>, <use address index>, <account number>)
    // await builder.withPaymentObserver(<your payment observer implementation>)
    let sdk = try await builder.build()
    // ANCHOR_END: init-sdk-advanced

    return sdk
}

func withRestChainService(builder: SdkBuilder) async {
    // ANCHOR: with-rest-chain-service
    let url = "<your REST chain service URL>"
    let chainApiType = ChainApiType.mempoolSpace
    let optionalCredentials = Credentials(
        username: "<username>",
        password: "<password>"
    )
    await builder.withRestChainService(
        url: url,
        apiType: chainApiType,
        credentials: optionalCredentials
    )
    // ANCHOR_END: with-rest-chain-service
}

func withKeySet(builder: SdkBuilder) async {
    // ANCHOR: with-key-set
    let keySetType = KeySetType.default
    let useAddressIndex = false
    let optionalAccountNumber = UInt32(21)
    await builder.withKeySet(
        keySetType: keySetType,
        useAddressIndex: useAddressIndex,
        accountNumber: optionalAccountNumber
    )
    // ANCHOR_END: with-key-set
}

// ANCHOR: with-storage
public protocol Storage {
    func deleteCachedItem(key: String) async throws 
    func getCachedItem(key: String) async throws  -> String?
    func setCachedItem(key: String, value: String) async throws 
    func listPayments(request: ListPaymentsRequest) async throws  -> [Payment]
    func insertPayment(payment: Payment) async throws 
    func setPaymentMetadata(paymentId: String, metadata: PaymentMetadata) async throws 
    func getPaymentById(id: String) async throws  -> Payment
    func getPaymentByInvoice(invoice: String) async throws  -> Payment?
    func addDeposit(txid: String, vout: UInt32, amountSats: UInt64) async throws 
    func deleteDeposit(txid: String, vout: UInt32) async throws 
    func listDeposits() async throws  -> [DepositInfo]
    func updateDeposit(txid: String, vout: UInt32, payload: UpdateDepositPayload) async throws 
}
// ANCHOR_END: with-storage

// ANCHOR: with-sync-storage
public protocol SyncStorage {
    func addOutgoingChange(record: UnversionedRecordChange) async throws  -> UInt64
    func completeOutgoingSync(record: Record) async throws 
    func getPendingOutgoingChanges(limit: UInt32) async throws  -> [OutgoingChange]
    func getLastRevision() async throws  -> UInt64
    func insertIncomingRecords(records: [Record]) async throws 
    func deleteIncomingRecord(record: Record) async throws 
    func rebasePendingOutgoingRecords(revision: UInt64) async throws 
    func getIncomingRecords(limit: UInt32) async throws  -> [IncomingChange]
    func getLatestOutgoingChange() async throws  -> OutgoingChange?
    func updateRecordFromIncoming(record: Record) async throws 
}
// ANCHOR_END: with-sync-storage

// ANCHOR: with-chain-service
protocol BitcoinChainService {
    func getAddressUtxos(address: String) async throws -> [Utxo]
    func getTransactionStatus(txid: String) async throws -> TxStatus
    func getTransactionHex(txid: String) async throws -> String
    func broadcastTransaction(tx: String) async throws
}
// ANCHOR_END: with-chain-service

// ANCHOR: with-rest-client
public protocol RestClient {
    func getRequest(url: String, headers: [String: String]?) async throws  -> RestResponse
    func postRequest(url: String, headers: [String: String]?, body: String?) async throws  -> RestResponse
    func deleteRequest(url: String, headers: [String: String]?, body: String?) async throws  -> RestResponse
}
// ANCHOR_END: with-rest-client

// ANCHOR: with-fiat-service
public protocol FiatService {
    func fetchFiatCurrencies() async throws  -> [FiatCurrency]
    func fetchFiatRates() async throws  -> [Rate]
}
// ANCHOR_END: with-fiat-service

// ANCHOR: with-payment-observer
protocol PaymentObserver {
    func beforeSend(payments: [ProvisionalPayment]) async throws
}
// ANCHOR_END: with-payment-observer
