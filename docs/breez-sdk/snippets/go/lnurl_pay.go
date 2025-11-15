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
		amountSats := uint64(5_000)
		optionalComment := "<comment>"
		optionalValidateSuccessActionUrl := true

		request := breez_sdk_spark.PrepareLnurlPayRequest{
			AmountSats:               amountSats,
			PayRequest:               inputType.Field0.PayRequest,
			Comment:                  &optionalComment,
			ValidateSuccessActionUrl: &optionalValidateSuccessActionUrl,
		}

		response, err := sdk.PrepareLnurlPay(request)

		if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
			return nil, err
		}

		// If the fees are acceptable, continue to create the LNURL Pay
		feeSats := response.FeeSats
		log.Printf("Fees: %v sats", feeSats)
		return &response, nil
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
