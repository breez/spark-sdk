# Sending payments using LNURL-Pay and Lightning address

## Preparing LNURL Payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.prepare_lnurl_pay

During the prepare step, the SDK ensures that the inputs are valid with respect to the LNURL-pay request,
and also returns the fees related to the payment so they can be confirmed.

Payments can be sent without holding Bitcoin by converting on-the-fly as a step before sending a payment. See <a href="./token_conversion.md">Converting tokens</a> for more information.

### Setting the receiver amount

When you want the payment recipient to receive a specific amount.

#### Rust

```rust
// Endpoint can also be of the form:
// lnurlp://domain.com/lnurl-pay?key=val
// lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4excttsv9un7um9wdekjmmw84jxywf5x43rvv35xgmr2enrxanr2cfcvsmnwe3jxcukvde48qukgdec89snwde3vfjxvepjxpjnjvtpxd3kvdnxx5crxwpjvyunsephsz36jf
let lnurl_pay_url = "lightning@address.com";

if let Ok(InputType::LightningAddress(details)) = sdk.parse(lnurl_pay_url).await {
    let amount_sats = 5_000;
    let optional_comment = Some("<comment>".to_string());
    let optional_validate_success_action_url = Some(true);

    let prepare_response = sdk
        .prepare_lnurl_pay(PrepareLnurlPayRequest {
            amount: amount_sats,
            pay_request: details.pay_request,
            comment: optional_comment,
            validate_success_action_url: optional_validate_success_action_url,
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    // If the fees are acceptable, continue to create the LNURL Pay
    let fee_sats = prepare_response.fee_sats;
    info!("Fees: {fee_sats} sats");
}
```

#### Swift

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

#### Kotlin

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

#### C#

```csharp
// Endpoint can also be of the form:
// lnurlp://domain.com/lnurl-pay?key=val
// lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4excttsv9un7um9wdekjmmw84jxywf5x43r
//     vv35xgmr2enrxanr2cfcvsmnwe3jxcukvde48qukgdec89snwde3vfjxvepjxpjnjvtpxd3k
//     vdnxx5crxwpjvyunsephsz36jf
var lnurlPayUrl = "lightning@address.com";
var parsedInput = await sdk.Parse(lnurlPayUrl);
if (parsedInput is InputType.LightningAddress lightningAddress)
{
    var details = lightningAddress.v1;
    var amountSats = 5_000UL;
    var optionalComment = "<comment>";
    var payRequest = details.payRequest;
    var optionalValidateSuccessActionUrl = true;

    var request = new PrepareLnurlPayRequest(
        amount: amountSats,
        payRequest: payRequest,
        comment: optionalComment,
        validateSuccessActionUrl: optionalValidateSuccessActionUrl,
        tokenIdentifier: null,
        conversionOptions: null,
        feePolicy: null
    );
    var prepareResponse = await sdk.PrepareLnurlPay(request: request);

    // If the fees are acceptable, continue to create the LNURL Pay
    var feeSats = prepareResponse.feeSats;
    Console.WriteLine($"Fees: {feeSats} sats");
}
```

#### Javascript (Wasm)

```typescript
// Endpoint can also be of the form:
// lnurlp://domain.com/lnurl-pay?key=val
// lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4excttsv9un7um9wdekjmmw84jxywf5x43rvv35xgmr2enrxanr2cfcvsmnwe3jxcukvde48qukgdec89snwde3vfjxvepjxpjnjvtpxd3kvdnxx5crxwpjvyunsephsz36jf
const lnurlPayUrl = 'lightning@address.com'

const input = await sdk.parse(lnurlPayUrl)
if (input.type === 'lightningAddress') {
  const amountSats = BigInt(5_000)
  const optionalComment = '<comment>'
  const payRequest = input.payRequest
  const optionalValidateSuccessActionUrl = true

  const prepareResponse = await sdk.prepareLnurlPay({
    amount: amountSats,
    payRequest,
    comment: optionalComment,
    validateSuccessActionUrl: optionalValidateSuccessActionUrl,
    tokenIdentifier: undefined,
    conversionOptions: undefined,
    feePolicy: undefined
  })

  // If the fees are acceptable, continue to create the LNURL Pay
  const feeSats = prepareResponse.feeSats
  console.log(`Fees: ${feeSats} sats`)
}
```

