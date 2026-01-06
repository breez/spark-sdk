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

func FetchTokenMetadata(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: fetch-token-metadata
	tokenIdentifiers := []string{"<token identifier 1>", "<token identifier 2>"}
	response, err := sdk.GetTokensMetadata(breez_sdk_spark.GetTokensMetadataRequest{
		TokenIdentifiers: tokenIdentifiers,
	})

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return err
	}

	tokensMetadata := response.TokensMetadata
	for _, tokenMetadata := range tokensMetadata {
		log.Printf("Token ID: %v", tokenMetadata.Identifier)
		log.Printf("Name: %v", tokenMetadata.Name)
		log.Printf("Ticker: %v", tokenMetadata.Ticker)
		log.Printf("Decimals: %v", tokenMetadata.Decimals)
		log.Printf("Max Supply: %v", tokenMetadata.MaxSupply)
		log.Printf("Is Freezable: %v", tokenMetadata.IsFreezable)
	}
	// ANCHOR_END: fetch-token-metadata
	return nil
}

func ReceiveTokenPaymentSparkInvoice(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.ReceivePaymentResponse, error) {
	// ANCHOR: receive-token-payment-spark-invoice
	tokenIdentifier := "<token identifier>"
	optionalDescription := "<invoice description>"
	optionalAmount := new(big.Int).SetInt64(5_000)
	optionalExpiryTimeSeconds := uint64(1716691200)
	optionalSenderPublicKey := "<sender public key>"

	request := breez_sdk_spark.ReceivePaymentRequest{
		PaymentMethod: breez_sdk_spark.ReceivePaymentMethodSparkInvoice{
			TokenIdentifier: &tokenIdentifier,
			Description:     &optionalDescription,
			Amount:          &optionalAmount,
			ExpiryTime:      &optionalExpiryTimeSeconds,
			SenderPublicKey: &optionalSenderPublicKey,
		},
	}

	response, err := sdk.ReceivePayment(request)

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}

	paymentRequest := response.PaymentRequest
	log.Printf("Payment Request: %v", paymentRequest)
	receiveFees := response.Fee
	log.Printf("Fees: %v token base units", receiveFees)
	// ANCHOR_END: receive-token-payment-spark-invoice
	return &response, nil
}

func SendTokenPayment(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: send-token-payment
	paymentRequest := "<spark address or invoice>"
	// Token identifier must match the invoice in case it specifies one.
	tokenIdentifier := "<token identifier>"
	// Set the amount of tokens you wish to send.
	optionalAmount := new(big.Int).SetInt64(1_000)

	prepareResponse, err := sdk.PrepareSendPayment(breez_sdk_spark.PrepareSendPaymentRequest{
		PaymentRequest:  paymentRequest,
		Amount:          &optionalAmount,
		TokenIdentifier: &tokenIdentifier,
	})

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return err
	}

	// If the fees are acceptable, continue to send the token payment
	switch method := prepareResponse.PaymentMethod.(type) {
	case breez_sdk_spark.SendPaymentMethodSparkAddress:
		log.Printf("Token ID: %v", method.TokenIdentifier)
		log.Printf("Fees: %v token base units", method.Fee)
	case breez_sdk_spark.SendPaymentMethodSparkInvoice:
		log.Printf("Token ID: %v", method.TokenIdentifier)
		log.Printf("Fees: %v token base units", method.Fee)
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

func PrepareSendTokenPaymentTokenConversion(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: prepare-send-payment-token-conversion
	paymentRequest := "<spark address or invoice>"
	// Token identifier must match the invoice in case it specifies one.
	tokenIdentifier := "<token identifier>"
	// Set the amount of tokens you wish to send.
	optionalAmount := new(big.Int).SetInt64(1_000)
	// Set to use Bitcoin funds to pay via token conversion
	optionalMaxSlippageBps := uint32(50)
	optionalCompletionTimeoutSecs := uint32(30)
	tokenConversionOptions := &breez_sdk_spark.TokenConversionOptions{
		ConversionType:        breez_sdk_spark.TokenConversionTypeToBitcoin{},
		MaxSlippageBps:        &optionalMaxSlippageBps,
		CompletionTimeoutSecs: &optionalCompletionTimeoutSecs,
	}

	prepareResponse, err := sdk.PrepareSendPayment(breez_sdk_spark.PrepareSendPaymentRequest{
		PaymentRequest:         paymentRequest,
		Amount:                 &optionalAmount,
		TokenIdentifier:        &tokenIdentifier,
		TokenConversionOptions: tokenConversionOptions,
	})

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return err
	}

	// If the fees are acceptable, continue to send the token payment
	if prepareResponse.TokenConversionFee != nil {
		log.Printf("Estimated token conversion fee: %v sats", *prepareResponse.TokenConversionFee)
	}
	// ANCHOR_END: prepare-send-payment-token-conversion
	return nil
}
