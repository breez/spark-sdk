package example

import (
	"log"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func InitSdk() (*breez_sdk_spark.BreezSdk, error) {
	// ANCHOR: init-sdk
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

	connectRequest := breez_sdk_spark.ConnectRequest{
		Config:     config,
		Seed:       seed,
		StorageDir: "./.data",
	}

	// Connect to the SDK using the simplified connect method
	sdk, err := breez_sdk_spark.Connect(connectRequest)

	return sdk, err
	// ANCHOR_END: init-sdk
}

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

	storage, err := breez_sdk_spark.DefaultStorage("./.data")
	if err != nil {
		return nil, err
	}

	builder := breez_sdk_spark.NewSdkBuilder(config, seed, storage)
	// You can also pass your custom implementations:
	// builder.WithChainService(<your chain service implementation>)
	// builder.WithRestClient(<your rest client implementation>)
	// builder.WithKeySet(<your key set type>, <use address index>, <account number>)
	sdk, err := builder.Build()

	return sdk, err
	// ANCHOR_END: init-sdk-advanced
}

func FetchBalance(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: fetch-balance
	info, err := sdk.GetInfo(breez_sdk_spark.GetInfoRequest{})

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return err
	}

	balanceSats := info.BalanceSats
	log.Printf("Balance: %v sats", balanceSats)
	// ANCHOR_END: fetch-balance
}

// ANCHOR: logging
type SdkLogger struct{}

func (SdkLogger) Log(l breez_sdk_spark.LogEntry) {
	log.Printf("Received log [%v]: %v", l.Level, l.Line)
}

func SetLogger() {
	var loggerImpl breez_sdk_spark.Logger = SdkLogger{}
	breez_sdk_spark.InitLogging(nil, &loggerImpl, nil)
}

// ANCHOR_END: logging

// ANCHOR: add-event-listener
type SdkListener struct{}

func (SdkListener) OnEvent(e breez_sdk_spark.SdkEvent) {
	log.Printf("Received event %#v", e)
}

func AddEventListener(sdk *breez_sdk_spark.BreezSdk, listener SdkListener) string {
	return sdk.AddEventListener(listener)
}

// ANCHOR_END: add-event-listener

// ANCHOR: remove-event-listener
func RemoveEventListener(sdk *breez_sdk_spark.BreezSdk, listenerId string) bool {
	return sdk.RemoveEventListener(listenerId)
}

// ANCHOR_END: remove-event-listener

// ANCHOR: disconnect
func Disconnect(sdk *breez_sdk_spark.BreezSdk) {
	sdk.Disconnect()
}

// ANCHOR_END: disconnect