#### React Native

```typescript
// Endpoint can also be of the form:
// lnurlp://domain.com/lnurl-pay?key=val
// lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4excttsv9un7um9wdekjmmw84jxywf5x43rvv35xgmr2enrxanr2cfcvsmnwe3jxcukvde48qukgdec89snwde3vfjxvepjxpjnjvtpxd3kvdnxx5crxwpjvyunsephsz36jf
const lnurlPayUrl = 'lightning@address.com'

const input = await sdk.parse(lnurlPayUrl)
if (input.tag === InputType_Tags.LightningAddress) {
  const amountSats = BigInt(5_000)
  const optionalComment = '<comment>'
  const payRequest = input.inner[0].payRequest
  const optionalValidateSuccessActionUrl = true

  const prepareResponse = await sdk.prepareLnurlPay({
    amount: amountSats,
    payRequest,
    comment: optionalComment,
    validateSuccessActionUrl: optionalValidateSuccessActionUrl,
    tokenIdentifier: undefined,
    conversionOptions: undefined,
    feePolicy: undefined
  })

  // If the fees are acceptable, continue to create the LNURL Pay
  const feeSats = prepareResponse.feeSats
  console.log(`Fees: ${feeSats} sats`)
}
```

#### Flutter

```dart
// Endpoint can also be of the form:
// lnurlp://domain.com/lnurl-pay?key=val
// lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4excttsv9un7um9wdekjmmw84jxywf5x43rvv35xgmr2enrxanr2cfcvsmnwe3jxcukvde48qukgdec89snwde3vfjxvepjxpjnjvtpxd3kvdnxx5crxwpjvyunsephsz36jf
String lnurlPayUrl = "lightning@address.com";

InputType inputType = await sdk.parse(input: lnurlPayUrl);
if (inputType is InputType_LightningAddress) {
  BigInt amountSats = BigInt.from(5000);
  String optionalComment = "<comment>";
  bool optionalValidateSuccessActionUrl = true;

  PrepareLnurlPayRequest request = PrepareLnurlPayRequest(
    amount: amountSats,
    payRequest: inputType.field0.payRequest,
    comment: optionalComment,
    validateSuccessActionUrl: optionalValidateSuccessActionUrl,
    tokenIdentifier: null,
    conversionOptions: null,
    feePolicy: null,
  );
  PrepareLnurlPayResponse prepareResponse =
      await sdk.prepareLnurlPay(request: request);

  // If the fees are acceptable, continue to create the LNURL Pay
  BigInt feeSats = prepareResponse.feeSats;
  print("Fees: $feeSats sats");
}
```

#### Python

```python
# Endpoint can also be of the form:
# lnurlp://domain.com/lnurl-pay?key=val
# lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4excttsv9un7um9wdekjmmw84jxywf5x43r
#     vv35xgmr2enrxanr2cfcvsmnwe3jxcukvde48qukgdec89snwde3vfjxvepjxpjnjvtpxd3k
#     vdnxx5crxwpjvyunsephsz36jf
lnurl_pay_url = "lightning@address.com"
try:
    parsed_input = await sdk.parse(lnurl_pay_url)
    if isinstance(parsed_input, InputType.LIGHTNING_ADDRESS):
        details = parsed_input[0]
        amount_sats = 5_000
        optional_comment = "<comment>"
        pay_request = details.pay_request
        optional_validate_success_action_url = True

        request = PrepareLnurlPayRequest(
            amount=amount_sats,
            pay_request=pay_request,
            comment=optional_comment,
            validate_success_action_url=optional_validate_success_action_url,
            token_identifier=None,
            conversion_options=None,
            fee_policy=None,
        )
        prepare_response = await sdk.prepare_lnurl_pay(request=request)

        # If the fees are acceptable, continue to create the LNURL Pay
        logging.debug(f"Fees: {prepare_response.fee_sats} sats")
        return prepare_response
except Exception as error:
    logging.error(error)
    raise
```

#### Go

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

By default, fees are added on top of the amount (`FeePolicy::FeesExcluded`). Use `FeePolicy::FeesIncluded` to deduct fees from the amount instead—the receiver gets the amount minus fees.

