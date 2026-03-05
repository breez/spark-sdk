package example

import (
	"log"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

// ANCHOR: implement-prf-provider
// In practice, implement using platform-specific passkey APIs.
type ExamplePasskeyPrfProvider struct{}

func (p *ExamplePasskeyPrfProvider) DerivePrfSeed(salt string) ([]byte, error) {
	// Call platform passkey API with PRF extension
	// Returns 32-byte PRF output
	panic("Implement using WebAuthn or native passkey APIs")
}

func (p *ExamplePasskeyPrfProvider) IsPrfAvailable() (bool, error) {
	// Check if PRF-capable passkey exists
	panic("Check platform passkey availability")
}

// ANCHOR_END: implement-prf-provider

func ConnectWithPasskey() (*breez_sdk_spark.BreezSdk, error) {
	// ANCHOR: connect-with-passkey
	prfProvider := &ExamplePasskeyPrfProvider{}
	passkey := breez_sdk_spark.NewPasskey(prfProvider, nil)

	// Derive the wallet from the passkey (pass nil for the default wallet)
	walletName := "personal"
	wallet, err := passkey.GetWallet(&walletName)
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

func ListWalletNames() ([]string, error) {
	// ANCHOR: list-wallet-names
	prfProvider := &ExamplePasskeyPrfProvider{}
	breezApiKey := "<breez api key>"
	relayConfig := &breez_sdk_spark.NostrRelayConfig{
		BreezApiKey: &breezApiKey,
	}
	passkey := breez_sdk_spark.NewPasskey(prfProvider, relayConfig)

	// Query Nostr for wallet names associated with this passkey
	walletNames, err := passkey.ListWalletNames()
	if err != nil {
		return nil, err
	}

	for _, walletName := range walletNames {
		log.Printf("Found wallet: %s", walletName)
	}
	// ANCHOR_END: list-wallet-names
	return walletNames, nil
}

func StoreWalletName() error {
	// ANCHOR: store-wallet-name
	prfProvider := &ExamplePasskeyPrfProvider{}
	breezApiKey := "<breez api key>"
	relayConfig := &breez_sdk_spark.NostrRelayConfig{
		BreezApiKey: &breezApiKey,
	}
	passkey := breez_sdk_spark.NewPasskey(prfProvider, relayConfig)

	// Publish the wallet name to Nostr for later discovery
	err := passkey.StoreWalletName("personal")
	if err != nil {
		return err
	}
	// ANCHOR_END: store-wallet-name
	return nil
}
