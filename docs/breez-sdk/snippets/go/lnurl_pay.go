package example

import (
	"errors"
	"log"
	"math/big"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func PrepareLnurlPay(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.PrepareLnurlPayResponse, error) {
	// ANCHOR: prepare-lnurl-pay
	// Endpoint can also be of the form:
	// lnurlp://domain.com/lnurl-pay?key=val
	// lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4excttsv9un7um9wdekjmmw84jxywf5x43rvv35xgmr2enrxanr2cfcvsmnwe3jxcukvde48qukgdec89snwde3vfjxvepjxpjnjvtpxd3kvdnxx5crxwpjvyunsephsz36jf
	lnurlPayUrl := "lightning@address.com"

	input, err := sdk.Parse(lnurlPayUrl)

	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
		return nil, err
	}

	switch inputType := input.(type) {
	case breez_sdk_spark.InputTypeLightningAddress:
		amountSats := new(big.Int).SetInt64(5_000)
		optionalComment := "<comment>"
		optionalValidateSuccessActionUrl := true

		request := breez_sdk_spark.PrepareLnurlPayRequest{
			Amount:                   amountSats,
			PayRequest:               inputType.Field0.PayRequest,
			Comment:                  &optionalComment,
			ValidateSuccessActionUrl: &optionalValidateSuccessActionUrl,
			TokenIdentifier:          nil,
			ConversionOptions:        nil,
			FeePolicy:                nil,
		}

		prepareResponse, err := sdk.PrepareLnurlPay(request)

		if err != nil {
			var sdkErr *breez_sdk_spark.SdkError
			if errors.As(err, &sdkErr) {
				// Handle SdkError - can inspect specific variants if needed
				// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
			}
			return nil, err
		}

		// If the fees are acceptable, continue to create the LNURL Pay
		feeSats := prepareResponse.FeeSats
		log.Printf("Fees: %v sats", feeSats)
		return &prepareResponse, nil
	}
	// ANCHOR_END: prepare-lnurl-pay
	return nil, nil
}

func LnurlPay(sdk *breez_sdk_spark.BreezSdk, prepareResponse breez_sdk_spark.PrepareLnurlPayResponse) (*breez_sdk_spark.Payment, error) {
	// ANCHOR: lnurl-pay
	optionalIdempotencyKey := "<idempotency key uuid>"
	request := breez_sdk_spark.LnurlPayRequest{
		PrepareResponse: prepareResponse,
		IdempotencyKey:  &optionalIdempotencyKey,
	}

	response, err := sdk.LnurlPay(request)
	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
		return nil, err
	}

	payment := response.Payment
	// ANCHOR_END: lnurl-pay
	return &payment, nil
}

func PrepareLnurlPayFeesIncluded(sdk *breez_sdk_spark.BreezSdk, payRequest breez_sdk_spark.LnurlPayRequestDetails) (*breez_sdk_spark.PrepareLnurlPayResponse, error) {
	// ANCHOR: prepare-lnurl-pay-fees-included
	// By default (FeePolicyFeesExcluded), fees are added on top of the amount.
	// Use FeePolicyFeesIncluded to deduct fees from the amount instead.
	// The receiver gets amount minus fees.
	amountSats := new(big.Int).SetInt64(5_000)
	optionalComment := "<comment>"
	optionalValidateSuccessActionUrl := true
	feePolicy := breez_sdk_spark.FeePolicyFeesIncluded

	request := breez_sdk_spark.PrepareLnurlPayRequest{
		Amount:                   amountSats,
		PayRequest:               payRequest,
		Comment:                  &optionalComment,
		ValidateSuccessActionUrl: &optionalValidateSuccessActionUrl,
		TokenIdentifier:          nil,
		ConversionOptions:        nil,
		FeePolicy:                &feePolicy,
	}

	response, err := sdk.PrepareLnurlPay(request)
	if err != nil {
		return nil, err
	}

	// If the fees are acceptable, continue to create the LNURL Pay
	feeSats := response.FeeSats
	log.Printf("Fees: %v sats", feeSats)
	// The receiver gets amountSats - feeSats
	// ANCHOR_END: prepare-lnurl-pay-fees-included
	return &response, nil
}
