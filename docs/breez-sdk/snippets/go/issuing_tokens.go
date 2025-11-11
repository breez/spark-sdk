package example

import (
	"math/big"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func GetIssuerSdk(sdk *breez_sdk_spark.BreezSdk) *breez_sdk_spark.BreezIssuerSdk {
	// ANCHOR: get-issuer-sdk
	issuerSdk := sdk.GetIssuerSdk()
	// ANCHOR_END: get-issuer-sdk
	return issuerSdk
}

func CreateToken(issuerSdk *breez_sdk_spark.BreezIssuerSdk) (*breez_sdk_spark.TokenMetadata, error) {
	// ANCHOR: create-token
	request := breez_sdk_spark.CreateIssuerTokenRequest{
		Name:        "My Token",
		Ticker:      "MTK",
		Decimals:    6,
		IsFreezable: false,
		MaxSupply:   new(big.Int).SetInt64(1_000_000),
	}
	tokenMetadata, err := issuerSdk.CreateIssuerToken(request)
	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}
	println("Token identifier: ", tokenMetadata.Identifier)
	// ANCHOR_END: create-token
	return &tokenMetadata, nil
}

func MintToken(issuerSdk *breez_sdk_spark.BreezIssuerSdk) (*breez_sdk_spark.Payment, error) {
	// ANCHOR: mint-token
	request := breez_sdk_spark.MintIssuerTokenRequest{
		Amount: new(big.Int).SetInt64(1_000),
	}
	payment, err := issuerSdk.MintIssuerToken(request)
	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}
	// ANCHOR_END: mint-token
	return &payment, nil
}

func BurnToken(issuerSdk *breez_sdk_spark.BreezIssuerSdk) (*breez_sdk_spark.Payment, error) {
	// ANCHOR: burn-token
	request := breez_sdk_spark.BurnIssuerTokenRequest{
		Amount: new(big.Int).SetInt64(1_000),
	}
	payment, err := issuerSdk.BurnIssuerToken(request)
	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}
	// ANCHOR_END: burn-token
	return &payment, nil
}

func GetTokenMetadata(issuerSdk *breez_sdk_spark.BreezIssuerSdk) (*breez_sdk_spark.TokenMetadata, error) {
	// ANCHOR: get-token-metadata
	tokenBalance, err := issuerSdk.GetIssuerTokenBalance()
	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}
	println("Token balance: ", tokenBalance.Balance)

	tokenMetadata, err := issuerSdk.GetIssuerTokenMetadata()
	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}
	println("Token ticker: ", tokenMetadata.Ticker)
	// ANCHOR_END: get-token-metadata
	return &tokenMetadata, nil
}

func FreezeToken(issuerSdk *breez_sdk_spark.BreezIssuerSdk) error {
	// ANCHOR: freeze-token
	sparkAddress := "<spark address>"
	// Freeze the tokens held at the specified Spark address
	freezeRequest := breez_sdk_spark.FreezeIssuerTokenRequest{
		Address: sparkAddress,
	}
	freezeResponse, err := issuerSdk.FreezeIssuerToken(freezeRequest)
	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return err
	}

	// Unfreeze the tokens held at the specified Spark address
	unfreezeRequest := breez_sdk_spark.UnfreezeIssuerTokenRequest{
		Address: sparkAddress,
	}
	unfreezeResponse, err := issuerSdk.UnfreezeIssuerToken(unfreezeRequest)
	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return err
	}
	// ANCHOR_END: freeze-token
	println("Freeze response: ", freezeResponse)
	println("Unfreeze response: ", unfreezeResponse)
	return nil
}
