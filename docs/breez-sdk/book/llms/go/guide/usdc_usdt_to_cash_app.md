# USDC/USDT to Cash App

`BridgeToCashApp` lets someone holding USDC or USDT on an external chain pay a Cash App user. The payer deposits the stablecoin, and the cross-chain provider pays the recipient in Bitcoin over Lightning. See [Cash App and USDC/USDT](./cash_app.md) for what the two Cash App bridges share.

## How it works

1. [Select a source route](./cash_app.md#selecting-a-route) with `CrossChainRouteFilterReceive`.
2. Call `BridgeToCashApp` with the recipient, the route, the USD amount and the payer's refund address. The SDK checks the recipient and gets a quote from the cross-chain provider.
3. Show the payer the returned `PaymentRequest`. Once their deposit lands, the provider pays the recipient.

```go
// List the stablecoin sources that can pay a Cash App user over Lightning
// and pick one, e.g. USDC on Base.
deliveryMethod := breez_sdk_spark.DeliveryMethodLightning
filter := breez_sdk_spark.CrossChainRouteFilterReceive{
	ContractAddress: nil,
	DeliveryMethod:  &deliveryMethod,
}
routes, err := sdk.GetCrossChainRoutes(filter)
if err != nil {
	return nil, err
}
var route *breez_sdk_spark.CrossChainRoutePair
for i := range routes {
	if routes[i].Asset == "USDC" && routes[i].Chain == "base" {
		route = &routes[i]
		break
	}
}
if route == nil {
	return nil, errors.New("no USDC route on Base")
}

// Pay $10 of USDC to the Cash App user $alice. The amount is in the route
// asset's base units (USDC, 6 decimals), so 10_000_000 = 10 USDC, about $10.
// The deposit is refunded to the payer's address if delivery fails.
request := breez_sdk_spark.BridgeToCashAppRequest{
	Recipient:      "$alice",
	Route:          *route,
	Amount:         new(big.Int).SetInt64(10_000_000),
	FeePolicy:      nil,
	RefundAddress:  "<payer address>",
	MaxSlippageBps: nil,
}
response, err := sdk.BridgeToCashApp(request)
if err != nil {
	return nil, err
}

// Show the payer what to pay. The recipient then receives Bitcoin.
info := response.Info
log.Printf("Pay with: %s", response.PaymentRequest)
log.Printf(
	"Deposit %v to %s, recipient receives ~%v sats",
	info.DepositAmount, info.DepositAddress, info.ExpectedReceivedAmount,
)
```



## Recipient

The `Recipient` is a Cash App username, given as `alice` or `$alice`, or the user's Cash App Lightning address, `alice@cash.app`. All three reach the same user, who is paid at that Lightning address.

The SDK looks the recipient up before quoting, so an unknown username fails with `SdkErrorInvalidInput` before the payer funds anything. So does an amount outside what the recipient accepts.

## Refund address

The `RefundAddress` is the payer's own address on the source chain. If the recipient can't be paid, the provider refunds the deposit there. It must belong to the route's chain family (e.g. an EVM `0x...` address for USDC on Base).

## Response fields

`BridgeToCashAppResponse` carries what the payer pays and the quote:

| Field | Meaning |
| ----- | ------- |
| `PaymentRequest` | What the payer pays: an EIP-681 URI on EVM chains, otherwise the bare deposit address. |
| `Info.DepositAddress` | The deposit address on the source chain. |
| `Info.DepositAmount` | The amount the payer deposits, in the route asset's base units. |
| `Info.ExpectedReceivedAmount` | The sats the recipient is expected to receive. |
| `Info.ServiceFeeAmount` | Provider fee, including your [partner fee](./cross_chain_partner_fees.md) if you set one, in `Info.ServiceFeeAsset` units. |
| `Info.ExpiresAt` | Quote expiry, in seconds since the Unix epoch. Call `BridgeToCashApp` again for a fresh quote if it lapses before the payer deposits. |

## Limitations

- **Delivery can still fail after funding.** The provider looks the recipient up again when it pays. If the account was closed, its limit lowered, or slippage moved the amount across a limit since the quote, the payment fails and the deposit is refunded to `RefundAddress`.
- **Lightning delivery only.** A completed bridge means the Lightning payment was delivered. Whether Cash App credits it as Bitcoin or converts it to dollars depends on the recipient's Cash App settings.
