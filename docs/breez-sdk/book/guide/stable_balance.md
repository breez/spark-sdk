# Stable Balance

The stable balance feature enables users to switch between bitcoin and a stablecoin, like <a href="https://sparkscan.io/token/3206c93b24a4d18ea19d0a9a213204af2c7e74a6d16c7535cc5d33eca4ad1eca?network=mainnet" target="_blank">USDB</a>, protecting against bitcoin price volatility. On receive, sats are automatically converted to the stablecoin. On send, the stablecoin is converted to Bitcoin.

## How it works

When Stable Balance is configured and activated, for example with <a href="https://sparkscan.io/token/3206c93b24a4d18ea19d0a9a213204af2c7e74a6d16c7535cc5d33eca4ad1eca?network=mainnet" target="_blank">USDB</a>, the SDK manages conversions in both directions using [token conversions](./token_conversion.md):

- **On receive** — When you receive a payment (Lightning, Spark, or on-chain), the SDK converts the incoming sats to USDB once your sats balance exceeds the configured threshold.
- **On send** — When you send a bitcoin payment and your sats balance is insufficient, the SDK converts USDB back to bitcoin to cover the payment. See [Sending payments with stable balance](#sending-payments-with-stable-balance) for more details.

Your balance remains stable in value, denominated in USD.

## Configuration

To enable stable balance, configure the [stable balance config](./config.md#stable-balance-configuration) when initializing the SDK:

- **Tokens** — The stablecoin to use. Specify its token identifier and a display label.
- **Default Active Label** — Optional label to activate by default. If unset, Stable Balance starts deactivated and can be activated at runtime via [user settings](./user_settings.md).
- **Threshold Sats** — Optional minimum sats balance to trigger automatic conversion. We recommend omitting this to use the conversion limit minimum.
- **Maximum Slippage** — Optional maximum slippage in basis points. We recommend omitting this to use the default of 10 bps (0.1%).

### Rust

```rust
let mut config = default_config(Network::Mainnet);

// Enable stable balance with USDB conversion
config.stable_balance_config = Some(StableBalanceConfig {
    tokens: vec![StableBalanceToken {
        label: "USDB".to_string(),
        token_identifier: "btkn1xgrvjwey5ngcagvap2dzzvsy4uk8ua9x69k82dwvt5e7ef9drm9qztux87"
            .to_string(),
    }],
    default_active_label: Some("USDB".to_string()),
    threshold_sats: None,
    max_slippage_bps: None,
});
```

### Swift

```swift
var config = defaultConfig(network: Network.mainnet)

// Enable stable balance with USDB conversion
config.stableBalanceConfig = StableBalanceConfig(
    tokens: [StableBalanceToken(
        label: "USDB",
        tokenIdentifier: "btkn1xgrvjwey5ngcagvap2dzzvsy4uk8ua9x69k82dwvt5e7ef9drm9qztux87"
    )],
    defaultActiveLabel: "USDB"
)
```

### Kotlin

```kotlin
val config = defaultConfig(Network.MAINNET)

// Enable stable balance with USDB conversion
config.stableBalanceConfig = StableBalanceConfig(
    tokens = listOf(StableBalanceToken(
        label = "USDB",
        tokenIdentifier = "btkn1xgrvjwey5ngcagvap2dzzvsy4uk8ua9x69k82dwvt5e7ef9drm9qztux87",
    )),
    defaultActiveLabel = "USDB",
)
```

### C#

```csharp
var config = BreezSdkSparkMethods.DefaultConfig(Network.Mainnet) with
{
    // Enable stable balance with USDB conversion
    stableBalanceConfig = new StableBalanceConfig(
        tokens: new StableBalanceToken[] {
            new StableBalanceToken(
                label: "USDB",
                tokenIdentifier: "btkn1xgrvjwey5ngcagvap2dzzvsy4uk8ua9x69k82dwvt5e7ef9drm9qztux87"
            )
        },
        defaultActiveLabel: "USDB"
    )
};
```

### Javascript (Wasm)

```typescript
const config = defaultConfig('mainnet')

// Enable stable balance with USDB conversion
config.stableBalanceConfig = {
  tokens: [{
    label: 'USDB',
    tokenIdentifier: 'btkn1xgrvjwey5ngcagvap2dzzvsy4uk8ua9x69k82dwvt5e7ef9drm9qztux87'
  }],
  defaultActiveLabel: 'USDB'
}
```

### React Native

```typescript
const config = defaultConfig(Network.Mainnet)

// Enable stable balance with USDB conversion
config.stableBalanceConfig = {
  tokens: [{
    label: 'USDB',
    tokenIdentifier: 'btkn1xgrvjwey5ngcagvap2dzzvsy4uk8ua9x69k82dwvt5e7ef9drm9qztux87'
  }],
  defaultActiveLabel: 'USDB',
  thresholdSats: undefined,
  maxSlippageBps: undefined
}
```

### Flutter

```dart
var config = defaultConfig(network: Network.mainnet).copyWith(
    // Enable stable balance with USDB conversion
    stableBalanceConfig: StableBalanceConfig(
        tokens: [StableBalanceToken(
          label: "USDB",
          tokenIdentifier: "btkn1xgrvjwey5ngcagvap2dzzvsy4uk8ua9x69k82dwvt5e7ef9drm9qztux87",
        )],
        defaultActiveLabel: "USDB",
        ));
```

### Python

```python
config = default_config(network=Network.MAINNET)

# Enable stable balance with USDB conversion
config.stable_balance_config = StableBalanceConfig(
    tokens=[StableBalanceToken(
        label="USDB",
        token_identifier="btkn1xgrvjwey5ngcagvap2dzzvsy4uk8ua9x69k82dwvt5e7ef9drm9qztux87",
    )],
    default_active_label="USDB",
)
```

### Go

```go
config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)

// Enable stable balance with USDB conversion
defaultActiveLabel := "USDB"
stableBalanceConfig := breez_sdk_spark.StableBalanceConfig{
	Tokens: []breez_sdk_spark.StableBalanceToken{
		{
			Label:           "USDB",
			TokenIdentifier: "btkn1xgrvjwey5ngcagvap2dzzvsy4uk8ua9x69k82dwvt5e7ef9drm9qztux87",
		},
	},
	DefaultActiveLabel: &defaultActiveLabel,
}
config.StableBalanceConfig = &stableBalanceConfig
```



**Developer note**

If the configured `threshold sats` is lower than the minimum amount required by the conversion protocol, the protocol minimum will be used instead. This ensures conversions always meet the minimum requirements.

## Switching Stable Balance mode

You can activate, switch, or deactivate Stable Balance at runtime using the [user settings](./user_settings.md) API. This allows users to choose when to enable Stable Balance and which stablecoin to use.

### Activating Stable Balance

To activate Stable Balance, set the active label to one of the labels defined in your #{{name StableBalanceConfig.tokens}} list:

#### Rust

```rust
sdk.update_user_settings(UpdateUserSettingsRequest {
    spark_private_mode_enabled: None,
    stable_balance_active_label: Some(StableBalanceActiveLabel::Set {
        label: "USDB".to_string(),
    }),
    spark_master_identity_public_key: None,
})
.await?;
```

#### Swift

```swift
try await sdk.updateUserSettings(
    request: UpdateUserSettingsRequest(
        sparkPrivateModeEnabled: nil,
        stableBalanceActiveLabel: .set(label: "USDB")
    ))
```

#### Kotlin

```kotlin
try {
    sdk.updateUserSettings(UpdateUserSettingsRequest(
        sparkPrivateModeEnabled = null,
        stableBalanceActiveLabel = StableBalanceActiveLabel.Set(label = "USDB")
    ))
} catch (e: Exception) {
    // handle error
}
```

#### C#

```csharp
await sdk.UpdateUserSettings(
    request: new UpdateUserSettingsRequest(
        sparkPrivateModeEnabled: null,
        stableBalanceActiveLabel: new StableBalanceActiveLabel.Set(label: "USDB")
    )
);
```

#### Javascript (Wasm)

```typescript
await sdk.updateUserSettings({
  stableBalanceActiveLabel: { type: 'set', label: 'USDB' }
})
```

#### React Native

```typescript
await sdk.updateUserSettings({
  sparkPrivateModeEnabled: undefined,
  stableBalanceActiveLabel: new StableBalanceActiveLabel.Set({ label: 'USDB' }),
  sparkMasterIdentityPublicKey: undefined
})
```

#### Flutter

```dart
await sdk.updateUserSettings(
    request: UpdateUserSettingsRequest(
        stableBalanceActiveLabel: StableBalanceActiveLabel_Set(label: "USDB")));
```

#### Python

```python
try:
    await sdk.update_user_settings(
        request=UpdateUserSettingsRequest(
            spark_private_mode_enabled=None,
            stable_balance_active_label=StableBalanceActiveLabel.SET(label="USDB")
        )
    )
except Exception as error:
    logging.error(error)
    raise
```

#### Go

```go
activeLabel := breez_sdk_spark.StableBalanceActiveLabel(
	breez_sdk_spark.StableBalanceActiveLabelSet{Label: "USDB"},
)
err := sdk.UpdateUserSettings(breez_sdk_spark.UpdateUserSettingsRequest{
	StableBalanceActiveLabel: &activeLabel,
})

if err != nil {
	return err
}
```



When activated, the SDK immediately converts any excess sats balance to the specified token.

### Deactivating Stable Balance

To deactivate Stable Balance, unset the active label:

#### Rust

```rust
sdk.update_user_settings(UpdateUserSettingsRequest {
    spark_private_mode_enabled: None,
    stable_balance_active_label: Some(StableBalanceActiveLabel::Unset),
    spark_master_identity_public_key: None,
})
.await?;
```

#### Swift

```swift
try await sdk.updateUserSettings(
    request: UpdateUserSettingsRequest(
        sparkPrivateModeEnabled: nil,
        stableBalanceActiveLabel: .unset
    ))
```

#### Kotlin

```kotlin
try {
    sdk.updateUserSettings(UpdateUserSettingsRequest(
        sparkPrivateModeEnabled = null,
        stableBalanceActiveLabel = StableBalanceActiveLabel.Unset
    ))
} catch (e: Exception) {
    // handle error
}
```

#### C#

```csharp
await sdk.UpdateUserSettings(
    request: new UpdateUserSettingsRequest(
        sparkPrivateModeEnabled: null,
        stableBalanceActiveLabel: new StableBalanceActiveLabel.Unset()
    )
);
```

#### Javascript (Wasm)

```typescript
await sdk.updateUserSettings({
  stableBalanceActiveLabel: { type: 'unset' }
})
```

#### React Native

```typescript
await sdk.updateUserSettings({
  sparkPrivateModeEnabled: undefined,
  stableBalanceActiveLabel: new StableBalanceActiveLabel.Unset(),
  sparkMasterIdentityPublicKey: undefined
})
```

#### Flutter

```dart
await sdk.updateUserSettings(
    request: UpdateUserSettingsRequest(
        stableBalanceActiveLabel: StableBalanceActiveLabel_Unset()));
```

#### Python

```python
try:
    await sdk.update_user_settings(
        request=UpdateUserSettingsRequest(
            spark_private_mode_enabled=None,
            stable_balance_active_label=StableBalanceActiveLabel.UNSET()
        )
    )
except Exception as error:
    logging.error(error)
    raise
```

#### Go

```go
activeLabel := breez_sdk_spark.StableBalanceActiveLabel(
	breez_sdk_spark.StableBalanceActiveLabelUnset{},
)
err := sdk.UpdateUserSettings(breez_sdk_spark.UpdateUserSettingsRequest{
	StableBalanceActiveLabel: &activeLabel,
})

if err != nil {
	return err
}
```



When deactivated, the SDK converts any remaining token balance back to Bitcoin.

### Checking the current mode

You can check which token is currently active using `get_user_settings`:

#### Rust

```rust
let user_settings = sdk.get_user_settings().await?;
info!("User settings: {:?}", user_settings);
```

#### Swift

```swift
let userSettings = try await sdk.getUserSettings()
print("User settings: \(userSettings)")
```

#### Kotlin

```kotlin
try {
    val userSettings = sdk.getUserSettings()
    println("User settings: $userSettings")
} catch (e: Exception) {
    // handle error
}
```

#### C#

```csharp
var userSettings = await sdk.GetUserSettings();

Console.WriteLine($"User settings: {userSettings}");
```

#### Javascript (Wasm)

```typescript
const userSettings = await sdk.getUserSettings()
console.log(`User settings: ${JSON.stringify(userSettings)}`)
```

#### React Native

```typescript
const userSettings = await sdk.getUserSettings()
console.log(`User settings: ${JSON.stringify(userSettings)}`)
```

#### Flutter

```dart
final userSettings = await sdk.getUserSettings();
print('User settings: $userSettings');
```

#### Python

```python
try:
    user_settings = await sdk.get_user_settings()

    print(f"User settings: {user_settings}")
except Exception as error:
    logging.error(error)
    raise
```

#### Go

```go
userSettings, err := sdk.GetUserSettings()

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return err
}

log.Printf("User settings: %v", userSettings)
```



The `stable_balance_active_label` field will be unset if Stable Balance is deactivated, or the label of the currently active token.

## Sending payments with stable balance

When your balance is held in a stablecoin, you can still send bitcoin payments. The SDK detects when there's not enough bitcoin balance to cover a payment and sets up the token-to-bitcoin conversion for you.

When you [prepare to send a payment](./send_payment.md#preparing-payments) without specifying conversion options:
1. If you have enough bitcoin balance, no conversion is needed
2. If your bitcoin balance is insufficient, the SDK configures conversion options using your Stable Balance settings (token identifier and slippage)

The same flow extends to [USDC/USDT](./send_payment.md#usdc-usdt): when paying a recipient on USDC or USDT, the SDK can spend the user's USDB balance directly (Orchestra routes that accept USDB as source) or auto-convert it through bitcoin if the chosen route only accepts sats.

**Developer note**

You can still explicitly specify `conversion options` in your request if you need custom slippage settings or want to override the default behavior.

## Sending entire balance

When Stable Balance is active, you can send your entire balance, both the token balance and any remaining bitcoin, in a single payment. 

To send all, provide the full token balance as the amount along with `FeePolicy::FeesIncluded` and `ConversionType::ToBitcoin` conversion options. The SDK converts all specified tokens to bitcoin, combines the result with any existing bitcoin balance, and deducts payment fees from the total.

The prepare response returns the estimated total Bitcoin available after conversion, and includes a `conversion_estimate` with the conversion details.

The same approach works with `prepare_lnurl_pay` for [LNURL payments](./lnurl_pay.md).

### Rust

```rust
let payment_request = "<payment request>".to_string();
let token_identifier = "<token identifier>".to_string();

let info = sdk
    .get_info(GetInfoRequest {
        ensure_synced: Some(false),
    })
    .await?;

let token_balance = info
    .token_balances
    .get(&token_identifier)
    .ok_or_else(|| anyhow::anyhow!("Token balance not found"))?;

let conversion_options = Some(ConversionOptions {
    conversion_type: ConversionType::ToBitcoin {
        from_token_identifier: token_identifier.clone(),
    },
    max_slippage_bps: None,
    completion_timeout_secs: None,
});

let prepare_response = sdk
    .prepare_send_payment(PrepareSendPaymentRequest {
        payment_request: PaymentRequest::Input {
            input: payment_request,
        },
        amount: Some(token_balance.balance),
        token_identifier: Some(token_identifier),
        conversion_options,
        fee_policy: Some(FeePolicy::FeesIncluded),
    })
    .await?;

// The response amount is the estimated total sats available
// (converted sats + existing sat balance)
info!("Total sats available: {}", prepare_response.amount);

if let Some(conversion_estimate) = &prepare_response.conversion_estimate {
    info!(
        "Converting {} token units → ~{} sats",
        conversion_estimate.amount_in, conversion_estimate.amount_out
    );
    info!("Conversion fee: {} token units", conversion_estimate.fee);
}
```

### Swift

```swift
let paymentRequest = "<payment request>"
let tokenIdentifier = "<token identifier>"

let info = try await sdk.getInfo(
    request: GetInfoRequest(ensureSynced: false))

guard let tokenBalance = info.tokenBalances[tokenIdentifier] else {
    throw SdkError.InvalidInput("Token balance not found")
}

let conversionOptions = ConversionOptions(
    conversionType: ConversionType.toBitcoin(
        fromTokenIdentifier: tokenIdentifier
    ),
    maxSlippageBps: nil,
    completionTimeoutSecs: nil
)

let prepareResponse = try await sdk.prepareSendPayment(
    request: PrepareSendPaymentRequest(
        paymentRequest: .input(input: paymentRequest),
        amount: tokenBalance.balance,
        tokenIdentifier: tokenIdentifier,
        conversionOptions: conversionOptions,
        feePolicy: .feesIncluded
    ))

// The response amount is the estimated total sats available
// (converted sats + existing sat balance)
print("Total sats available: \(prepareResponse.amount)")

if let conversionEstimate = prepareResponse.conversionEstimate {
    print(
        "Converting \(conversionEstimate.amountIn) token units "
            + "→ ~\(conversionEstimate.amountOut) sats")
    print("Conversion fee: \(conversionEstimate.fee) token units")
}
```

### Kotlin

```kotlin
val paymentRequest = "<payment request>"
val tokenIdentifier = "<token identifier>"

try {
    val info = sdk.getInfo(GetInfoRequest(false))
    val tokenBalance = info.tokenBalances[tokenIdentifier]
        ?: throw Exception("Token balance not found")

    val conversionOptions = ConversionOptions(
        conversionType = ConversionType.ToBitcoin(
            tokenIdentifier
        ),
        maxSlippageBps = null,
        completionTimeoutSecs = null
    )

    val req = PrepareSendPaymentRequest(
        paymentRequest = PaymentRequest.Input(input = paymentRequest),
        amount = tokenBalance.balance,
        tokenIdentifier = tokenIdentifier,
        conversionOptions = conversionOptions,
        feePolicy = FeePolicy.FEES_INCLUDED,
    )
    val prepareResponse = sdk.prepareSendPayment(req)

    // The response amount is the estimated total sats available
    // (converted sats + existing sat balance)
    // Log.v("Breez", "Total sats available: ${prepareResponse.amount}")

    prepareResponse.conversionEstimate?.let { conversionEstimate ->
        // Log.v("Breez", "Converting ${conversionEstimate.amountIn} token
        // units → ~${conversionEstimate.amountOut} sats")
        // Log.v("Breez", "Conversion fee: ${conversionEstimate.fee} token units")
    }
} catch (e: Exception) {
    // handle error
}
```

### C#

```csharp
var paymentRequest = "<payment request>";
var tokenIdentifier = "<token identifier>";

var info = await sdk.GetInfo(request: new GetInfoRequest(ensureSynced: false));
if (!info.tokenBalances.TryGetValue(tokenIdentifier, out var tokenBalance))
{
    throw new Exception("Token balance not found");
}

var conversionOptions = new ConversionOptions(
    conversionType: new ConversionType.ToBitcoin(
        fromTokenIdentifier: tokenIdentifier
    ),
    maxSlippageBps: null,
    completionTimeoutSecs: null
);

var request = new PrepareSendPaymentRequest(
    paymentRequest: new PaymentRequest.Input(input: paymentRequest),
    amount: tokenBalance.balance,
    tokenIdentifier: tokenIdentifier,
    conversionOptions: conversionOptions,
    feePolicy: FeePolicy.FeesIncluded
);
var prepareResponse = await sdk.PrepareSendPayment(request: request);

// The response amount is the estimated total sats available
// (converted sats + existing sat balance)
Console.WriteLine($"Total sats available: {prepareResponse.amount}");

if (prepareResponse.conversionEstimate != null)
{
    Console.WriteLine("Converting " +
        $"{prepareResponse.conversionEstimate.amountIn} token units " +
        $"→ ~{prepareResponse.conversionEstimate.amountOut} sats");
    Console.WriteLine("Conversion fee: " +
        $"{prepareResponse.conversionEstimate.fee} token units");
}
```

### Javascript (Wasm)

```typescript
const paymentRequest = '<payment request>'
const tokenIdentifier = '<token identifier>'

const info = await sdk.getInfo({ ensureSynced: false })
const tokenBalance = info.tokenBalances.get(tokenIdentifier)
if (tokenBalance === undefined) {
  throw new Error('Token balance not found')
}

const conversionOptions: ConversionOptions = {
  conversionType: {
    type: 'toBitcoin',
    fromTokenIdentifier: tokenIdentifier
  }
}
const feePolicy: FeePolicy = 'feesIncluded'

const prepareResponse = await sdk.prepareSendPayment({
  paymentRequest: { type: 'input', input: paymentRequest },
  amount: tokenBalance.balance,
  tokenIdentifier,
  conversionOptions,
  feePolicy
})

// The response amount is the estimated total sats available
// (converted sats + existing sat balance)
console.log(`Total sats available: ${prepareResponse.amount}`)

if (prepareResponse.conversionEstimate !== undefined) {
  const estimate = prepareResponse.conversionEstimate
  console.log(`Converting ${estimate.amountIn} token units → ~${estimate.amountOut} sats`)
  console.log(`Conversion fee: ${estimate.fee} token units`)
}
```

### React Native

```typescript
const paymentRequest = '<payment request>'
const tokenIdentifier = '<token identifier>'

const info = await sdk.getInfo({ ensureSynced: false })
const tokenBalance = info.tokenBalances.get(tokenIdentifier)
if (tokenBalance === undefined) {
  throw new Error('Token balance not found')
}

const conversionOptions = {
  conversionType: new ConversionType.ToBitcoin({
    fromTokenIdentifier: tokenIdentifier
  }),
  maxSlippageBps: undefined,
  completionTimeoutSecs: undefined
}

const prepareResponse = await sdk.prepareSendPayment({
  paymentRequest: new PaymentRequest.Input({ input: paymentRequest }),
  amount: tokenBalance.balance,
  tokenIdentifier,
  conversionOptions,
  feePolicy: FeePolicy.FeesIncluded
})

// The response amount is the estimated total sats available
// (converted sats + existing sat balance)
console.log(`Total sats available: ${prepareResponse.amount}`)

if (prepareResponse.conversionEstimate !== undefined) {
  const estimate = prepareResponse.conversionEstimate
  console.log(`Converting ${estimate.amountIn} token units → ~${estimate.amountOut} sats`)
  console.log(`Conversion fee: ${estimate.fee} token units`)
}
```

### Flutter

```dart
String paymentRequest = "<payment request>";
String tokenIdentifier = "<token identifier>";

final info = await sdk.getInfo(request: GetInfoRequest(ensureSynced: false));
final tokenBalance = info.tokenBalances[tokenIdentifier];
if (tokenBalance == null) {
  throw Exception("Token balance not found");
}

final conversionOptions = ConversionOptions(
  conversionType: ConversionType.toBitcoin(
    fromTokenIdentifier: tokenIdentifier,
  ),
);

final request = PrepareSendPaymentRequest(
    paymentRequest: PaymentRequest.input(input: paymentRequest),
    amount: tokenBalance.balance,
    tokenIdentifier: tokenIdentifier,
    conversionOptions: conversionOptions,
    feePolicy: FeePolicy.feesIncluded);
final response = await sdk.prepareSendPayment(request: request);

// The response amount is the estimated total sats available
// (converted sats + existing sat balance)
print("Total sats available: ${response.amount}");

if (response.conversionEstimate != null) {
  print(
      "Converting ${response.conversionEstimate!.amountIn} token units "
      "→ ~${response.conversionEstimate!.amountOut} sats");
  print(
      "Conversion fee: ${response.conversionEstimate!.fee} token units");
}
```

### Python

```python
payment_request = "<payment request>"
token_identifier = "<token identifier>"
try:
    info = await sdk.get_info(request=GetInfoRequest(ensure_synced=False))
    token_balance = info.token_balances.get(token_identifier)
    if token_balance is None:
        raise ValueError("Token balance not found")

    conversion_options = ConversionOptions(
        conversion_type=ConversionType.TO_BITCOIN(
            from_token_identifier=token_identifier
        ),
    )

    request = PrepareSendPaymentRequest(
        payment_request=PaymentRequest.INPUT(input=payment_request),
        amount=token_balance.balance,
        token_identifier=token_identifier,
        conversion_options=conversion_options,
        fee_policy=FeePolicy.FEES_INCLUDED,
    )
    prepare_response = await sdk.prepare_send_payment(request=request)

    # The response amount is the estimated total sats available
    # (converted sats + existing sat balance)
    logging.debug(f"Total sats available: {prepare_response.amount}")

    if prepare_response.conversion_estimate is not None:
        conversion_estimate = prepare_response.conversion_estimate
        logging.debug(
            f"Converting {conversion_estimate.amount_in}"
            f" token units → ~{conversion_estimate.amount_out} sats"
        )
        logging.debug(
            f"Conversion fee: {conversion_estimate.fee} token units"
        )
except Exception as error:
    logging.error(error)
    raise
```

### Go

```go
paymentRequest := "<payment request>"
tokenIdentifier := "<token identifier>"

ensureSynced := false
info, err := sdk.GetInfo(breez_sdk_spark.GetInfoRequest{
	EnsureSynced: &ensureSynced,
})
if err != nil {
	return nil, err
}

tokenBalance, ok := info.TokenBalances[tokenIdentifier]
if !ok {
	return nil, errors.New("token balance not found")
}

conversionOptions := breez_sdk_spark.ConversionOptions{
	ConversionType: breez_sdk_spark.ConversionTypeToBitcoin{
		FromTokenIdentifier: tokenIdentifier,
	},
}
feePolicy := breez_sdk_spark.FeePolicyFeesIncluded

request := breez_sdk_spark.PrepareSendPaymentRequest{
	PaymentRequest:    breez_sdk_spark.PaymentRequestInput{Input: paymentRequest},
	Amount:            &tokenBalance.Balance,
	TokenIdentifier:   &tokenIdentifier,
	ConversionOptions: &conversionOptions,
	FeePolicy:         &feePolicy,
}
response, err := sdk.PrepareSendPayment(request)

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

// The response amount is the estimated total sats available
// (converted sats + existing sat balance)
log.Printf("Total sats available: %v", response.Amount)

if response.ConversionEstimate != nil {
	log.Printf(
		"Converting %v token units → ~%v sats",
		response.ConversionEstimate.AmountIn,
		response.ConversionEstimate.AmountOut,
	)
	log.Printf("Conversion fee: %v token units", response.ConversionEstimate.Fee)
}
```



**Developer note**

The actual sats received from conversion may differ slightly from the estimate due to price movement. The SDK handles this by querying the actual balance after conversion completes and sending the full available amount.

## Conversion details

Payments involving token conversions include a `conversion_details` field that describes the conversion that took place. This is useful for displaying conversion context in your UI.

### Status

The `status` field tracks the lifecycle of the conversion:

| Status | Description |
|--------|-------------|
| `ConversionStatus::Pending` | Conversion is queued or in progress |
| `ConversionStatus::Completed` | Conversion finished successfully |
| `ConversionStatus::Failed` | Conversion could not be completed |
| `ConversionStatus::RefundNeeded` | Conversion failed and requires a refund |
| `ConversionStatus::Refunded` | Failed conversion has been refunded |

### Conversion steps

The `from` and `to` fields are conversion step objects describing each side of the conversion:

| Field | Description |
|-------|-------------|
| `payment_id` | The ID of the internal conversion payment |
| `amount` | The amount in the step's denomination (sats or token units) |
| `fee` | Fee charged for this step |
| `method` | Payment method (`PaymentMethod::Spark` for bitcoin, `PaymentMethod::Token` for stablecoins) |
| `token_metadata` | Token metadata (name, symbol, etc.) — present when method is `PaymentMethod::Token` |
| `amount_adjustment` | Present if the amount was modified before conversion (see [amount adjustments](#amount-adjustments)) |

### Amount adjustments

The `amount_adjustment` field is present when the conversion amount was modified before execution:

| Reason | Description |
|--------|-------------|
| `AmountAdjustmentReason::FlooredToMinLimit` | Amount was increased to meet the minimum conversion limit |
| `AmountAdjustmentReason::IncreasedToAvoidDust` | Amount was increased to convert the entire remaining balance, avoiding a leftover too small to convert back |

## Related pages

- [Token conversion](./token_conversion.md) - Learn about converting between Bitcoin and tokens
- [Custom configuration](./config.md#stable-balance-configuration) - All configuration options
- [User settings](./user_settings.md) - Getting and updating user settings
- [Handling tokens](./tokens.md) - Working with tokens in the SDK

---

Identifier casing: `get_info` here is `getInfo` in Swift, Kotlin, JavaScript, React Native and Flutter, and `GetInfo` in Go and C#. Enum variants: `SdkEvent::Synced` is `SdkEvent.SYNCED` in Python, `SdkEvent.synced` in Swift, `SdkEventSynced` in Go, and `SdkEvent.Synced` elsewhere.
