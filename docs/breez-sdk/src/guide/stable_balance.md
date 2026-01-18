# Stable balance

The stable balance feature enables users to automatically convert received Bitcoin to a stable token, protecting against Bitcoin price volatility. This is ideal for users who want to receive Bitcoin payments but prefer to hold their value in a stable asset like a USD-pegged stablecoin.

## How it works

When stable balance is configured, the SDK automatically monitors your sats balance after each wallet sync. When your sats balance exceeds the configured threshold plus the reserved amount, the SDK automatically converts the excess sats (above the reserve) to the specified stable token using [token conversions](./token_conversion.md).

This creates a seamless experience where:

1. You can receive payments in any format (Lightning, Spark, on-chain deposits)
2. The SDK automatically converts any received sats to your chosen stable token
3. Your balance remains stable in value, denominated in the stable token
4. A Bitcoin reserve is maintained for sending payments without conversion

## Configuration

To enable stable balance, configure the `stable balance config` when initializing the SDK with the following options:
- **Token Identifier** - The identifier of the stable token to convert Bitcoin to.
- **Threshold Sats** - Optional minimum sats balance to trigger auto-conversion. Defaults to the conversion limit minimum if not specified.
- **Maximum Slippage** - Optional maximum slippage in basis points. Defaults to 50 bps (0.5%).
- **Reserved Sats** - Optional amount of sats to keep as Bitcoin and not convert. Defaults to the conversion limit minimum if not specified. This reserve ensures you can send some Bitcoin payments that fall below the conversion limit minimum.

**Example**: With a `threshold sats` of 2000 and a `reserved sats` of 1000:
- Auto-conversion triggers when balance exceeds 3000 sats (threshold + reserve)
- Only the amount above 1000 sats is converted
- You always maintain ~1000 sats for direct Bitcoin payments

{{#tabs config:stable-balance-config}}

<div class="warning">
<h4>Developer note</h4>

If the configured `threshold sats` is lower than the minimum amount required by the conversion protocol, the protocol minimum will be used instead. This ensures conversions always meet the minimum requirements.

</div>

## Sending payments with stable balance

When your balance is held in a stable token, you can still send Bitcoin payments by using [token conversions](./token_conversion.md#token-to-bitcoin). The SDK will convert the necessary amount of tokens to Bitcoin and then complete the payment.

For example, to pay a Lightning invoice when your balance is in USDB:

{{#tabs send_payment:prepare-send-payment-with-conversion}}

This allows you to:

- Pay any Lightning invoice
- Pay to any Bitcoin address
- Send to Spark addresses

All while keeping your balance in the stable token.

## Related pages

- [Token conversion](./token_conversion.md) - Learn about converting between Bitcoin and tokens
- [Custom configuration](./config.md#stable-balance-configuration) - All configuration options
- [Handling tokens](./tokens.md) - Working with tokens in the SDK
