package example

import (
	"errors"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

// ANCHOR: default-external-signer
func createSigners() (breez_sdk_spark.ExternalSigners, error) {
	mnemonic := "<mnemonic words>"
	network := breez_sdk_spark.NetworkMainnet
	var accountNumber uint32 = 0

	signers, err := breez_sdk_spark.DefaultExternalSigners(
		mnemonic,
		nil, // passphrase
		network,
		&accountNumber,
	)
	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
		return breez_sdk_spark.ExternalSigners{}, err
	}

	return signers, nil
}

// ANCHOR_END: default-external-signer

// ANCHOR: connect-with-signer
func connectWithSigner(
	signers breez_sdk_spark.ExternalSigners,
) (*breez_sdk_spark.BreezSdk, error) {
	// Create the config
	config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
	apiKey := "<breez api key>"
	config.ApiKey = &apiKey

	// Connect using the external signers
	sdk, err := breez_sdk_spark.ConnectWithSigner(breez_sdk_spark.ConnectWithSignerRequest{
		Config:      config,
		BreezSigner: signers.BreezSigner,
		SparkSigner: signers.SparkSigner,
		StorageDir:  "./.data",
	})
	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
		return nil, err
	}

	return sdk, nil
}

// ANCHOR_END: connect-with-signer
