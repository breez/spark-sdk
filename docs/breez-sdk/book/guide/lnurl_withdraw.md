# Receiving payments using LNURL-Withdraw

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.lnurl_withdraw

After [parsing](parse.md) an LNURL-Withdraw input, you can use the resulting input data to initiate a withdrawal from an LNURL service.

By default, this function returns immediately. You can override this behavior by specifying a completion timeout in seconds. If the completion timeout is hit, a pending payment object is returned if available. If the payment completes, the completed payment object is returned.

**Developer note**

The minimum and maximum withdrawable amount returned from calling parse is denominated in millisatoshi.

## Rust

```rust
// Endpoint can also be of the form:
// lnurlw://domain.com/lnurl-withdraw?key=val
let lnurl_withdraw_url = "lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4exctthd96xserjv9mn7um9wdekjmmw843xxwpexdnxzen9vgunsvfexq6rvdecx93rgdmyxcuxverrvcursenpxvukzv3c8qunsdecx33nzwpnvg6ryc3hv93nzvecxgcxgwp3h33lxk";

if let Ok(InputType::LnurlWithdraw(withdraw_request)) = sdk.parse(lnurl_withdraw_url).await {
    // Amount to withdraw in sats between min/max withdrawable amounts
    let amount_sats = 5_000;
    let optional_completion_timeout_secs = Some(30);

    let response = sdk
        .lnurl_withdraw(LnurlWithdrawRequest {
            amount_sats,
            withdraw_request,
            completion_timeout_secs: optional_completion_timeout_secs,
        })
        .await?;

    let payment = response.payment;
    info!("Payment: {payment:?}");
}
```

## Swift

```swift
// Endpoint can also be of the form:
// lnurlw://domain.com/lnurl-withdraw?key=val
let lnurlWithdrawUrl = "lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4exctthd96xserjv9mn7um9wdekjmmw843xxwpexdnxzen9vgunsvfexq6rvdecx93rgdmyxcuxverrvcursenpxvukzv3c8qunsdecx33nzwpnvg6ryc3hv93nzvecxgcxgwp3h33lxk"

let inputType = try await sdk.parse(input: lnurlWithdrawUrl)
if case .lnurlWithdraw(v1: let withdrawRequest) = inputType {
    // Amount to withdraw in sats between min/max withdrawable amounts
    let amountSats: UInt64 = 5_000
    let optionalCompletionTimeoutSecs: UInt32 = 30

    let request = LnurlWithdrawRequest(
        amountSats: amountSats,
        withdrawRequest: withdrawRequest,
        completionTimeoutSecs: optionalCompletionTimeoutSecs
    )
    let response = try await sdk.lnurlWithdraw(request: request)

    let payment = response.payment
    print("Payment: \(payment)")
}
```

## Kotlin

```kotlin
// Endpoint can also be of the form:
// lnurlw://domain.com/lnurl-withdraw?key=val
val lnurlWithdrawUrl = "lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7..."
try {
    val inputType = sdk.parse(lnurlWithdrawUrl)
    if (inputType is InputType.LnurlWithdraw) {
        // Amount to withdraw in sats between min/max withdrawable amounts
        val amountSats = 5_000.toULong()
        val withdrawRequest = inputType.v1
        val optionalCompletionTimeoutSecs = 30.toUInt()

        val request = LnurlWithdrawRequest(
            amountSats,
            withdrawRequest,
            optionalCompletionTimeoutSecs
        )
        val response = sdk.lnurlWithdraw(request)

        val payment = response.payment
        // Log.v("Breez", "Payment: $payment")
    }
} catch (e: Exception) {
    // handle error
}
```

## C#

```csharp
// Endpoint can also be of the form:
// lnurlw://domain.com/lnurl-withdraw?key=val
var lnurlWithdrawUrl = "lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4exctthd96xserjv9mn7um9wdekj" +
                        "mmw843xxwpexdnxzen9vgunsvfexq6rvdecx93rgdmyxcuxverrvcursenpxvukzv3c8" +
                        "qunsdecx33nzwpnvg6ryc3hv93nzvecxgcxgwp3h33lxk";

var inputType = await sdk.Parse(lnurlWithdrawUrl);
if (inputType is InputType.LnurlWithdraw lnurlWithdraw)
{
    // Amount to withdraw in sats between min/max withdrawable amounts
    var amountSats = 5_000UL;
    var withdrawRequest = lnurlWithdraw.v1;
    var optionalCompletionTimeoutSecs = 30U;

    var request = new LnurlWithdrawRequest(
        amountSats: amountSats,
        withdrawRequest: withdrawRequest,
        completionTimeoutSecs: optionalCompletionTimeoutSecs
    );
    var response = await sdk.LnurlWithdraw(request: request);

    var payment = response.payment;
    Console.WriteLine($"Payment: {payment}");
}
```

