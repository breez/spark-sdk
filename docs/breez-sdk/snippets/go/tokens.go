package example

import (
	"errors"
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

	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
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

	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
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
	// Optionally set the expiry UNIX timestamp in seconds
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

	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
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

	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
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

	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
		return err
	}

	payment := sendResponse.Payment
	log.Printf("Payment: %#v", payment)
	// ANCHOR_END: send-token-payment
	return nil
}

func FetchConversionLimits(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: fetch-conversion-limits
	// Fetch limits for converting Bitcoin to a token
	tokenIdentifier := "<token identifier>"
	fromBitcoinResponse, err := sdk.FetchConversionLimits(breez_sdk_spark.FetchConversionLimitsRequest{
		ConversionType:  breez_sdk_spark.ConversionTypeFromBitcoin{},
		TokenIdentifier: &tokenIdentifier,
	})

	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
		return err
	}

	if fromBitcoinResponse.MinFromAmount != nil {
		log.Printf("Minimum BTC to convert: %v sats", *fromBitcoinResponse.MinFromAmount)
	}
	if fromBitcoinResponse.MinToAmount != nil {
		log.Printf("Minimum tokens to receive: %v base units", *fromBitcoinResponse.MinToAmount)
	}

	// Fetch limits for converting a token to Bitcoin
	fromTokenIdentifier := "<token identifier>"
	toBitcoinResponse, err := sdk.FetchConversionLimits(breez_sdk_spark.FetchConversionLimitsRequest{
		ConversionType: breez_sdk_spark.ConversionTypeToBitcoin{
			FromTokenIdentifier: fromTokenIdentifier,
		},
		TokenIdentifier: nil,
	})

	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
		return err
	}

	if toBitcoinResponse.MinFromAmount != nil {
		log.Printf("Minimum tokens to convert: %v base units", *toBitcoinResponse.MinFromAmount)
	}
	if toBitcoinResponse.MinToAmount != nil {
		log.Printf("Minimum BTC to receive: %v sats", *toBitcoinResponse.MinToAmount)
	}
	// ANCHOR_END: fetch-conversion-limits
	return nil
}

func PrepareSendTokenPaymentTokenConversion(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: prepare-send-payment-with-conversion
	paymentRequest := "<spark address or invoice>"
	// Token identifier must match the invoice in case it specifies one.
	tokenIdentifier := "<token identifier>"
	// Set the amount of tokens you wish to send.
	optionalAmount := new(big.Int).SetInt64(1_000)
	// Set to use Bitcoin funds to pay via conversion
	optionalMaxSlippageBps := uint32(50)
	optionalCompletionTimeoutSecs := uint32(30)
	conversionOptions := &breez_sdk_spark.ConversionOptions{
		ConversionType:        breez_sdk_spark.ConversionTypeToBitcoin{},
		MaxSlippageBps:        &optionalMaxSlippageBps,
		CompletionTimeoutSecs: &optionalCompletionTimeoutSecs,
	}

	prepareResponse, err := sdk.PrepareSendPayment(breez_sdk_spark.PrepareSendPaymentRequest{
		PaymentRequest:    paymentRequest,
		Amount:            &optionalAmount,
		TokenIdentifier:   &tokenIdentifier,
		ConversionOptions: conversionOptions,
	})

	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
		return err
	}

	// If the fees are acceptable, continue to send the token payment
	if prepareResponse.ConversionEstimate != nil {
		log.Printf("Estimated conversion amount: %v sats", prepareResponse.ConversionEstimate.Amount)
		log.Printf("Estimated conversion fee: %v sats", prepareResponse.ConversionEstimate.Fee)
	}
	// ANCHOR_END: prepare-send-payment-with-conversion
	return nil
}
