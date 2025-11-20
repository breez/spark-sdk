using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class SdkBuilding
    {
        async Task InitSdkAdvanced()
        {
            // ANCHOR: init-sdk-advanced
            // Construct the seed using mnemonic words or entropy bytes
            var mnemonic = "<mnemonic words>";
            var seed = new Seed.Mnemonic(mnemonic: mnemonic, passphrase: null);
            // Create the default config
            var config = BreezSdkSparkMethods.DefaultConfig(Network.Mainnet) with
            {
                apiKey = "<breez api key>"
            };
            // Build the SDK using the config, seed and default storage
            var builder = new SdkBuilder(config: config, seed: seed);
            await builder.WithDefaultStorage(storageDir: "./.data");
            // You can also pass your custom implementations:
            // await builder.WithStorage(<your storage implementation>)
            // await builder.WithRealTimeSyncStorage(<your real-time sync storage implementation>)
            // await builder.WithChainService(<your chain service implementation>)
            // await builder.WithRestClient(<your rest client implementation>)
            // await builder.WithKeySet(<your key set type>, <use address index>, <account number>)
            // await builder.WithPaymentObserver(<your payment observer implementation>);
            var sdk = await builder.Build();
            // ANCHOR_END: init-sdk-advanced
        }

        async Task WithRestChainService(SdkBuilder builder)
        {
            // ANCHOR: with-rest-chain-service
            var url = "<your REST chain service URL>";
            var chainApiType = ChainApiType.MempoolSpace;
            var optionalCredentials = new Credentials(
                username: "<username>",
                password: "<password>"
            );
            await builder.WithRestChainService(
                url: url,
                apiType: chainApiType,
                credentials: optionalCredentials
            );
            // ANCHOR_END: with-rest-chain-service
        }

        async Task WithKeySet(SdkBuilder builder)
        {
            // ANCHOR: with-key-set
            var keySetType = KeySetType.Default;
            var useAddressIndex = false;
            var optionalAccountNumber = 21u;
            await builder.WithKeySet(
                keySetType: keySetType,
                useAddressIndex: useAddressIndex,
                accountNumber: optionalAccountNumber
            );
            // ANCHOR_END: with-key-set
        }

        // ANCHOR: with-storage
        public interface Storage
        {
            Task DeleteCachedItem(string @key);
            Task<string?> GetCachedItem(string @key);
            Task SetCachedItem(string @key, string @value);
            Task<List<Payment>> ListPayments(ListPaymentsRequest @request);
            Task InsertPayment(Payment @payment);
            Task SetPaymentMetadata(string @paymentId, PaymentMetadata @metadata);
            Task<Payment> GetPaymentById(string @id);
            Task<Payment?> GetPaymentByInvoice(string @invoice);
            Task AddDeposit(string @txid, uint @vout, ulong @amountSats);
            Task DeleteDeposit(string @txid, uint @vout);
            Task<List<DepositInfo>> ListDeposits();
            Task UpdateDeposit(string @txid, uint @vout, UpdateDepositPayload @payload);
        }
        // ANCHOR_END: with-storage

        // ANCHOR: with-sync-storage
        public interface SyncStorage
        {
            Task<ulong> AddOutgoingChange(UnversionedRecordChange @record);
            Task CompleteOutgoingSync(Record @record);
            Task<List<OutgoingChange>> GetPendingOutgoingChanges(uint @limit);
            Task<ulong> GetLastRevision();
            Task InsertIncomingRecords(List<Record> @records);
            Task DeleteIncomingRecord(Record @record);
            Task RebasePendingOutgoingRecords(ulong @revision);
            Task<List<IncomingChange>> GetIncomingRecords(uint @limit);
            Task<OutgoingChange?> GetLatestOutgoingChange();
            Task UpdateRecordFromIncoming(Record @record);
        }
        // ANCHOR_END: with-sync-storage

        // ANCHOR: with-chain-service
        public interface BitcoinChainService
        {
            Task<List<Utxo>> GetAddressUtxos(string @address);
            Task<TxStatus> GetTransactionStatus(string @txid);
            Task<string> GetTransactionHex(string @txid);
            Task BroadcastTransaction(string @tx);
            Task<RecommendedFees> RecommendedFees();
        }
        // ANCHOR_END: with-chain-service

        // ANCHOR: with-rest-client
        public interface RestClient
        {
            Task<RestResponse> GetRequest(string @url, Dictionary<string, string>? @headers);
            Task<RestResponse> PostRequest(string @url, Dictionary<string, string>? @headers, string? @body);
            Task<RestResponse> DeleteRequest(string @url, Dictionary<string, string>? @headers, string? @body);
        }
        // ANCHOR_END: with-rest-client

        // ANCHOR: with-fiat-service
        public interface FiatService
        {
            Task<List<FiatCurrency>> FetchFiatCurrencies();
            Task<List<Rate>> FetchFiatRates();
        }
        // ANCHOR_END: with-fiat-service

        // ANCHOR: with-payment-observer
        public interface PaymentObserver
        {
            Task BeforeSend(List<ProvisionalPayment> payments);
        }
        // ANCHOR_END: with-payment-observer
    }
}
