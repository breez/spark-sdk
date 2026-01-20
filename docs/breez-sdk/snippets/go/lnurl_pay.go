package example

import (
	"log"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func PrepareLnurlPay(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.PrepareLnurlPayResponse, error) {
	// ANCHOR: prepare-lnurl-pay
	// Endpoint can also be of the form:
	// lnurlp://domain.com/lnurl-pay?key=val
	// lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4excttsv9un7um9wdekjmmw84jxywf5x43rvv35xgmr2enrxanr2cfcvsmnwe3jxcukvde48qukgdec89snwde3vfjxvepjxpjnjvtpxd3kvdnxx5crxwpjvyunsephsz36jf
	lnurlPayUrl := "lightning@address.com"

	input, err := sdk.Parse(lnurlPayUrl)

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}

	switch inputType := input.(type) {
	case breez_sdk_spark.InputTypeLightningAddress:
		payAmount := breez_sdk_spark.BitcoinPayAmountBitcoin{AmountSats: 5_000}
		optionalComment := "<comment>"
		optionalValidateSuccessActionUrl := true
		// Optionally set to use token funds to pay via token conversion
		optionalMaxSlippageBps := uint32(50)
		optionalCompletionTimeoutSecs := uint32(30)
		optionalConversionOptions := breez_sdk_spark.ConversionOptions{
			ConversionType: breez_sdk_spark.ConversionTypeToBitcoin{
				FromTokenIdentifier: "<token identifier>",
			},
			MaxSlippageBps:        &optionalMaxSlippageBps,
			CompletionTimeoutSecs: &optionalCompletionTimeoutSecs,
		}

		request := breez_sdk_spark.PrepareLnurlPayRequest{
			PayAmount:                payAmount,
			PayRequest:               inputType.Field0.PayRequest,
			Comment:                  &optionalComment,
			ValidateSuccessActionUrl: &optionalValidateSuccessActionUrl,
			ConversionOptions:        &optionalConversionOptions,
		}

		prepareResponse, err := sdk.PrepareLnurlPay(request)

		if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
			return nil, err
		}

		// If the fees are acceptable, continue to create the LNURL Pay
		if prepareResponse.ConversionEstimate != nil {
			log.Printf("Estimated conversion amount: %v token base units", prepareResponse.ConversionEstimate.Amount)
			log.Printf("Estimated conversion fee: %v token base units", prepareResponse.ConversionEstimate.Fee)
		}

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
	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}

	payment := response.Payment
	// ANCHOR_END: lnurl-pay
	return &payment, nil
}

func PrepareLnurlPayDrain(sdk *breez_sdk_spark.BreezSdk, payRequest breez_sdk_spark.LnurlPayRequestDetails) (*breez_sdk_spark.PrepareLnurlPayResponse, error) {
	// ANCHOR: prepare-lnurl-pay-drain
	payAmount := breez_sdk_spark.BitcoinPayAmountDrain{}
	optionalComment := "<comment>"
	optionalValidateSuccessActionUrl := true

	request := breez_sdk_spark.PrepareLnurlPayRequest{
		PayAmount:                payAmount,
		PayRequest:               payRequest,
		Comment:                  &optionalComment,
		ValidateSuccessActionUrl: &optionalValidateSuccessActionUrl,
		ConversionOptions:        nil,
	}

	response, err := sdk.PrepareLnurlPay(request)
	if err != nil {
		return nil, err
	}

	// If the fees are acceptable, continue to create the LNURL Pay
	feeSats := response.FeeSats
	log.Printf("Fees: %v sats", feeSats)
	// ANCHOR_END: prepare-lnurl-pay-drain
	return &response, nil
}
