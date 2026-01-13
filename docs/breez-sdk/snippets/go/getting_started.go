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

func FetchBalance(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: fetch-balance
	ensureSynced := false
	info, err := sdk.GetInfo(breez_sdk_spark.GetInfoRequest{
		// EnsureSynced: true will ensure the SDK is synced with the Spark network
		// before returning the balance
		EnsureSynced: &ensureSynced,
	})

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return err
	}

	balanceSats := info.BalanceSats
	log.Printf("Balance: %v sats", balanceSats)
	// ANCHOR_END: fetch-balance
	return nil
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
	switch event := e.(type) {
	case breez_sdk_spark.SdkEventSynced:
		// Data has been synchronized with the network. When this event is received,
		// it is recommended to refresh the payment list and wallet balance.
	case breez_sdk_spark.SdkEventUnclaimedDeposits:
		// SDK was unable to claim some deposits automatically
		unclaimedDeposits := event.UnclaimedDeposits
		_ = unclaimedDeposits
	case breez_sdk_spark.SdkEventClaimedDeposits:
		// Deposits were successfully claimed
		claimedDeposits := event.ClaimedDeposits
		_ = claimedDeposits
	case breez_sdk_spark.SdkEventPaymentSucceeded:
		// A payment completed successfully
		payment := event.Payment
		_ = payment
	case breez_sdk_spark.SdkEventPaymentPending:
		// A payment is pending (waiting for confirmation)
		pendingPayment := event.Payment
		_ = pendingPayment
	case breez_sdk_spark.SdkEventPaymentFailed:
		// A payment failed
		failedPayment := event.Payment
		_ = failedPayment
	case breez_sdk_spark.SdkEventOptimization:
		// An optimization event occurred
		optimizationEvent := event.OptimizationEvent
		_ = optimizationEvent
	default:
		// Handle any future event types
	}
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
