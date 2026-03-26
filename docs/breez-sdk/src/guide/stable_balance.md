# Stable balance

The stable balance feature enables users to automatically convert received Bitcoin to a stable token, protecting against Bitcoin price volatility. This is ideal for users who want to receive Bitcoin payments but prefer to hold their value in a stable asset like a USD-pegged stablecoin.

## How it works

When stable balance is configured and activated, the SDK automatically monitors your sats balance. When your sats balance exceeds the configured threshold, the SDK automatically converts the excess sats to the active stable token using [token conversions](./token_conversion.md).

This creates a seamless experience where:

1. You can receive payments in any format (Lightning, Spark, on-chain deposits)
2. The SDK automatically converts any received sats to your chosen stable token
3. Your balance remains stable in value, denominated in the stable token

## Configuration

To enable stable balance, configure the [stable balance config](./config.md#stable-balance-configuration) when initializing the SDK with the following options:
- **Tokens** - A list of available stable tokens, each with a display label and token identifier. Labels must be unique and are used to activate a specific token at runtime.
- **Default Active Label** - Optional label of the token to activate by default. If unset, stable balance starts deactivated and can be activated at runtime via [user settings](./user_settings.md).
- **Threshold Sats** - Optional minimum sats balance to trigger auto-conversion. Defaults to the conversion limit minimum if not specified.
- **Maximum Slippage** - Optional maximum slippage in basis points. Defaults to 50 bps (0.5%).

{{#tabs config:stable-balance-config}}

<div class="warning">
<h4>Developer note</h4>

If the configured `threshold sats` is lower than the minimum amount required by the conversion protocol, the protocol minimum will be used instead. This ensures conversions always meet the minimum requirements.

</div>

## Switching stable balance mode

You can activate, switch, or deactivate stable balance at runtime using the [user settings](./user_settings.md) API. This allows users to choose when to enable stable balance and which token to use.

### Activating stable balance

To activate stable balance, set the active label to one of the labels defined in your `#{{name StableBalanceConfig.tokens}}` list:

{{#tabs user_settings:activate-stable-balance}}

When activated, the SDK immediately converts any excess sats balance to the specified token.

### Deactivating stable balance

To deactivate stable balance, unset the active label:

{{#tabs user_settings:deactivate-stable-balance}}

When deactivated, the SDK automatically converts any remaining token balance back to Bitcoin.

### Checking the current mode

You can check which token is currently active using `{{#name get_user_settings}}`:

{{#tabs user_settings:get-user-settings}}

The `{{#name stable_balance_active_label}}` field will be unset if stable balance is deactivated, or the label of the currently active token.

## Sending payments with stable balance

When your balance is held in a stable token, you can still send Bitcoin payments. The SDK automatically detects when there's not enough Bitcoin balance to cover a payment and sets up the token-to-Bitcoin conversion for you.

When you [prepare to send a payment](./send_payment.md#preparing-payments) without specifying conversion options:
1. If you have enough Bitcoin balance, no conversion is needed
2. If your Bitcoin balance is insufficient, the SDK automatically configures conversion options using your stable balance settings (token identifier and slippage)

<div class="warning">
<h4>Developer note</h4>

You can still explicitly specify `conversion options` in your request if you need custom slippage settings or want to override the automatic behavior.

</div>

## Related pages

- [Token conversion](./token_conversion.md) - Learn about converting between Bitcoin and tokens
- [Custom configuration](./config.md#stable-balance-configuration) - All configuration options
- [User settings](./user_settings.md) - Getting and updating user settings
- [Handling tokens](./tokens.md) - Working with tokens in the SDK
