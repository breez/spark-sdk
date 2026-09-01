# Sending payments using LNURL-Pay and Lightning address

## Preparing LNURL Payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.prepare_lnurl_pay

During the prepare step, the SDK ensures that the inputs are valid with respect to the LNURL-pay request,
and also returns the fees related to the payment so they can be confirmed.

Payments can be sent without holding Bitcoin by converting on-the-fly as a step before sending a payment. See <a href="./token_conversion.md">Converting tokens</a> for more information.

### Setting the receiver amount

When you want the payment recipient to receive a specific amount.

```kotlin
// Endpoint can also be of the form:
// lnurlp://domain.com/lnurl-pay?key=val
// lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4excttsv9un7um9wdekjmmw84jxywf5x43rvv35xgmr2enrxanr2cfcvsmnwe3jxcukvde48qukgdec89snwde3vfjxvepjxpjnjvtpxd3kvdnxx5crxwpjvyunsephsz36jf
val lnurlPayUrl = "lightning@address.com"
try {
    val inputType = sdk.parse(lnurlPayUrl)
    if (inputType is InputType.LightningAddress) {
        val amountSats = BigInteger.fromLong(5_000L)
        val optionalComment = "<comment>"
        val payRequest = inputType.v1.payRequest
        val optionalValidateSuccessActionUrl = true

        val req = PrepareLnurlPayRequest(
            amount = amountSats,
            payRequest = payRequest,
            comment = optionalComment,
            validateSuccessActionUrl = optionalValidateSuccessActionUrl,
            tokenIdentifier = null,
            conversionOptions = null,
            feePolicy = null,
        )
        val prepareResponse = sdk.prepareLnurlPay(req)

        // If the fees are acceptable, continue to create the LNURL Pay
        val feeSats = prepareResponse.feeSats
        // Log.v("Breez", "Fees: ${feeSats} sats")
    }
} catch (e: Exception) {
    // handle error
}
```



### Setting the fee policy

By default, fees are added on top of the amount (`FeePolicy.FeesExcluded`). Use `FeePolicy.FeesIncluded` to deduct fees from the amount instead—the receiver gets the amount minus fees.

This is particularly useful when you want to spend your entire balance in a single payment—simply provide your full balance as the amount.

```kotlin
// By default (FeePolicy.FEES_EXCLUDED), fees are added on top of the amount.
// Use FeePolicy.FEES_INCLUDED to deduct fees from the amount instead.
// The receiver gets amount minus fees.
val optionalComment = "<comment>"
val optionalValidateSuccessActionUrl = true
val amountSats = BigInteger.fromLong(5_000L)

val req = PrepareLnurlPayRequest(
    amount = amountSats,
    payRequest = payRequest,
    comment = optionalComment,
    validateSuccessActionUrl = optionalValidateSuccessActionUrl,
    tokenIdentifier = null,
    conversionOptions = null,
    feePolicy = FeePolicy.FEES_INCLUDED,
)
val prepareResponse = sdk.prepareLnurlPay(req)

// If the fees are acceptable, continue to create the LNURL Pay
val feeSats = prepareResponse.feeSats
// Log.v("Breez", "Fees: ${feeSats} sats")
// The receiver gets amountSats - feeSats
```



### Sending entire token balance

When [stable balance](./stable_balance.md) is active, you can send your entire wallet balance via LNURL. See [Sending entire balance](./stable_balance.md#sending-entire-balance) for details.

## LNURL Payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.lnurl_pay

Once the payment has been prepared and the fees are accepted, the payment can be sent by passing:
- **Prepare Response** - The response from the [Preparing LNURL Payments](lnurl_pay.md#preparing-lnurl-payments) step.
- **Idempotency Key** - An optional UUID that identifies the payment. If set, providing the same idempotency key for multiple requests will ensure that only one payment is made.

```kotlin
try {
    val optionalIdempotencyKey = "<idempotency key uuid>"
    val response = sdk.lnurlPay(LnurlPayRequest(prepareResponse, optionalIdempotencyKey))
} catch (e: Exception) {
    // handle error
}
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
