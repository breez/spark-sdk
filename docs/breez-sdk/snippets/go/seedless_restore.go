package example

import (
	"log"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

// ANCHOR: implement-prf-provider
// In practice, implement PRF provider using platform passkey APIs
type ExamplePasskeyPrfProvider struct{}

func (p *ExamplePasskeyPrfProvider) DerivePrfSeed(salt string) ([]byte, error) {
	// Call platform passkey API with PRF extension
	// Returns 32-byte PRF output
	panic("Implement using platform passkey APIs")
}

func (p *ExamplePasskeyPrfProvider) IsPrfAvailable() (bool, error) {
	// Check if PRF-capable passkey exists
	panic("Check platform passkey availability")
}

// ANCHOR_END: implement-prf-provider

func CreateSeed() (*breez_sdk_spark.BreezSdk, error) {
	// ANCHOR: create-seed
	prfProvider := &ExamplePasskeyPrfProvider{}
	seedless := breez_sdk_spark.NewSeedlessRestore(prfProvider, nil)

	// Create a new seed with user-chosen salt
	// The salt is published to Nostr for later discovery
	seed, err := seedless.CreateSeed("personal")
	if err != nil {
		return nil, err
	}

	// Use the seed to initialize the SDK
	config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
	builder := breez_sdk_spark.NewSdkBuilder(config, seed)
	builder.WithDefaultStorage("./.data")
	sdk, err := builder.Build()
	if err != nil {
		return nil, err
	}
	// ANCHOR_END: create-seed
	return sdk, nil
}

func ListSalts() ([]string, error) {
	// ANCHOR: list-salts
	prfProvider := &ExamplePasskeyPrfProvider{}
	seedless := breez_sdk_spark.NewSeedlessRestore(prfProvider, nil)

	// Query Nostr for salts associated with this passkey
	salts, err := seedless.ListSalts()
	if err != nil {
		return nil, err
	}

	for _, salt := range salts {
		log.Printf("Found wallet: %s", salt)
	}
	// ANCHOR_END: list-salts
	return salts, nil
}

func RestoreSeed() (*breez_sdk_spark.BreezSdk, error) {
	// ANCHOR: restore-seed
	prfProvider := &ExamplePasskeyPrfProvider{}
	seedless := breez_sdk_spark.NewSeedlessRestore(prfProvider, nil)

	// Restore seed using a known salt
	seed, err := seedless.RestoreSeed("personal")
	if err != nil {
		return nil, err
	}

	// Use the seed to initialize the SDK
	config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
	builder := breez_sdk_spark.NewSdkBuilder(config, seed)
	builder.WithDefaultStorage("./.data")
	sdk, err := builder.Build()
	if err != nil {
		return nil, err
	}
	// ANCHOR_END: restore-seed
	return sdk, nil
}
