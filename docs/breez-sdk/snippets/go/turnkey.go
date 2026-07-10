package example

import (
	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func ConnectWithTurnkey() (*breez_sdk_spark.BreezSdk, error) {
	// ANCHOR: turnkey-connect
	turnkeyConfig := breez_sdk_spark.TurnkeyConfig{
		BaseUrl:        nil,
		OrganizationId: "<turnkey sub-organization id>",
		ApiPublicKey:   "<api public key hex>",
		ApiPrivateKey:  "<api private key hex>",
		WalletId:       "<turnkey wallet id>",
		Network:        breez_sdk_spark.NetworkMainnet,
		AccountNumber:  nil,
		// Set after the first connect to make later signer setup network-free
		IdentityPublicKey: nil,
		Retry:             nil,
		MaxRps:            nil,
	}

	signers, err := breez_sdk_spark.CreateTurnkeySigner(turnkeyConfig)
	if err != nil {
		return nil, err
	}

	config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
	apiKey := "<breez api key>"
	config.ApiKey = &apiKey

	sdk, err := breez_sdk_spark.ConnectWithSigner(breez_sdk_spark.ConnectWithSignerRequest{
		Config:      config,
		BreezSigner: signers.BreezSigner,
		SparkSigner: signers.SparkSigner,
		StorageDir:  "./.data",
	})
	if err != nil {
		return nil, err
	}
	// ANCHOR_END: turnkey-connect
	return sdk, nil
}
