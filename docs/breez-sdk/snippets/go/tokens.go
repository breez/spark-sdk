package example

import (
	"log"
	"math/big"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func FetchTokenBalances(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: fetch-token-balances
	ensureSynced := false
	info, err := sdk.GetInfo(breez_sdk_spark.GetInfoRequest{
		// EnsureSynced: true will ensure the SDK is synced with the Spark network
		// before returning the balance
		EnsureSynced: &ensureSynced,
	})

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return err
	}

	// Token balances are a map of token identifier to balance
	tokenBalances := info.TokenBalances
	for tokenId, tokenBalance := range tokenBalances {
		log.Printf("Token ID: %v", tokenId)
		log.Printf("Balance: %v", tokenBalance.Balance)
		log.Printf("Name: %v", tokenBalance.TokenMetadata.Name)
		log.Printf("Ticker: %v", tokenBalance.TokenMetadata.Ticker)
		log.Printf("Decimals: %v", tokenBalance.TokenMetadata.Decimals)
	}
	// ANCHOR_END: fetch-token-balances
	return nil
}

func SendTokenPayment(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: send-token-payment
	paymentRequest := "<spark address>"
	tokenIdentifier := "<token identifier>"
	// Set the amount of tokens you wish to send
	amount := new(big.Int).SetInt64(1_000)

	prepareResponse, err := sdk.PrepareSendPayment(breez_sdk_spark.PrepareSendPaymentRequest{
		PaymentRequest:  paymentRequest,
		Amount:          &amount,
		TokenIdentifier: &tokenIdentifier,
	})

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return err
	}

	// If the fees are acceptable, continue to send the token payment
	switch method := prepareResponse.PaymentMethod.(type) {
	case breez_sdk_spark.SendPaymentMethodSparkAddress:
		log.Printf("Token ID: %v", method.TokenIdentifier)
		log.Printf("Fees: %v sats", method.Fee)
	}

	// Send the token payment
	sendResponse, err := sdk.SendPayment(breez_sdk_spark.SendPaymentRequest{
		PrepareResponse: prepareResponse,
		Options:         nil,
	})

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return err
	}

	payment := sendResponse.Payment
	log.Printf("Payment: %#v", payment)
	// ANCHOR_END: send-token-payment
	return nil
}
