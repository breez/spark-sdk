# Sending payments using LNURL-Pay and Lightning address

## Preparing LNURL Payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.prepare_lnurl_pay

During the prepare step, the SDK ensures that the inputs are valid with respect to the LNURL-pay request,
and also returns the fees related to the payment so they can be confirmed.

Payments can be sent without holding Bitcoin by converting on-the-fly as a step before sending a payment. See <a href="./token_conversion.md">Converting tokens</a> for more information.

### Setting the receiver amount

When you want the payment recipient to receive a specific amount.

```go
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
```



### Setting the fee policy

By default, fees are added on top of the amount (`FeePolicyFeesExcluded`). Use `FeePolicyFeesIncluded` to deduct fees from the amount instead—the receiver gets the amount minus fees.

This is particularly useful when you want to spend your entire balance in a single payment—simply provide your full balance as the amount.

```go
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
```



### Sending entire token balance

When [stable balance](./stable_balance.md) is active, you can send your entire wallet balance via LNURL. See [Sending entire balance](./stable_balance.md#sending-entire-balance) for details.

## LNURL Payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.lnurl_pay

Once the payment has been prepared and the fees are accepted, the payment can be sent by passing:
- **Prepare Response** - The response from the [Preparing LNURL Payments](lnurl_pay.md#preparing-lnurl-payments) step.
- **Idempotency Key** - An optional UUID that identifies the payment. If set, providing the same idempotency key for multiple requests will ensure that only one payment is made.

```go
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
```



**Developer note**

By default when the LNURL-pay results in a success action with a URL, the URL is validated to check if there is a mismatch with the LNURL callback domain. You can disable this behaviour by setting the optional validation <code>PrepareLnurlPayRequest</code> param to false.

## Managing contacts

You can save frequently used Lightning addresses as contacts for quick access. See [Managing contacts](contacts.md) for details.

## Supported Specs

- [LUD-01](https://github.com/lnurl/luds/blob/luds/01.md) LNURL bech32 encoding
- [LUD-06](https://github.com/lnurl/luds/blob/luds/06.md) `payRequest` spec
- [LUD-09](https://github.com/lnurl/luds/blob/luds/09.md) `successAction` field for `payRequest`
- [LUD-16](https://github.com/lnurl/luds/blob/luds/16.md) LN Address
- [LUD-17](https://github.com/lnurl/luds/blob/luds/17.md) Support for lnurlp prefix with non-bech32-encoded LNURL URLs
