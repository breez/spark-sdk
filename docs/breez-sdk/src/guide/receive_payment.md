<h1 id="receiving-payments">
    <a class="header" href="#receiving-payments">Receiving payments</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.receive_payment">API docs</a>
</h1>

Once the SDK is initialized, you can directly begin receiving payments. The SDK supports receiving via Lightning, Bitcoin, Spark, and USDC/USDT into Spark from a supported external chain.

## Lightning

#### BOLT11 invoice

When receiving via Lightning, we can generate a BOLT11 invoice to be paid. Setting the invoice amount fixes the amount the sender should pay.

**Note:** the payment may fallback to a direct Spark payment (if the payer's client supports this).

{{#tabs receive_payment:receive-payment-lightning-bolt11}}

#### LNURL-Pay & Lightning address

To receive via LNURL-Pay and/or a Lightning address, follow [these instructions](/guide/receive_lnurl_pay.md).

> Note: Lightning payments work in Spark even if the receiver is offline. To understand how it works under the hood, read [this](https://docs.spark.money/learn/lightning).

## Bitcoin

For on-chain payments you can generate a Bitcoin deposit address to receive payments. By default the existing address is returned; you can optionally request a new address to rotate to a fresh one for improved privacy. All previously generated addresses remain monitored.

On-chain deposits go through the following lifecycle:

1. **Detected** — The SDK detects the deposit and emits a {{#enum SdkEvent::NewDeposits}} event. The deposit may or may not have sufficient confirmations to be claimed yet.
2. **Sufficient confirmations** — After **3 on-chain confirmations**, the deposit has sufficient confirmations and the SDK automatically attempts to claim it.
3. **Claimed or unclaimed** — If claiming succeeds, the funds are added to your balance. If it fails (e.g. fees too high), the deposit remains unclaimed and can be [manually claimed or refunded](/guide/onchain_claims.md).

{{#tabs receive_payment:receive-payment-onchain}}

To track pending deposits, use {{#name list_unclaimed_deposits}} and filter by the {{#name is_mature}} field:

{{#tabs refunding_payments:list-pending-deposits}}

## Spark

For payments between Spark users, you can use a Spark address or generate a Spark invoice to receive payments.

#### Spark address

Spark addresses are static.

{{#tabs receive_payment:receive-payment-spark-address}}

#### Spark invoice

Spark invoices are single-use and may impose restrictions on the payment, such as amount, expiry, and who is able to pay it.

{{#tabs receive_payment:receive-payment-spark-invoice}}

<h2 id="usdc-usdt">
    <a class="header" href="#usdc-usdt">USDC/USDT</a>
</h2>

Receive USDC or USDT from a sender on one of several supported chains: Ethereum-family chains (Arbitrum, Base, and similar EVM networks), Solana, and Tron. The receiver lands either BTC sats or USDB on the Spark side. This feature must be enabled in [the SDK configuration](./config.md#send-usdc-usdt) before using. See [USDC/USDT](./cross_chain.md) for provider details and the status lifecycle. Cross-chain receive is supported only via the Orchestra provider.

Call {{#name get_cross_chain_routes}} with {{#enum CrossChainRouteFilter::Receive}} to discover supported source assets. Each {{#name CrossChainRoutePair}} names the provider, source chain and asset, decimals, optional token contract address, and the Spark-side destinations the route lands ({{#name CrossChainRoutePair.spark_assets}}).

{{#tabs cross_chain:cross-chain-get-receive-routes}}

Build {{#enum ReceivePaymentMethod::CrossChain}} with the chosen route and an `amount` denominated in the source-asset base units (e.g. USDC base units when the source is USDC). The optional {{#name destination}} picks which Spark-side asset the receiver wants delivered: when unset, the SDK auto-picks the wallet's active stable-balance token if the route supports it, otherwise BTC. An optional {{#name max_slippage_bps}} accepts 10 to 500.

The response carries the provider-controlled deposit address as {{#name payment_request}} (share it with the sender) and a {{#name cross_chain_info}} block with the deposit amount, expected receive amount, destination denomination, and quote {{#name expires_at}}. The receiver doesn't pay a fee directly — the sender's deposit covers it.

{{#tabs cross_chain:cross-chain-receive}}

## Event Flows

Once a receive payment is initiated, you can follow and react to the different payment events using the guide below for each payment method. See [listening to events](/guide/events.md) for how to subscribe to events. 

The {{#enum SdkEvent::Synced}} event is also emitted as the SDK syncs in the background. See [fetching the balance](/guide/get_info.md) for the recommended pattern for refreshing the balance and payments list.

#### Lightning

| Event                | Description                                                       | UX Suggestion                                    |
| -------------------- | ----------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The Spark transfer was detected and the claim process will start. | Show payment as pending.                         |
| **PaymentSucceeded** | The Spark transfer is claimed and the payment is complete.        | Show the payment as complete and call {{#name get_info}} to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/guide/get_info.md). |

#### Bitcoin

The following events are emitted in order during the deposit lifecycle. See [Listening to events](/guide/events.md) for how to subscribe.

| Event                 | Description                                                                                                                              | UX Suggestion                                                                                               |
| --------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| **NewDeposits**       | New deposits were detected. Each deposit includes a {{#name is_mature}} field indicating whether it has enough confirmations to be claimed. | Show the deposit to the user. If it does not yet have sufficient confirmations, show it as pending.          |
| **ClaimedDeposits**   | The SDK successfully claimed confirmed deposits.                                                                                         |                                                                                                             |
| **UnclaimedDeposits** | Claiming failed (e.g. fee exceeded the configured maximum or the UTXO could not be found).                                               | Allow the user to manually claim or refund. See [Claiming on-chain deposits](/guide/onchain_claims.md). |
| **PaymentPending**    | The Spark transfer was detected and the claim process will start.                                                                        | Show payment as pending.                                                                                    |
| **PaymentSucceeded**  | The Spark transfer is claimed and the payment is complete.                                                                               | Show the payment as complete and call {{#name get_info}} to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/guide/get_info.md).                                                            |

#### Spark

| Event                | Description                                                                                                                                                                                          | UX Suggestion                                    |
| -------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The Spark transfer was detected and the claim process will start. For Spark HTLC payments, the claim will only start once the HTLC is claimed. For more details see [Spark HTLC payments](htlcs.md). | Show payment as pending.                         |
| **PaymentSucceeded** | The Spark transfer is claimed and the payment is complete.                                                                                                                                           | Show the payment as complete and call {{#name get_info}} to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/guide/get_info.md). |

<h4 id="usdc-usdt-1">
    <a class="header" href="#usdc-usdt-1">USDC/USDT</a>
</h4>

| Event                | Description                                                                                          | UX Suggestion                                    |
| -------------------- | ---------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The sender's deposit was detected and the inbound Spark transfer claim is in progress.               | Show payment as pending.                         |
| **PaymentSucceeded** | The inbound Spark transfer is claimed and the payment is complete.                                   | Show the payment as complete and call {{#name get_info}} to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/guide/get_info.md). |
