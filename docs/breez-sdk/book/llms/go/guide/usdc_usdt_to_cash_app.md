# USDC/USDT to Cash App

`BridgeToCashApp` lets someone holding USDC or USDT on an external chain pay a Cash App user. The payer deposits the stablecoin, and the cross-chain provider pays the recipient in Bitcoin over Lightning. See [Cash App and USDC/USDT](./cash_app.md) for what the two Cash App bridges share.

## How it works

1. [Select a source route](./cash_app.md#selecting-a-route) for the Cash App user.
2. Call `BridgeToCashApp` with the recipient, the route, the USD amount and the payer's refund address. The SDK checks the recipient and gets a quote from the cross-chain provider.
3. Show the payer the returned `PaymentRequest`, and store the `BridgeId` and `StatusToken` to [track the bridge](./bridging.md#tracking-a-bridge). Once their deposit lands, the provider pays the recipient.

```go
// List the routes to the Cash App user and pick what the payer pays in,
// e.g. USDC on Base.
recipient := "$alice"
routesResponse, err := client.GetRoutes(breez_sdk_spark.GetBridgeRoutesRequest{
	Recipient: recipient,
})
if err != nil {
	return nil, err
}
routes := routesResponse.Routes
var route *breez_sdk_spark.BridgeRoutePair
for i := range routes {
	if routes[i].Source.Asset == "USDC" && routes[i].Source.Chain == "base" {
		route = &routes[i]
		break
	}
}
if route == nil {
	return nil, errors.New("no route from USDC on Base")
}

// Pay $10 of USDC to the Cash App user $alice. The amount is in USDC base
// units (6 decimals), so 10_000_000 = 10 USDC, about $10. The deposit is
// refunded to the payer's address if delivery fails.
refundAddress := "<payer address>"
response, err := client.BridgeToCashApp(breez_sdk_spark.CreateBridgeRequest{
	Recipient:      recipient,
	Route:          *route,
	Amount:         new(big.Int).SetInt64(10_000_000),
	RefundAddress:  &refundAddress,
	FeePolicy:      nil,
	MaxSlippageBps: nil,
})
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

The `Recipient` is a Cash App username, given as `$alice`, or the user's Cash App Lightning address, `alice@cash.app`. Both reach the same user, who is paid at that Lightning address. `BridgeToCashApp` also takes the bare `alice`, but `GetRoutes` needs the `$`.

`GetRoutes` looks the user up, so an unknown username fails before anything is quoted. `BridgeToCashApp` fails with `SdkErrorInvalidInput` for a recipient who isn't a Cash App user, or an amount outside what the recipient accepts.

## Refund address

The `RefundAddress` is the payer's own address on the source chain. If the recipient can't be paid, the provider refunds the deposit there. It must belong to the `Source` chain's family (e.g. an EVM `0x...` address for USDC on Base).

## Response fields

`BridgeToCashApp` returns a `CreateBridgeResponse`, described in [Bridging USDC/USDT](./bridging.md#response-fields). Its `PaymentRequest` is what the payer pays: an EIP-681 URI on EVM chains, otherwise the bare deposit address.

## Limitations

- **Delivery can still fail after funding.** The provider looks the recipient up again when it pays. If the account was closed, its limit lowered, or slippage moved the amount across a limit since the quote, the payment fails and the deposit is refunded to `RefundAddress`.
- **Lightning delivery only.** A completed bridge means the Lightning payment was delivered. Whether Cash App credits it as Bitcoin or converts it to dollars depends on the recipient's Cash App settings.