This is particularly useful when you want to spend your entire balance in a single payment—simply provide your full balance as the amount.

#### Rust

```rust
// By default (FeePolicy::FeesExcluded), fees are added on top of the amount.
// Use FeePolicy::FeesIncluded to deduct fees from the amount instead.
// The receiver gets amount minus fees.
let optional_comment = Some("<comment>".to_string());
let optional_validate_success_action_url = Some(true);
let amount_sats = 5_000;

let prepare_response = sdk
    .prepare_lnurl_pay(PrepareLnurlPayRequest {
        amount: amount_sats,
        pay_request,
        comment: optional_comment,
        validate_success_action_url: optional_validate_success_action_url,
        token_identifier: None,
        conversion_options: None,
        fee_policy: Some(FeePolicy::FeesIncluded),
    })
    .await?;

// If the fees are acceptable, continue to create the LNURL Pay
let fee_sats = prepare_response.fee_sats;
info!("Fees: {fee_sats} sats");
// The receiver gets amount_sats - fee_sats
```

#### Swift

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

#### Kotlin

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

#### C#

```csharp
// By default (FeePolicy.FeesExcluded), fees are added on top of the amount.
// Use FeePolicy.FeesIncluded to deduct fees from the amount instead.
// The receiver gets amount minus fees.
var amountSats = 5_000UL;
var optionalComment = "<comment>";
var optionalValidateSuccessActionUrl = true;

var request = new PrepareLnurlPayRequest(
    amount: amountSats,
    payRequest: payRequest,
    comment: optionalComment,
    validateSuccessActionUrl: optionalValidateSuccessActionUrl,
    tokenIdentifier: null,
    conversionOptions: null,
    feePolicy: FeePolicy.FeesIncluded
);
var prepareResponse = await sdk.PrepareLnurlPay(request: request);

// If the fees are acceptable, continue to create the LNURL Pay
var feeSats = prepareResponse.feeSats;
Console.WriteLine($"Fees: {feeSats} sats");
// The receiver gets amountSats - feeSats
```

#### Javascript (Wasm)

```typescript
// By default ({ type: 'feesExcluded' }), fees are added on top of the amount.
// Use { type: 'feesIncluded' } to deduct fees from the amount instead.
// The receiver gets amount minus fees.
const optionalComment = '<comment>'
const optionalValidateSuccessActionUrl = true
const amountSats = BigInt(5_000)
const feePolicy: FeePolicy = 'feesIncluded'

const prepareResponse = await sdk.prepareLnurlPay({
  amount: amountSats,
  payRequest,
  comment: optionalComment,
  validateSuccessActionUrl: optionalValidateSuccessActionUrl,
  tokenIdentifier: undefined,
  conversionOptions: undefined,
  feePolicy
})

// If the fees are acceptable, continue to create the LNURL Pay
const feeSats = prepareResponse.feeSats
console.log(`Fees: ${feeSats} sats`)
// The receiver gets amountSats - feeSats
```

#### React Native

```typescript
// By default (FeePolicy.FeesExcluded), fees are added on top of the amount.
// Use FeePolicy.FeesIncluded to deduct fees from the amount instead.
// The receiver gets amount minus fees.
const optionalComment = '<comment>'
const optionalValidateSuccessActionUrl = true
const amountSats = BigInt(5_000)

const prepareResponse = await sdk.prepareLnurlPay({
  amount: amountSats,
  payRequest,
  comment: optionalComment,
  validateSuccessActionUrl: optionalValidateSuccessActionUrl,
  tokenIdentifier: undefined,
  conversionOptions: undefined,
  feePolicy: FeePolicy.FeesIncluded
})

// If the fees are acceptable, continue to create the LNURL Pay
const feeSats = prepareResponse.feeSats
console.log(`Fees: ${feeSats} sats`)
// The receiver gets amountSats - feeSats
```

#### Flutter

