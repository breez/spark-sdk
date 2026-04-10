package example

import (
	"log"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

// ANCHOR: implement-prf-provider
// Implement using platform-specific passkey APIs if the SDK does not ship a built-in provider for your target.
type CustomPasskeyPrfProvider struct{}

func (p *CustomPasskeyPrfProvider) DerivePrfSeed(salt string) ([]byte, error) {
	// Call platform passkey API with PRF extension
	// Returns 32-byte PRF output
	panic("Implement using WebAuthn or native passkey APIs")
}

func (p *CustomPasskeyPrfProvider) IsPrfAvailable() (bool, error) {
	// Check if PRF-capable passkey exists
	panic("Check platform passkey availability")
}

// ANCHOR_END: implement-prf-provider

func CheckAvailability() {
	// ANCHOR: check-availability
	prfProvider := &CustomPasskeyPrfProvider{}

	available, err := prfProvider.IsPrfAvailable()
	if err == nil && available {
		// Show passkey as primary option
	} else {
		// Fall back to mnemonic flow
	}
	// ANCHOR_END: check-availability
}

func ConnectWithPasskey() (*breez_sdk_spark.BreezSdk, error) {
	// ANCHOR: connect-with-passkey
	prfProvider := &CustomPasskeyPrfProvider{}
	passkey := breez_sdk_spark.NewPasskey(prfProvider, nil)

	// Derive the wallet from the passkey (pass nil for the default wallet)
	label := "personal"
	wallet, err := passkey.GetWallet(&label)
	if err != nil {
		return nil, err
	}

	config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
	sdk, err := breez_sdk_spark.Connect(breez_sdk_spark.ConnectRequest{
		Config:     config,
		Seed:       wallet.Seed,
		StorageDir: "./.data",
	})
	if err != nil {
		return nil, err
	}
	// ANCHOR_END: connect-with-passkey
	return sdk, nil
}

func ListLabels() ([]string, error) {
	// ANCHOR: list-labels
	prfProvider := &CustomPasskeyPrfProvider{}
	breezApiKey := "<breez api key>"
	relayConfig := &breez_sdk_spark.NostrRelayConfig{
		BreezApiKey: &breezApiKey,
	}
	passkey := breez_sdk_spark.NewPasskey(prfProvider, relayConfig)

	// Query Nostr for labels associated with this passkey
	labels, err := passkey.ListLabels()
	if err != nil {
		return nil, err
	}

	for _, label := range labels {
		log.Printf("Found label: %s", label)
	}
	// ANCHOR_END: list-labels
	return labels, nil
}

func StoreLabel() error {
	// ANCHOR: store-label
	prfProvider := &CustomPasskeyPrfProvider{}
	breezApiKey := "<breez api key>"
	relayConfig := &breez_sdk_spark.NostrRelayConfig{
		BreezApiKey: &breezApiKey,
	}
	passkey := breez_sdk_spark.NewPasskey(prfProvider, relayConfig)

	// Publish the label to Nostr for later discovery
	err := passkey.StoreLabel("personal")
	if err != nil {
		return err
	}
	// ANCHOR_END: store-label
	return nil
}
