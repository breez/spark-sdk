package example

import (
	"log"
	"math/big"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func GetTokenIssuer(sdk *breez_sdk_spark.BreezSdk) *breez_sdk_spark.TokenIssuer {
	// ANCHOR: get-token-issuer
	tokenIssuer := sdk.GetTokenIssuer()
	// ANCHOR_END: get-token-issuer
	return tokenIssuer
}

func CreateToken(tokenIssuer *breez_sdk_spark.TokenIssuer) (*breez_sdk_spark.TokenMetadata, error) {
	// ANCHOR: create-token
	request := breez_sdk_spark.CreateIssuerTokenRequest{
		Name:        "My Token",
		Ticker:      "MTK",
		Decimals:    6,
		IsFreezable: false,
		MaxSupply:   new(big.Int).SetInt64(1_000_000),
	}
	tokenMetadata, err := tokenIssuer.CreateIssuerToken(request)
	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}
	log.Printf("Token identifier: %v", tokenMetadata.Identifier)
	// ANCHOR_END: create-token
	return &tokenMetadata, nil
}

func CreateTokenWithCustomAccountNumber() (*breez_sdk_spark.BreezSdk, error) {
	// ANCHOR: custom-account-number
	accountNumber := uint32(21)

	mnemonic := "<mnemonic words>"
	var seed breez_sdk_spark.Seed = breez_sdk_spark.SeedMnemonic{
		Mnemonic:   mnemonic,
		Passphrase: nil,
	}
	apiKey := "<breez api key>"
	config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
	config.ApiKey = &apiKey
	builder := breez_sdk_spark.NewSdkBuilder(config, seed)
	builder.WithDefaultStorage("./.data")

	// Set the account number for the SDK
	builder.WithKeySet(breez_sdk_spark.KeySetTypeDefault, false, &accountNumber)

	sdk, err := builder.Build()
	// ANCHOR_END: custom-account-number
	return sdk, err
}

func MintToken(tokenIssuer *breez_sdk_spark.TokenIssuer) (*breez_sdk_spark.Payment, error) {
	// ANCHOR: mint-token
	request := breez_sdk_spark.MintIssuerTokenRequest{
		Amount: new(big.Int).SetInt64(1_000),
	}
	payment, err := tokenIssuer.MintIssuerToken(request)
	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}
	// ANCHOR_END: mint-token
	return &payment, nil
}

func BurnToken(tokenIssuer *breez_sdk_spark.TokenIssuer) (*breez_sdk_spark.Payment, error) {
	// ANCHOR: burn-token
	request := breez_sdk_spark.BurnIssuerTokenRequest{
		Amount: new(big.Int).SetInt64(1_000),
	}
	payment, err := tokenIssuer.BurnIssuerToken(request)
	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}
	// ANCHOR_END: burn-token
	return &payment, nil
}

func GetTokenMetadata(tokenIssuer *breez_sdk_spark.TokenIssuer) (*breez_sdk_spark.TokenMetadata, error) {
	// ANCHOR: get-token-metadata
	tokenBalance, err := tokenIssuer.GetIssuerTokenBalance()
	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}
	log.Printf("Token balance: %v", tokenBalance.Balance)

	tokenMetadata, err := tokenIssuer.GetIssuerTokenMetadata()
	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}
	log.Printf("Token ticker: %v", tokenMetadata.Ticker)
	// ANCHOR_END: get-token-metadata
	return &tokenMetadata, nil
}

func FreezeToken(tokenIssuer *breez_sdk_spark.TokenIssuer) error {
	// ANCHOR: freeze-token
	sparkAddress := "<spark address>"
	// Freeze the tokens held at the specified Spark address
	freezeRequest := breez_sdk_spark.FreezeIssuerTokenRequest{
		Address: sparkAddress,
	}
	freezeResponse, err := tokenIssuer.FreezeIssuerToken(freezeRequest)
	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return err
	}

	// Unfreeze the tokens held at the specified Spark address
	unfreezeRequest := breez_sdk_spark.UnfreezeIssuerTokenRequest{
		Address: sparkAddress,
	}
	unfreezeResponse, err := tokenIssuer.UnfreezeIssuerToken(unfreezeRequest)
	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return err
	}
	// ANCHOR_END: freeze-token
	log.Printf("Freeze response: %v", freezeResponse)
	log.Printf("Unfreeze response: %v", unfreezeResponse)
	return nil
}
