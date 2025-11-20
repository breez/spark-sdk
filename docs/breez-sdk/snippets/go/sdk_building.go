package example

import (
	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func InitSdkAdvanced() (*breez_sdk_spark.BreezSdk, error) {
	// ANCHOR: init-sdk-advanced
	// Construct the seed using mnemonic words or entropy bytes
	mnemonic := "<mnemonic words>"
	var seed breez_sdk_spark.Seed = breez_sdk_spark.SeedMnemonic{
		Mnemonic:   mnemonic,
		Passphrase: nil,
	}

	// Create the default config
	apiKey := "<breez api key>"
	config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
	config.ApiKey = &apiKey

	// Build the SDK using the config, seed and default storage
	builder := breez_sdk_spark.NewSdkBuilder(config, seed)
	builder.WithDefaultStorage("./.data")
	// You can also pass your custom implementations:
	// builder.WithStorage(<your storage implementation>)
	// builder.WithRealTimeSyncStorage(<your real-time sync storage implementation>)
	// builder.WithChainService(<your chain service implementation>)
	// builder.WithRestClient(<your rest client implementation>)
	// builder.WithKeySet(<your key set type>, <use address index>, <account number>)
	// builder.WithPaymentObserver(<your payment observer implementation>)
	sdk, err := builder.Build()

	return sdk, err
	// ANCHOR_END: init-sdk-advanced
}

func WithRestChainService(builder *breez_sdk_spark.SdkBuilder) {
	// ANCHOR: with-rest-chain-service
	url := "<your REST chain service URL>"
	chainApiType := breez_sdk_spark.ChainApiTypeMempoolSpace
	optionalCredentials := &breez_sdk_spark.Credentials{
		Username: "<username>",
		Password: "<password>",
	}
	builder.WithRestChainService(url, chainApiType, optionalCredentials)
	// ANCHOR_END: with-rest-chain-service
}

func WithKeySet(builder *breez_sdk_spark.SdkBuilder) {
	// ANCHOR: with-key-set
	keySetType := breez_sdk_spark.KeySetTypeDefault
	useAccountIndex := true
	optionalAccountNumber := uint32(21)
	builder.WithKeySet(keySetType, useAccountIndex, &optionalAccountNumber)
	// ANCHOR_END: with-key-set
}

// ANCHOR: with-storage
type Storage interface {
	DeleteCachedItem(key string) error
	GetCachedItem(key string) (*string, error)
	SetCachedItem(key string, value string) error
	ListPayments(request breez_sdk_spark.ListPaymentsRequest) ([]breez_sdk_spark.Payment, error)
	InsertPayment(payment breez_sdk_spark.Payment) error
	SetPaymentMetadata(paymentId string, metadata breez_sdk_spark.PaymentMetadata) error
	GetPaymentById(id string) (breez_sdk_spark.Payment, error)
	GetPaymentByInvoice(invoice string) (*breez_sdk_spark.Payment, error)
	AddDeposit(txid string, vout uint32, amountSats uint64) error
	DeleteDeposit(txid string, vout uint32) error
	ListDeposits() ([]breez_sdk_spark.DepositInfo, error)
	UpdateDeposit(txid string, vout uint32, payload breez_sdk_spark.UpdateDepositPayload) error
}
// ANCHOR_END: with-storage

// ANCHOR: with-sync-storage
type SyncStorage interface {
	AddOutgoingChange(record breez_sdk_spark.UnversionedRecordChange) (uint64, error)
	CompleteOutgoingSync(record breez_sdk_spark.Record) error
	GetPendingOutgoingChanges(limit uint32) ([]breez_sdk_spark.OutgoingChange, error)
	GetLastRevision() (uint64, error)
	InsertIncomingRecords(records []breez_sdk_spark.Record) error
	DeleteIncomingRecord(record breez_sdk_spark.Record) error
	RebasePendingOutgoingRecords(revision uint64) error
	GetIncomingRecords(limit uint32) ([]breez_sdk_spark.IncomingChange, error)
	GetLatestOutgoingChange() (*breez_sdk_spark.OutgoingChange, error)
	UpdateRecordFromIncoming(record breez_sdk_spark.Record) error
}
// ANCHOR_END: with-sync-storage

// ANCHOR: with-chain-service
type BitcoinChainService interface {
	GetAddressUtxos(address string) ([]breez_sdk_spark.Utxo, error)
	GetTransactionStatus(txid string) (breez_sdk_spark.TxStatus, error)
	GetTransactionHex(txid string) (string, error)
	BroadcastTransaction(tx string) error
}
// ANCHOR_END: with-chain-service

// ANCHOR: with-rest-client
type RestClient interface {
	GetRequest(url string, headers *map[string]string) (breez_sdk_spark.RestResponse, error)
	PostRequest(url string, headers *map[string]string, body *string) (breez_sdk_spark.RestResponse, error)
	DeleteRequest(url string, headers *map[string]string, body *string) (breez_sdk_spark.RestResponse, error)
}
// ANCHOR_END: with-rest-client

// ANCHOR: with-fiat-service
type FiatService interface {
	FetchFiatCurrencies() ([]breez_sdk_spark.FiatCurrency, error)
	FetchFiatRates() ([]breez_sdk_spark.Rate, error)
}
// ANCHOR_END: with-fiat-service

// ANCHOR: with-payment-observer
type PaymentObserver interface {
	BeforeSend(payments []breez_sdk_spark.ProvisionalPayment) error
}
// ANCHOR_END: with-payment-observer
