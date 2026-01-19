package example

import (
	"errors"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

// ANCHOR: default-external-signer
func createSigner() (breez_sdk_spark.ExternalSigner, error) {
	mnemonic := "<mnemonic words>"
	network := breez_sdk_spark.NetworkMainnet
	keySetType := breez_sdk_spark.KeySetTypeDefault
	useAddressIndex := false
	var accountNumber uint32 = 0

	keySetConfig := breez_sdk_spark.KeySetConfig{
		KeySetType:      keySetType,
		UseAddressIndex: useAddressIndex,
		AccountNumber:   &accountNumber,
	}

	signer, err := breez_sdk_spark.DefaultExternalSigner(
		mnemonic,
		nil, // passphrase
		network,
		&keySetConfig,
	)
	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
		return nil, err
	}

	return signer, nil
}

// ANCHOR_END: default-external-signer

// ANCHOR: connect-with-signer
func connectWithSigner(signer breez_sdk_spark.ExternalSigner) (*breez_sdk_spark.BreezSdk, error) {
	// Create the config
	config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
	apiKey := "<breez api key>"
	config.ApiKey = &apiKey

	// Connect using the external signer
	sdk, err := breez_sdk_spark.ConnectWithSigner(breez_sdk_spark.ConnectWithSignerRequest{
		Config:     config,
		Signer:     signer,
		StorageDir: "./.data",
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
