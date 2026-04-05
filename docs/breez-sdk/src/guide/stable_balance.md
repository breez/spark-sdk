# Stable balance

The stable balance feature enables users to convert between Bitcoin and a stable token, like <a href="https://sparkscan.io/token/3206c93b24a4d18ea19d0a9a213204af2c7e74a6d16c7535cc5d33eca4ad1eca?network=mainnet" target="_blank">USDB</a>, protecting against Bitcoin price volatility. On receive, sats are converted to the stable token. On send, the stable token is converted back to Bitcoin.

## How it works

When stable balance is configured and activated, for example with <a href="https://sparkscan.io/token/3206c93b24a4d18ea19d0a9a213204af2c7e74a6d16c7535cc5d33eca4ad1eca?network=mainnet" target="_blank">USDB</a>, the SDK manages conversions in both directions using [token conversions](./token_conversion.md):

- **On receive** — When you receive a payment (Lightning, Spark, or on-chain), the SDK converts the incoming sats to USDB once your sats balance exceeds the configured threshold.
- **On send** — When you send a Bitcoin payment and your sats balance is insufficient, the SDK converts USDB back to Bitcoin to cover the payment. See [Sending payments with stable balance](#sending-payments-with-stable-balance) for more details.

Your balance remains stable in value, denominated in USDB.

## Configuration

To enable stable balance, configure the [stable balance config](./config.md#stable-balance-configuration) when initializing the SDK:

- **Tokens** — The stable token to use. Specify its token identifier and a display label.
- **Default Active Label** — Optional label to activate by default. If unset, stable balance starts deactivated and can be activated at runtime via [user settings](./user_settings.md).
- **Threshold Sats** — Optional minimum sats balance to trigger conversion. We recommend omitting this to use the conversion limit minimum.
- **Maximum Slippage** — Optional maximum slippage in basis points. We recommend omitting this to use the default of 10 bps (0.1%).

{{#tabs config:stable-balance-config}}

<div class="warning">
<h4>Developer note</h4>

If the configured `threshold sats` is lower than the minimum amount required by the conversion protocol, the protocol minimum will be used instead. This ensures conversions always meet the minimum requirements.

</div>

## Switching stable balance mode

You can activate, switch, or deactivate stable balance at runtime using the [user settings](./user_settings.md) API. This allows users to choose when to enable stable balance and which token to use.

### Activating stable balance

To activate stable balance, set the active label to one of the labels defined in your #{{name StableBalanceConfig.tokens}} list:

{{#tabs user_settings:activate-stable-balance}}

When activated, the SDK immediately converts any excess sats balance to the specified token.

### Deactivating stable balance

To deactivate stable balance, unset the active label:

{{#tabs user_settings:deactivate-stable-balance}}

When deactivated, the SDK converts any remaining token balance back to Bitcoin.

### Checking the current mode

You can check which token is currently active using {{#name get_user_settings}}:

{{#tabs user_settings:get-user-settings}}

The {{#name stable_balance_active_label}} field will be unset if stable balance is deactivated, or the label of the currently active token.

## Sending payments with stable balance

When your balance is held in a stable token, you can still send Bitcoin payments. The SDK detects when there's not enough Bitcoin balance to cover a payment and sets up the token-to-Bitcoin conversion for you.

When you [prepare to send a payment](./send_payment.md#preparing-payments) without specifying conversion options:
1. If you have enough Bitcoin balance, no conversion is needed
2. If your Bitcoin balance is insufficient, the SDK configures conversion options using your stable balance settings (token identifier and slippage)

<div class="warning">
<h4>Developer note</h4>

You can still explicitly specify `conversion options` in your request if you need custom slippage settings or want to override the default behavior.

</div>

## Sending entire balance

When stable balance is active, you can send your entire wallet balance — both the token balance and any remaining sats — in a single payment. This is useful for draining a wallet completely.

To send all, provide the full token balance as the amount along with {{#enum FeePolicy::FeesIncluded}} and {{#enum ConversionType::ToBitcoin}} conversion options. The SDK converts all specified tokens to Bitcoin, combines the result with any existing sat balance, and deducts payment fees from the total.

The prepare response returns the estimated total sats available after conversion, and includes a {{#name conversion_estimate}} with the conversion details.

The same approach works with {{#name prepare_lnurl_pay}} for [LNURL payments](./lnurl_pay.md).

{{#tabs send_payment:prepare-send-payment-send-all}}

<div class="warning">
<h4>Developer note</h4>

The actual sats received from conversion may differ slightly from the estimate due to price movement. The SDK handles this by querying the actual balance after conversion completes and sending the full available amount.

</div>

## Conversion details

Payments involving token conversions include a {{#name conversion_details}} field that describes the conversion that took place. This is useful for displaying conversion context in your UI.

### Status

The {{#name status}} field tracks the lifecycle of the conversion:

| Status | Description |
|--------|-------------|
| {{#enum ConversionStatus::Pending}} | Conversion is queued or in progress |
| {{#enum ConversionStatus::Completed}} | Conversion finished successfully |
| {{#enum ConversionStatus::Failed}} | Conversion could not be completed |
| {{#enum ConversionStatus::RefundNeeded}} | Conversion failed and requires a refund |
| {{#enum ConversionStatus::Refunded}} | Failed conversion has been refunded |

### Conversion steps

The {{#name from}} and {{#name to}} fields are conversion step objects describing each side of the conversion:

| Field | Description |
|-------|-------------|
| {{#name payment_id}} | The ID of the internal conversion payment |
| {{#name amount}} | The amount in the step's denomination (sats or token units) |
| {{#name fee}} | Fee charged for this step |
| {{#name method}} | Payment method ({{#enum PaymentMethod::Spark}} for Bitcoin, {{#enum PaymentMethod::Token}} for stable tokens) |
| {{#name token_metadata}} | Token metadata (name, symbol, etc.) — present when method is {{#enum PaymentMethod::Token}} |
| {{#name amount_adjustment}} | Present if the amount was modified before conversion (see [amount adjustments](#amount-adjustments)) |

### Amount adjustments

The {{#name amount_adjustment}} field is present when the conversion amount was modified before execution:

| Reason | Description |
|--------|-------------|
| {{#enum AmountAdjustmentReason::FlooredToMinLimit}} | Amount was increased to meet the minimum conversion limit |
| {{#enum AmountAdjustmentReason::IncreasedToAvoidDust}} | Amount was increased to convert the entire remaining balance, avoiding a leftover too small to convert back |

## Related pages

- [Token conversion](./token_conversion.md) - Learn about converting between Bitcoin and tokens
- [Custom configuration](./config.md#stable-balance-configuration) - All configuration options
- [User settings](./user_settings.md) - Getting and updating user settings
- [Handling tokens](./tokens.md) - Working with tokens in the SDK
