package example

import (
	"log"

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

	keySetConfig := breez_sdk_spark.KeySetConfig{
		KeySetType:      keySetType,
		UseAddressIndex: useAccountIndex,
		AccountNumber:   &optionalAccountNumber,
	}

	builder.WithKeySet(keySetConfig)
	// ANCHOR_END: with-key-set
}

// ANCHOR: with-payment-observer
type ExamplePaymentObserver struct{}

func (ExamplePaymentObserver) BeforeSend(payments []breez_sdk_spark.ProvisionalPayment) error {
	for _, payment := range payments {
		log.Printf("About to send payment: %v of amount %v", payment.PaymentId, payment.Amount)
	}
	return nil
}

func WithPaymentObserver(builder *breez_sdk_spark.SdkBuilder) {
	observer := ExamplePaymentObserver{}
	builder.WithPaymentObserver(observer)
}

// ANCHOR_END: with-payment-observer

func InitSdkPostgres() (*breez_sdk_spark.BreezSdk, error) {
	// ANCHOR: init-sdk-postgres
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

	// Configure PostgreSQL storage
	// Connection string format: "host=localhost user=postgres password=secret dbname=spark"
	// Or URI format: "postgres://user:password@host:port/dbname?sslmode=require"
	postgresConfig := breez_sdk_spark.DefaultPostgresStorageConfig("host=localhost user=postgres dbname=spark")
	// Optionally pool settings can be adjusted. Some examples:
	postgresConfig.MaxPoolSize = 8 // Max connections in pool
	waitTimeoutSecs := uint64(30)
	postgresConfig.WaitTimeoutSecs = &waitTimeoutSecs // Timeout waiting for connection

	// Build the SDK with PostgreSQL storage
	builder := breez_sdk_spark.NewSdkBuilder(config, seed)
	builder.WithPostgresStorage(postgresConfig)
	sdk, err := builder.Build()
	if err != nil {
		return nil, err
	}
	// ANCHOR_END: init-sdk-postgres

	return sdk, nil
}
