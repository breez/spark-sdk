# Receiving payments using LNURL-Withdraw

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.lnurl_withdraw

After [parsing](parse.md) an LNURL-Withdraw input, you can use the resulting input data to initiate a withdrawal from an LNURL service.

By default, this function returns immediately. You can override this behavior by specifying a completion timeout in seconds. If the completion timeout is hit, a pending payment object is returned if available. If the payment completes, the completed payment object is returned.

**Developer note**

The minimum and maximum withdrawable amount returned from calling parse is denominated in millisatoshi.

```go
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
```



## Supported Specs

- [LUD-01](https://github.com/lnurl/luds/blob/luds/01.md) LNURL bech32 encoding
- [LUD-03](https://github.com/lnurl/luds/blob/luds/03.md) `withdrawRequest` spec
- [LUD-17](https://github.com/lnurl/luds/blob/luds/17.md) Support for lnurlw prefix with non-bech32-encoded LNURL URLs
