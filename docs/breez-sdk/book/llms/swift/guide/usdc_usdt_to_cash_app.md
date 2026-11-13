# USDC/USDT to Cash App

`bridgeToCashApp` lets someone holding USDC or USDT on an external chain pay a Cash App user. The payer deposits the stablecoin, and the cross-chain provider pays the recipient in Bitcoin over Lightning. See [Cash App and USDC/USDT](./cash_app.md) for what the two Cash App bridges share.

## How it works

1. Get a [bridging client](./bridging.md#getting-a-bridging-client): build one from your API key with `newBridgingClient`, or call `getBridgingClient` on a connected SDK.
2. [Select a source route](./cash_app.md#selecting-a-route) for the Cash App user.
3. Call `bridgeToCashApp` with the recipient, the route, the USD amount and the payer's refund address. The SDK checks the recipient and gets a quote from the cross-chain provider.
4. Show the payer the returned `paymentRequest`, and store the `bridgeId` and `statusToken` to [track the bridge](#tracking-the-bridge). Once their deposit lands, the provider pays the recipient.

```swift
// List the routes to the Cash App user and pick what the payer pays in,
// e.g. USDC on Base.
let recipient = "$alice"
let routes = try await client.getRoutes(
    request: GetBridgeRoutesRequest(recipient: recipient)
).routes
guard
    let route = routes.first(where: {
        $0.source.asset == "USDC" && $0.source.chain == "base"
    })
else {
    throw NSError(domain: "CashApp", code: 2)
}

// Pay $10 of USDC to the Cash App user $alice. The amount is in USDC base
// units (6 decimals), so 10_000_000 = 10 USDC, about $10. The deposit is
// refunded to the payer's address if delivery fails.
let response = try await client.bridgeToCashApp(
    request: CreateBridgeRequest(
        recipient: recipient,
        route: route,
        amount: BInt(10_000_000),
        refundAddress: "<payer address>",
        feePolicy: nil,
        maxSlippageBps: nil
    ))

// Show the payer what to pay. The recipient then receives Bitcoin.
let info = response.info
print("Pay with: \(response.paymentRequest)")
print(
    "Deposit \(info.depositAmount) to \(info.depositAddress), "
        + "recipient receives ~\(info.expectedReceivedAmount) sats"
)
```



## Recipient

The `recipient` is a Cash App username, given as `$alice`, or the user's Cash App Lightning address, `alice@cash.app`. Both reach the same user, who is paid at that Lightning address. `bridgeToCashApp` also takes the bare `alice`, but `getRoutes` needs the `$`.

`getRoutes` looks the user up, so an unknown username fails before anything is quoted. `bridgeToCashApp` fails with `SdkError.invalidInput` for a recipient who isn't a Cash App user, or an amount outside what the recipient accepts.

## Refund address

The `refundAddress` is the payer's own address on the source chain. If the recipient can't be paid, the provider refunds the deposit there. It must belong to the `source` chain's family (e.g. an EVM `0x...` address for USDC on Base).

## Response fields

`bridgeToCashApp` returns a `CreateBridgeResponse`, described in [Bridging USDC/USDT](./bridging.md#response-fields). Its `paymentRequest` is what the payer pays: an EIP-681 URI on EVM chains, otherwise the bare deposit address. A Solana transfer built in code must also create the deposit address's token account, which wallet apps do on their own (see [Paying from Solana](./cross_chain.md#paying-from-solana)).

## Tracking the bridge

Pass the `bridgeId` and `statusToken` from the response to `getBridgeStatus`. The bridge reads:

- `BridgeStatus.created` until the provider sees the deposit. It stays `BridgeStatus.created` after its `expiresAt`, because a late deposit may still be accepted at a new price. Stop polling when you no longer expect one.
- `BridgeStatus.pending` from the deposit to the Lightning payment. `depositTxHash` is the payer's transaction.
- `BridgeStatus.completed` once the Lightning payment reached the recipient. `amountOut` is in sats. `deliveryTxHash` is the provider's reference for the payment, not a Lightning payment hash.
- `BridgeStatus.refunding`, then `BridgeStatus.refunded`, if the recipient couldn't be paid (see [Limitations](#limitations)) and the deposit is returned to the `refundAddress`.

```swift
let response = try await client.getBridgeStatus(
    request: GetBridgeStatusRequest(bridgeId: bridgeId, statusToken: statusToken))
switch response.status {
case .completed:
    print("Delivered \(response.amountOut.map { "\($0)" } ?? "unknown")")
case .refunded, .failed, .expired:
    print("Not delivered: \(response.status)")
default:
    print("Not final yet: \(response.status)")
}
```



See [Tracking a bridge](./bridging.md#tracking-a-bridge) for every status.

## Limitations

- **Delivery can still fail after funding.** The provider looks the recipient up again when it pays. If the account was closed, its limit lowered, or slippage moved the amount across a limit since the quote, the payment fails and the deposit is refunded to `refundAddress`.
- **Lightning delivery only.** A completed bridge means the Lightning payment was delivered. Whether Cash App credits it as Bitcoin or converts it to dollars depends on the recipient's Cash App settings.
