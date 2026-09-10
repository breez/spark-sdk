# Sending payments using LNURL-Pay and Lightning address

## Preparing LNURL Payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.prepare_lnurl_pay

During the prepare step, the SDK ensures that the inputs are valid with respect to the LNURL-pay request,
and also returns the fees related to the payment so they can be confirmed.

Payments can be sent without holding Bitcoin by converting on-the-fly as a step before sending a payment. See <a href="./token_conversion.md">Converting tokens</a> for more information.

### Setting the receiver amount

When you want the payment recipient to receive a specific amount.

```swift
// Endpoint can also be of the form:
// lnurlp://domain.com/lnurl-pay?key=val
// lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4excttsv9un7um9wdekjmmw84jxywf5x43rvv35xgmr2enrxanr2cfcvsmnwe3jxcukvde48qukgdec89snwde3vfjxvepjxpjnjvtpxd3kvdnxx5crxwpjvyunsephsz36jf
let lnurlPayUrl = "lightning@address.com"

let inputType = try await sdk.parse(input: lnurlPayUrl)
if case .lightningAddress(v1: let details) = inputType {
    let amountSats = BInt(5_000)
    let optionalComment = "<comment>"
    let payRequest = details.payRequest
    let optionalValidateSuccessActionUrl = true

    let request = PrepareLnurlPayRequest(
        amount: amountSats,
        payRequest: payRequest,
        comment: optionalComment,
        validateSuccessActionUrl: optionalValidateSuccessActionUrl,
        tokenIdentifier: nil,
        conversionOptions: nil,
        feePolicy: nil
    )
    let prepareResponse = try await sdk.prepareLnurlPay(request: request)

    // If the fees are acceptable, continue to create the LNURL Pay
    let feeSats = prepareResponse.feeSats
    print("Fees: \(feeSats) sats")
}
```



### Setting the fee policy

By default, fees are added on top of the amount (`FeePolicy.feesExcluded`). Use `FeePolicy.feesIncluded` to deduct fees from the amount instead—the receiver gets the amount minus fees.

This is particularly useful when you want to spend your entire balance in a single payment—simply provide your full balance as the amount.

```swift
// By default (.feesExcluded), fees are added on top of the amount.
// Use .feesIncluded to deduct fees from the amount instead.
// The receiver gets amount minus fees.
let amountSats = BInt(5_000)
let optionalComment = "<comment>"
let optionalValidateSuccessActionUrl = true

let request = PrepareLnurlPayRequest(
    amount: amountSats,
    payRequest: payRequest,
    comment: optionalComment,
    validateSuccessActionUrl: optionalValidateSuccessActionUrl,
    tokenIdentifier: nil,
    conversionOptions: nil,
    feePolicy: .feesIncluded
)
let response = try await sdk.prepareLnurlPay(request: request)

// If the fees are acceptable, continue to create the LNURL Pay
let feeSats = response.feeSats
print("Fees: \(feeSats) sats")
// The receiver gets amountSats - feeSats
```



### Sending entire token balance

When [stable balance](./stable_balance.md) is active, you can send your entire wallet balance via LNURL. See [Sending entire balance](./stable_balance.md#sending-entire-balance) for details.

## LNURL Payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.lnurl_pay

Once the payment has been prepared and the fees are accepted, the payment can be sent by passing:
- **Prepare Response** - The response from the [Preparing LNURL Payments](lnurl_pay.md#preparing-lnurl-payments) step.
- **Idempotency Key** - An optional UUID that identifies the payment. If set, providing the same idempotency key for multiple requests will ensure that only one payment is made.

```swift
let optionalIdempotencyKey = "<idempotency key uuid>"
let response = try await sdk.lnurlPay(
    request: LnurlPayRequest(
        prepareResponse: prepareResponse,
        idempotencyKey: optionalIdempotencyKey
    ))
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