## Javascript (Wasm)

```typescript
// Endpoint can also be of the form:
// lnurlw://domain.com/lnurl-withdraw?key=val
const lnurlWithdrawUrl =
  'lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4exctthd96xserjv9mn7um9wdekjmmw843xxwpexdnxzen9vgunsvfexq6rvdecx93rgdmyxcuxverrvcursenpxvukzv3c8qunsdecx33nzwpnvg6ryc3hv93nzvecxgcxgwp3h33lxk'

const input = await sdk.parse(lnurlWithdrawUrl)
if (input.type === 'lnurlWithdraw') {
  // Amount to withdraw in sats between min/max withdrawable amounts
  const amountSats = 5_000
  const withdrawRequest = input
  const optionalCompletionTimeoutSecs = 30

  const response = await sdk.lnurlWithdraw({
    amountSats,
    withdrawRequest,
    completionTimeoutSecs: optionalCompletionTimeoutSecs
  })

  const payment = response.payment
  console.log(`Payment: ${JSON.stringify(payment)}`)
}
```

## React Native

```typescript
// Endpoint can also be of the form:
// lnurlw://domain.com/lnurl-withdraw?key=val
const lnurlWithdrawUrl =
  'lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4exctthd96xserjv9mn7um9wdekjmmw843xxwpexdnxzen9vgunsvfexq6rvdecx93rgdmyxcuxverrvcursenpxvukzv3c8qunsdecx33nzwpnvg6ryc3hv93nzvecxgcxgwp3h33lxk'

const input = await sdk.parse(lnurlWithdrawUrl)
if (input.tag === InputType_Tags.LnurlWithdraw) {
  // Amount to withdraw in sats between min/max withdrawable amounts
  const amountSats = BigInt(5_000)
  const withdrawRequest = input.inner[0]
  const optionalCompletionTimeoutSecs = 30

  const response = await sdk.lnurlWithdraw({
    amountSats,
    withdrawRequest,
    completionTimeoutSecs: optionalCompletionTimeoutSecs
  })

  const payment = response.payment
  console.log(`Payment: ${JSON.stringify(payment)}`)
}
```

## Flutter

```dart
/// Endpoint can also be of the form:
/// lnurlw://domain.com/lnurl-withdraw?key=val
String lnurlWithdrawUrl =
    "lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4exctthd96xserjv9mn7um9wdekjmmw843xxwpexdnxzen9vgunsvfexq6rvdecx93rgdmyxcuxverrvcursenpxvukzv3c8qunsdecx33nzwpnvg6ryc3hv93nzvecxgcxgwp3h33lxk";

InputType inputType = await sdk.parse(input: lnurlWithdrawUrl);
if (inputType is InputType_LnurlWithdraw) {
  // Amount to withdraw in sats between min/max withdrawable amounts
  BigInt amountSats = BigInt.from(5000);
  LnurlWithdrawRequestDetails withdrawRequest = inputType.field0;
  int optionalCompletionTimeoutSecs = 30;

  LnurlWithdrawRequest request = LnurlWithdrawRequest(
    amountSats: amountSats,
    withdrawRequest: withdrawRequest,
    completionTimeoutSecs: optionalCompletionTimeoutSecs,
  );

  LnurlWithdrawResponse response = await sdk.lnurlWithdraw(request: request);

  Payment? payment = response.payment;
  print('Payment: $payment');
}
```

## Python

```python
# Endpoint can also be of the form:
# lnurlw://domain.com/lnurl-withdraw?key=val
lnurl_withdraw_url = (
    "lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4exctthd96xserjv9mn7um9wdekj"
    "mmw843xxwpexdnxzen9vgunsvfexq6rvdecx93rgdmyxcuxverrvcursenpxvukzv3c8"
    "qunsdecx33nzwpnvg6ryc3hv93nzvecxgcxgwp3h33lxk"
)

try:
    input_type = await sdk.parse(lnurl_withdraw_url)
    if isinstance(input_type, InputType.LNURL_WITHDRAW):
        # Amount to withdraw in sats between min/max withdrawable amounts
        amount_sats = 5_000
        withdraw_request = input_type[0]
        optional_completion_timeout_secs = 30

        request = LnurlWithdrawRequest(
            amount_sats=amount_sats,
            withdraw_request=withdraw_request,
            completion_timeout_secs=optional_completion_timeout_secs,
        )
        response = await sdk.lnurl_withdraw(request=request)


        payment = response.payment
        logging.debug(f"Payment: {payment}")
        return response
except Exception as error:
    logging.error(error)
    raise
```

## Go

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