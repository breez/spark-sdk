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

You can check which token is currently active using `GetUserSettings`:

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



The `StableBalanceActiveLabel` field will be unset if Stable Balance is deactivated, or the label of the currently active token.

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

To send all, provide the full token balance as the amount along with `FeePolicyFeesIncluded` and `ConversionTypeToBitcoin` conversion options. The SDK converts all specified tokens to bitcoin, combines the result with any existing bitcoin balance, and deducts payment fees from the total.

The prepare response returns the estimated total Bitcoin available after conversion, and includes a `ConversionEstimate` with the conversion details.

The same approach works with `PrepareLnurlPay` for [LNURL payments](./lnurl_pay.md).

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

Payments involving token conversions include a `ConversionDetails` field that describes the conversion that took place. This is useful for displaying conversion context in your UI.

### Status

The `Status` field tracks the lifecycle of the conversion:

| Status | Description |
|--------|-------------|
| `ConversionStatusPending` | Conversion is queued or in progress |
| `ConversionStatusCompleted` | Conversion finished successfully |
| `ConversionStatusFailed` | Conversion could not be completed |
| `ConversionStatusRefundNeeded` | Conversion failed and requires a refund |
| `ConversionStatusRefunded` | Failed conversion has been refunded |

### Conversion steps

The `From` and `To` fields are conversion step objects describing each side of the conversion:

| Field | Description |
|-------|-------------|
| `PaymentId` | The ID of the internal conversion payment |
| `Amount` | The amount in the step's denomination (sats or token units) |
| `Fee` | Fee charged for this step |
| `Method` | Payment method (`PaymentMethodSpark` for bitcoin, `PaymentMethodToken` for stablecoins) |
| `TokenMetadata` | Token metadata (name, symbol, etc.) — present when method is `PaymentMethodToken` |
| `AmountAdjustment` | Present if the amount was modified before conversion (see [amount adjustments](#amount-adjustments)) |

### Amount adjustments

The `AmountAdjustment` field is present when the conversion amount was modified before execution:

| Reason | Description |
|--------|-------------|
| `AmountAdjustmentReasonFlooredToMinLimit` | Amount was increased to meet the minimum conversion limit |
| `AmountAdjustmentReasonIncreasedToAvoidDust` | Amount was increased to convert the entire remaining balance, avoiding a leftover too small to convert back |

## Related pages

- [Token conversion](./token_conversion.md) - Learn about converting between Bitcoin and tokens
- [Custom configuration](./config.md#stable-balance-configuration) - All configuration options
- [User settings](./user_settings.md) - Getting and updating user settings
- [Handling tokens](./tokens.md) - Working with tokens in the SDK
