package example

import (
	"log"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func InitSdkAdvanced() (*breez_sdk_spark.BreezSdk, error) {
	// ANCHOR: init-sdk-advanced
	// Construct the seed using a mnemonic, entropy or passkey
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
	// Construct the seed using a mnemonic, entropy or passkey
	mnemonic := "<mnemonic words>"
	var seed breez_sdk_spark.Seed = breez_sdk_spark.SeedMnemonic{
		Mnemonic:   mnemonic,
		Passphrase: nil,
	}

	// Create the default config
	apiKey := "<breez api key>"
	config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
	config.ApiKey = &apiKey

	// Configure PostgreSQL backend
	// Connection string format: "host=localhost user=postgres password=secret dbname=spark"
	// Or URI format: "postgres://user:password@host:port/dbname?sslmode=require"
	postgresConfig := breez_sdk_spark.DefaultPostgresStorageConfig("host=localhost user=postgres dbname=spark")
	// Optionally pool settings can be adjusted. Some examples:
	postgresConfig.MaxPoolSize = 8 // Max connections in pool
	waitTimeoutSecs := uint64(30)
	postgresConfig.WaitTimeoutSecs = &waitTimeoutSecs // Timeout waiting for connection
	// If your service owns SDK-compatible schema migrations:
	postgresConfig.RunMigration = false

	// Build the SDK with the PostgreSQL storage backend (storage, tree store,
	// and token store). Per-tenant scoping (rows isolated by seed identity)
	// is applied automatically.
	builder := breez_sdk_spark.NewSdkBuilder(config, seed)
	builder.WithStorageBackend(breez_sdk_spark.PostgresStorage(postgresConfig))
	sdk, err := builder.Build()
	if err != nil {
		return nil, err
	}
	// ANCHOR_END: init-sdk-postgres

	return sdk, nil
}

func InitSdkMysql() (*breez_sdk_spark.BreezSdk, error) {
	// ANCHOR: init-sdk-mysql
	// Construct the seed using a mnemonic, entropy or passkey
	mnemonic := "<mnemonic words>"
	var seed breez_sdk_spark.Seed = breez_sdk_spark.SeedMnemonic{
		Mnemonic:   mnemonic,
		Passphrase: nil,
	}

	// Create the default config
	apiKey := "<breez api key>"
	config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
	config.ApiKey = &apiKey

	// Configure MySQL backend (MySQL 8.0+).
	// Connection string format (URL only):
	//   "mysql://user:password@host:3306/dbname?ssl-mode=required"
	mysqlConfig := breez_sdk_spark.DefaultMysqlStorageConfig("mysql://user:password@localhost:3306/spark")
	// Optionally pool settings can be adjusted. Some examples:
	mysqlConfig.MaxPoolSize = 8 // Max connections in pool
	recycleTimeoutSecs := uint64(60)
	mysqlConfig.RecycleTimeoutSecs = &recycleTimeoutSecs // Recycle idle connections after this many seconds
	// Provide a custom CA certificate when using ssl-mode=verify_ca or verify_identity:
	// rootCa := "-----BEGIN CERTIFICATE-----\n..."
	// mysqlConfig.RootCaPem = &rootCa

	// Build the SDK with the MySQL storage backend (storage, tree store, and
	// token store). Per-tenant scoping (rows isolated by seed identity) is
	// applied automatically.
	builder := breez_sdk_spark.NewSdkBuilder(config, seed)
	builder.WithStorageBackend(breez_sdk_spark.MysqlStorage(mysqlConfig))
	sdk, err := builder.Build()
	if err != nil {
		return nil, err
	}
	// ANCHOR_END: init-sdk-mysql

	return sdk, nil
}

func InitSdkServer() (*breez_sdk_spark.BreezSdk, error) {
	// ANCHOR: init-sdk-server
	// Construct the seed using a mnemonic, entropy or passkey
	mnemonic := "<mnemonic words>"
	var seed breez_sdk_spark.Seed = breez_sdk_spark.SeedMnemonic{
		Mnemonic:   mnemonic,
		Passphrase: nil,
	}

	// Build a server-mode config: same as DefaultConfig(network) with
	// BackgroundTasksEnabled = false. No periodic sync, no real-time sync
	// client, no leaf/token optimizer, no flashnet refunder, no lightning-
	// address recovery, no spark private-mode init.
	apiKey := "<breez api key>"
	config := breez_sdk_spark.DefaultServerConfig(breez_sdk_spark.NetworkMainnet)
	config.ApiKey = &apiKey

	// Typically server-mode SDKs are built per request and share infrastructure
	// (DB pool, REST chain service, SSP/Connection Manager) across instances.
	// Pass the shared resources via the builder.
	builder := breez_sdk_spark.NewSdkBuilder(config, seed)
	builder.WithDefaultStorage("./.data")
	sdk, err := builder.Build()
	if err != nil {
		return nil, err
	}
	// ANCHOR_END: init-sdk-server

	return sdk, nil
}

func ServerModeRequestHandler(sdk *breez_sdk_spark.BreezSdk) (string, error) {
	// ANCHOR: server-mode-request-handler
	// User-facing request handler: do not call SyncWallet here. Operations
	// that read from local storage (GetInfo, ListPayments, etc.) do not need
	// a defensive sync. Call SyncWallet only from webhook handlers or
	// reconciliation jobs that need to observe an external state change.
	amountSats := uint64(5_000)
	expirySecs := uint32(3600)
	response, err := sdk.ReceivePayment(breez_sdk_spark.ReceivePaymentRequest{
		PaymentMethod: breez_sdk_spark.ReceivePaymentMethodBolt11Invoice{
			Description: "<invoice description>",
			AmountSats:  &amountSats,
			ExpirySecs:  &expirySecs,
			PaymentHash: nil,
		},
	})
	if err != nil {
		return "", err
	}

	// Always disconnect at the end of the request lifecycle to flush
	// outstanding storage writes.
	if err := sdk.Disconnect(); err != nil {
		return "", err
	}
	// ANCHOR_END: server-mode-request-handler
	return response.PaymentRequest, nil
}

func ServerModeProvisioning(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: server-mode-provisioning
	// One-time setup when a wallet is first registered. The client-mode SDK
	// would normally apply the private-mode preset itself on first startup;
	// server-mode SDKs do not, so opt in once here via UpdateUserSettings.
	sparkPrivateModeEnabled := true
	if err := sdk.UpdateUserSettings(breez_sdk_spark.UpdateUserSettingsRequest{
		SparkPrivateModeEnabled: &sparkPrivateModeEnabled,
	}); err != nil {
		return err
	}

	return sdk.Disconnect()
	// ANCHOR_END: server-mode-provisioning
}

func RefundPendingConversions(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: refund-pending-conversions
	// The flashnet conversion refunder doesn't run in the background in server
	// mode. Call this from your own scheduler (e.g. once per minute) to issue
	// pending refunds for failed conversions.
	return sdk.RefundPendingConversions()
	// ANCHOR_END: refund-pending-conversions
}