```dart
// By default (FeePolicy.feesExcluded), fees are added on top of the amount.
// Use FeePolicy.feesIncluded to deduct fees from the amount instead.
// The receiver gets amount minus fees.
String optionalComment = "<comment>";
bool optionalValidateSuccessActionUrl = true;
BigInt amountSats = BigInt.from(5000);

PrepareLnurlPayRequest request = PrepareLnurlPayRequest(
  amount: amountSats,
  payRequest: payRequest,
  comment: optionalComment,
  validateSuccessActionUrl: optionalValidateSuccessActionUrl,
  tokenIdentifier: null,
  conversionOptions: null,
  feePolicy: FeePolicy.feesIncluded,
);
PrepareLnurlPayResponse prepareResponse =
    await sdk.prepareLnurlPay(request: request);

// If the fees are acceptable, continue to create the LNURL Pay
BigInt feeSats = prepareResponse.feeSats;
print("Fees: $feeSats sats");
// The receiver gets amountSats - feeSats
```

#### Python

```python
# By default (FeePolicy.FEES_EXCLUDED), fees are added on top of the amount.
# Use FeePolicy.FEES_INCLUDED to deduct fees from the amount instead.
# The receiver gets amount minus fees.
amount_sats = 5_000
optional_comment = "<comment>"
optional_validate_success_action_url = True

request = PrepareLnurlPayRequest(
    amount=amount_sats,
    pay_request=pay_request,
    comment=optional_comment,
    validate_success_action_url=optional_validate_success_action_url,
    token_identifier=None,
    conversion_options=None,
    fee_policy=FeePolicy.FEES_INCLUDED,
)
prepare_response = await sdk.prepare_lnurl_pay(request=request)

# If the fees are acceptable, continue to create the LNURL Pay
fee_sats = prepare_response.fee_sats
logging.debug(f"Fees: {fee_sats} sats")
# The receiver gets amount_sats - fee_sats
```

#### Go

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

### Rust

```rust
let optional_idempotency_key = Some("<idempotency key uuid>".to_string());
let response = sdk
    .lnurl_pay(LnurlPayRequest {
        prepare_response,
        idempotency_key: optional_idempotency_key,
    })
    .await?;
```

### Swift

```swift
let optionalIdempotencyKey = "<idempotency key uuid>"
let response = try await sdk.lnurlPay(
    request: LnurlPayRequest(
        prepareResponse: prepareResponse,
        idempotencyKey: optionalIdempotencyKey
    ))
```

### Kotlin

```kotlin
try {
    val optionalIdempotencyKey = "<idempotency key uuid>"
    val response = sdk.lnurlPay(LnurlPayRequest(prepareResponse, optionalIdempotencyKey))
} catch (e: Exception) {
    // handle error
}
```

### C#

```csharp
var optionalIdempotencyKey = "<idempotency key uuid>";
var response = await sdk.LnurlPay(
    new LnurlPayRequest(
        prepareResponse: prepareResponse,
        idempotencyKey: optionalIdempotencyKey
    )
);
```

### Javascript (Wasm)

```typescript
const optionalIdempotencyKey = '<idempotency key uuid>'
const response = await sdk.lnurlPay({
  prepareResponse,
  idempotencyKey: optionalIdempotencyKey
})
```

### React Native

```typescript
const optionalIdempotencyKey = '<idempotency key uuid>'
const response = await sdk.lnurlPay({
  prepareResponse,
  idempotencyKey: optionalIdempotencyKey
})
```

### Flutter

```dart
String? optionalIdempotencyKey = "<idempotency key uuid>";
LnurlPayResponse response = await sdk.lnurlPay(
  request: LnurlPayRequest(
    prepareResponse: prepareResponse,
    idempotencyKey: optionalIdempotencyKey),
);
```

### Python

```python
try:
    optional_idempotency_key = "<idempotency key uuid>"
    response = await sdk.lnurl_pay(
        LnurlPayRequest(
            prepare_response=prepare_response,
            idempotency_key=optional_idempotency_key,
        )
    )
except Exception as error:
    logging.error(error)
    raise
```

### Go

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

---

Identifier casing: `get_info` here is `getInfo` in Swift, Kotlin, JavaScript, React Native and Flutter, and `GetInfo` in Go and C#. Enum variants: `SdkEvent::Synced` is `SdkEvent.SYNCED` in Python, `SdkEvent.synced` in Swift, `SdkEventSynced` in Go, and `SdkEvent.Synced` elsewhere.
