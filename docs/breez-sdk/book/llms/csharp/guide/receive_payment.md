# Receiving payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.receive_payment

Once the SDK is initialized, you can directly begin receiving payments. The SDK supports receiving via Lightning, Bitcoin, Spark, and USDC/USDT into Spark from a supported external chain.

## Lightning

#### BOLT11 invoice

When receiving via Lightning, we can generate a BOLT11 invoice to be paid. Setting the invoice amount fixes the amount the sender should pay.

To create an invoice for another Spark wallet, set `ReceiverIdentityPublicKey` to that wallet's identity public key. Creating the invoice requires only the receiver's public key, not their private keys.

**Note:** the payment may fallback to a direct Spark payment (if the payer's client supports this).

```csharp
var description = "<invoice description>";
// Optionally set the invoice amount you wish the payer to send
var optionalAmountSats = 5_000UL;
// Optionally set the expiry duration in seconds
var optionalExpirySecs = 3600U;
string? optionalReceiverIdentityPublicKey = null;
var paymentMethod = new ReceivePaymentMethod.Bolt11Invoice(
    description: description,
    amountSats: optionalAmountSats,
    expirySecs: optionalExpirySecs,
    paymentHash: null,
    receiverIdentityPublicKey: optionalReceiverIdentityPublicKey
);
var request = new ReceivePaymentRequest(paymentMethod: paymentMethod);
var response = await sdk.ReceivePayment(request: request);

var paymentRequest = response.paymentRequest;
Console.WriteLine($"Payment Request: {paymentRequest}");
var receiveFeeSats = response.fee;
Console.WriteLine($"Fees: {receiveFeeSats} sats");
```



#### LNURL-Pay & Lightning address

To receive via LNURL-Pay and/or a Lightning address, follow [these instructions](/llms/csharp/guide/receive_lnurl_pay.md).

> Note: Lightning payments work in Spark even if the receiver is offline. To understand how it works under the hood, read [this](https://docs.spark.money/learn/lightning).

## Bitcoin

For on-chain payments you can generate a Bitcoin deposit address to receive payments. By default the existing address is returned; you can optionally request a new address to rotate to a fresh one for improved privacy. All previously generated addresses remain monitored.

On-chain deposits go through the following lifecycle:

1. **Detected** — The SDK detects the deposit and emits a `SdkEvent.NewDeposits` event. The deposit may or may not have sufficient confirmations to be claimed yet.
2. **Sufficient confirmations** — After **3 on-chain confirmations**, the deposit has sufficient confirmations and the SDK automatically attempts to claim it.
3. **Claimed or unclaimed** — If claiming succeeds, the funds are added to your balance. If it fails (e.g. fees too high), the deposit remains unclaimed and can be [manually claimed or refunded](/llms/csharp/guide/onchain_claims.md).

```csharp
bool? newAddress = null; // Set to true to get a new address
var request = new ReceivePaymentRequest(
    paymentMethod: new ReceivePaymentMethod.BitcoinAddress(
        newAddress: newAddress)
);
var response = await sdk.ReceivePayment(request: request);

var paymentRequest = response.paymentRequest;
Console.WriteLine($"Payment Request: {paymentRequest}");
var receiveFeeSats = response.fee;
Console.WriteLine($"Fees: {receiveFeeSats} sats");
```



To track pending deposits, use `ListUnclaimedDeposits` and filter by the `IsMature` field:

```csharp
var request = new ListUnclaimedDepositsRequest();
var response = await sdk.ListUnclaimedDeposits(request: request);

var pendingDeposits = response.deposits.Where(d => !d.isMature).ToList();

foreach (var deposit in pendingDeposits)
{
    Console.WriteLine($"Pending deposit: {deposit.txid}:{deposit.vout}");
    Console.WriteLine($"Amount: {deposit.amountSats} sats");
}
```



## Spark

For payments between Spark users, you can use a Spark address or generate a Spark invoice to receive payments.

#### Spark address

Spark addresses are static.

```csharp
var request = new ReceivePaymentRequest(
    paymentMethod: new ReceivePaymentMethod.SparkAddress()
);
var response = await sdk.ReceivePayment(request: request);

var paymentRequest = response.paymentRequest;
Console.WriteLine($"Payment Request: {paymentRequest}");
var receiveFeeSats = response.fee;
Console.WriteLine($"Fees: {receiveFeeSats} sats");
```



#### Spark invoice

Spark invoices are single-use and may impose restrictions on the payment, such as amount, expiry, and who is able to pay it.

```csharp
var optionalDescription = "<invoice description>";
var optionalAmountSats = new BigInteger(5000);
// Optionally set the expiry UNIX timestamp in seconds
var optionalExpiryTimeSeconds = 1716691200UL;
var optionalSenderPublicKey = "<sender public key>";

var request = new ReceivePaymentRequest(
    paymentMethod: new ReceivePaymentMethod.SparkInvoice(
        description: optionalDescription,
        amount: optionalAmountSats,
        expiryTime: optionalExpiryTimeSeconds,
        senderPublicKey: optionalSenderPublicKey,
        tokenIdentifier: null
    )
);
var response = await sdk.ReceivePayment(request: request);

var paymentRequest = response.paymentRequest;
Console.WriteLine($"Payment Request: {paymentRequest}");
var receiveFeeSats = response.fee;
Console.WriteLine($"Fees: {receiveFeeSats} sats");
```



## USDC/USDT

Cross-chain receive is supported only via the Orchestra provider. Receive USDC or USDT from a sender on one of several supported chains: Ethereum-family chains (Arbitrum, Base, and similar EVM networks), Solana, and Tron. The receiver lands either BTC sats or [USDB](./stable_balance.md) (a 6-decimal USD-pegged token on Spark) on the Spark side. This feature must be enabled in [the SDK configuration](./config.md#usdc-usdt) before using. See [USDC/USDT](./cross_chain.md) for provider details and the status lifecycle.

Call `GetCrossChainRoutes` with `CrossChainRouteFilter.Receive` to discover supported source assets. Each `CrossChainRoutePair` names the provider, source chain and asset, decimals, optional token contract address, and the Spark-side destinations the route lands (`CrossChainRoutePair.AcceptedAssets`).

```csharp
var filter = new CrossChainRouteFilter.Receive(contractAddress: null);
var routes = await sdk.GetCrossChainRoutes(filter: filter);

foreach (var route in routes)
{
    Console.WriteLine(
        $"Route via {route.provider}: {route.chain}/{route.asset} -> Spark"
    );
}
```



Build `ReceivePaymentMethod.CrossChain` with the chosen route and an `amount`.

The `Amount` on {{#name ReceivePaymentMethod::CrossChain}} is in the source asset's base units, per the route's `CrossChainRoutePair.Decimals`. USD-stable sources sit at USD parity, so `1_000_000` is 1 USDC (6 decimals), about $1.

`FeeMode` controls what the amount means:

- `CrossChainFeeMode.FeesExcluded` (default): `amount` is the receiver's target on Spark. The SDK pads the sender's deposit to cover provider fees plus an overpay buffer.
- `CrossChainFeeMode.FeesIncluded`: `amount` is the deposit the sender pays. The receiver lands `amount - fees`.

`Destination` picks which Spark-side asset the receiver wants delivered. Left unset, the SDK auto-picks the wallet's active stable-balance token if the route supports it, otherwise BTC (converted from the USD amount via the live BTC/USD rate).

`MaxSlippageBps` (10 to 500) bounds the price movement tolerated between quote and delivery. `TargetOverpayBps` (0 to 500) sets the FeesExcluded overpay buffer. Left unset, the SDK defaults apply.

The `PaymentRequest` field carries an EIP-681 URI for EVM routes and the bare deposit address for Solana and Tron. The `CrossChainInfo` block surfaces the bare deposit address, deposit amount, expected receive amount, destination denomination, and quote `ExpiresAt`. The receiver pays no fee; the sender's deposit covers it.

```csharp
// amount is in the route's source-asset base units (USD-stable
// parity: 1_000_000 = $1 on 6-decimal routes). See the guide for
// feeMode, destination, and the slippage/overpay overrides.
var amount = new BigInteger(1_000_000);
SparkAsset? optionalDestination = null;
uint? optionalMaxSlippageBps = 100;
uint? optionalTargetOverpayBps = null;
CrossChainFeeMode? optionalFeeMode = null;

var request = new ReceivePaymentRequest(
    paymentMethod: new ReceivePaymentMethod.CrossChain(
        route: route,
        amount: amount,
        destination: optionalDestination,
        feeMode: optionalFeeMode,
        maxSlippageBps: optionalMaxSlippageBps,
        targetOverpayBps: optionalTargetOverpayBps
    )
);
var response = await sdk.ReceivePayment(request: request);

Console.WriteLine($"Payment request: {response.paymentRequest}");
if (response.crossChainInfo is { } info)
{
    Console.WriteLine($"Deposit address: {info.depositAddress}");
    Console.WriteLine($"Deposit amount: {info.depositAmount}");
    Console.WriteLine(
        "Expected received: "
            + $"{info.expectedReceivedAmount} {info.destinationAsset}"
    );
    Console.WriteLine($"Expires at: {info.expiresAt}");
}
```



## Event Flows

Once a receive payment is initiated, you can follow and react to the different payment events using the guide below for each payment method. See [listening to events](/llms/csharp/guide/events.md) for how to subscribe to events. 

The `SdkEvent.Synced` event is also emitted as the SDK syncs in the background. See [fetching the balance](/llms/csharp/guide/get_info.md) for the recommended pattern for refreshing the balance and payments list.

#### Lightning

| Event                | Description                                                       | UX Suggestion                                    |
| -------------------- | ----------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The Spark transfer was detected and the claim process will start. | Show payment as pending.                         |
| **PaymentSucceeded** | The Spark transfer is claimed and the payment is complete.        | Show the payment as complete and call `GetInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/csharp/guide/get_info.md). |

#### Bitcoin

The following events are emitted in order during the deposit lifecycle. See [Listening to events](/llms/csharp/guide/events.md) for how to subscribe.

| Event                 | Description                                                                                                                              | UX Suggestion                                                                                               |
| --------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| **NewDeposits**       | New deposits were detected. Each deposit includes a `IsMature` field indicating whether it has enough confirmations to be claimed. | Show the deposit to the user. If it does not yet have sufficient confirmations, show it as pending.          |
| **ClaimedDeposits**   | The SDK successfully claimed confirmed deposits.                                                                                         |                                                                                                             |
| **UnclaimedDeposits** | Claiming failed (e.g. fee exceeded the configured maximum or the UTXO could not be found).                                               | Allow the user to manually claim or refund. See [Claiming on-chain deposits](/llms/csharp/guide/onchain_claims.md). |
| **PaymentPending**    | The Spark transfer was detected and the claim process will start.                                                                        | Show payment as pending.                                                                                    |
| **PaymentSucceeded**  | The Spark transfer is claimed and the payment is complete.                                                                               | Show the payment as complete and call `GetInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/csharp/guide/get_info.md).                                                            |

#### Spark

| Event                | Description                                                                                                                                                                                          | UX Suggestion                                    |
| -------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The Spark transfer was detected and the claim process will start. For Spark HTLC payments, the claim will only start once the HTLC is claimed. For more details see [Spark HTLC payments](htlcs.md). | Show payment as pending.                         |
| **PaymentSucceeded** | The Spark transfer is claimed and the payment is complete.                                                                                                                                           | Show the payment as complete and call `GetInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/csharp/guide/get_info.md). |

#### USDC/USDT

| Event                | Description                                                                                          | UX Suggestion                                    |
| -------------------- | ---------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The sender's deposit was detected and the inbound Spark transfer claim is in progress.               | Show payment as pending.                         |
| **PaymentSucceeded** | The inbound Spark transfer is claimed and the payment is complete.                                   | Show the payment as complete and call `GetInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/csharp/guide/get_info.md). |
