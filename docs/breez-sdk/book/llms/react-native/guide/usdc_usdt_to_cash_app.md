# USDC/USDT to Cash App

`bridgeToCashApp` lets someone holding USDC or USDT on an external chain pay a Cash App user. The payer deposits the stablecoin, and the cross-chain provider pays the recipient in Bitcoin over Lightning. See [Cash App and USDC/USDT](./cash_app.md) for what the two Cash App bridges share.

## How it works

1. [Select a source route](./cash_app.md#selecting-a-route) with `CrossChainRouteFilter.Receive`.
2. Call `bridgeToCashApp` with the recipient, the route, the USD amount and the payer's refund address. The SDK checks the recipient and gets a quote from the cross-chain provider.
3. Show the payer the returned `paymentRequest`. Once their deposit lands, the provider pays the recipient.

```typescript
// List the stablecoin sources that can pay a Cash App user over Lightning
// and pick one, e.g. USDC on Base.
const routes = await sdk.getCrossChainRoutes(
  new CrossChainRouteFilter.Receive({
    contractAddress: undefined,
    deliveryMethod: DeliveryMethod.Lightning
  })
)
const route = routes.find((r) => r.asset === 'USDC' && r.chain === 'base')
if (route === undefined) {
  throw new Error('No USDC route on Base')
}

// Pay $10 of USDC to the Cash App user $alice. The amount is in the route
// asset's base units (USDC, 6 decimals), so 10_000_000 = 10 USDC, about $10.
// The deposit is refunded to the payer's address if delivery fails.
const response = await sdk.bridgeToCashApp({
  recipient: '$alice',
  route,
  amount: BigInt(10_000_000),
  feePolicy: undefined,
  refundAddress: '<payer address>',
  maxSlippageBps: undefined
})

// Show the payer what to pay. The recipient then receives Bitcoin.
const info = response.info
console.log(`Pay with: ${response.paymentRequest}`)
console.log(
  `Deposit ${info.depositAmount} to ${info.depositAddress}, ` +
    `recipient receives ~${info.expectedReceivedAmount} sats`
)
```



## Recipient

The `recipient` is a Cash App username, given as `alice` or `$alice`, or the user's Cash App Lightning address, `alice@cash.app`. All three reach the same user, who is paid at that Lightning address.

The SDK looks the recipient up before quoting, so an unknown username fails with `SdkError.InvalidInput` before the payer funds anything. So does an amount outside what the recipient accepts.

## Refund address

The `refundAddress` is the payer's own address on the source chain. If the recipient can't be paid, the provider refunds the deposit there. It must belong to the route's chain family (e.g. an EVM `0x...` address for USDC on Base).

## Response fields

`BridgeToCashAppResponse` carries what the payer pays and the quote:

| Field | Meaning |
| ----- | ------- |
| `paymentRequest` | What the payer pays: an EIP-681 URI on EVM chains, otherwise the bare deposit address. |
| `info.depositAddress` | The deposit address on the source chain. |
| `info.depositAmount` | The amount the payer deposits, in the route asset's base units. |
| `info.expectedReceivedAmount` | The sats the recipient is expected to receive. |
| `info.serviceFeeAmount` | Provider fee, including your [partner fee](./cross_chain_partner_fees.md) if you set one, in `info.serviceFeeAsset` units. |
| `info.expiresAt` | Quote expiry, in seconds since the Unix epoch. Call `bridgeToCashApp` again for a fresh quote if it lapses before the payer deposits. |

## Limitations

- **Delivery can still fail after funding.** The provider looks the recipient up again when it pays. If the account was closed, its limit lowered, or slippage moved the amount across a limit since the quote, the payment fails and the deposit is refunded to `refundAddress`.
- **Lightning delivery only.** A completed bridge means the Lightning payment was delivered. Whether Cash App credits it as Bitcoin or converts it to dollars depends on the recipient's Cash App settings.
