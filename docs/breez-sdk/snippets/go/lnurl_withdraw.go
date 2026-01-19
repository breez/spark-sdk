package example

import (
	"errors"
	"log"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func LnurlWithdraw(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.LnurlWithdrawResponse, error) {
	// ANCHOR: lnurl-withdraw
	// Endpoint can also be of the form:
	// lnurlw://domain.com/lnurl-withdraw?key=val
	lnurlWithdrawUrl := "lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4exctthd96xserjv9mn7um9wdekj" +
		"mmw843xxwpexdnxzen9vgunsvfexq6rvdecx93rgdmyxcuxverrvcursenpxvukzv3c8qunsdecx33nzwpnvg6ryc3hv93nzvecxgcxgwp3h33lxk"

	input, err := sdk.Parse(lnurlWithdrawUrl)

	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
		return nil, err
	}

	switch inputType := input.(type) {
	case breez_sdk_spark.InputTypeLnurlWithdraw:
		// Amount to withdraw in sats between min/max withdrawable amounts
		amountSats := uint64(5_000)
		withdrawRequest := inputType.Field0
		optionalCompletionTimeoutSecs := uint32(30)

		request := breez_sdk_spark.LnurlWithdrawRequest{
			AmountSats:            amountSats,
			WithdrawRequest:       withdrawRequest,
			CompletionTimeoutSecs: &optionalCompletionTimeoutSecs,
		}

		response, err := sdk.LnurlWithdraw(request)

		if err != nil {
			var sdkErr *breez_sdk_spark.SdkError
			if errors.As(err, &sdkErr) {
				// Handle SdkError - can inspect specific variants if needed
				// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
			}
			return nil, err
		}

		payment := response.Payment
		log.Printf("Payment: %#v", payment)
		return &response, nil
	}
	// ANCHOR_END: lnurl-withdraw
	return nil, nil
}
