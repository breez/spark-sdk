# Receiving payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.receive_payment

Once the SDK is initialized, you can directly begin receiving payments. The SDK currently supports three methods of receiving: Lightning, Bitcoin and Spark.

## Lightning

#### BOLT11 invoice

When receiving via Lightning, we can generate a BOLT11 invoice to be paid. Setting the invoice amount fixes the amount the sender should pay.

To create an invoice for another Spark wallet, set `receiverIdentityPublicKey` to that wallet's identity public key. Creating the invoice requires only the receiver's public key, not their private keys.

**Note:** the payment may fallback to a direct Spark payment (if the payer's client supports this).

```dart
String description = "<invoice description>";
// Optionally set the invoice amount you wish the payer to send
BigInt optionalAmountSats = BigInt.from(5000);
// Optionally set the expiry duration in seconds
int optionalExpirySecs = 3600;
// Set this to create an invoice for another Spark identity
String? optionalReceiverIdentityPublicKey;

// Create an invoice and set the amount you wish the payer to send
ReceivePaymentRequest request = ReceivePaymentRequest(
    paymentMethod: ReceivePaymentMethod.bolt11Invoice(
        description: description,
        amountSats: optionalAmountSats,
        expirySecs: optionalExpirySecs,
        paymentHash: null,
        receiverIdentityPublicKey: optionalReceiverIdentityPublicKey));
ReceivePaymentResponse response = await sdk.receivePayment(
  request: request,
);

String paymentRequest = response.paymentRequest;
print("Payment request: $paymentRequest");
BigInt receiveFeeSats = response.fee;
print("Fees: $receiveFeeSats sats");
```



#### LNURL-Pay & Lightning address

To receive via LNURL-Pay and/or a Lightning address, follow [these instructions](/llms/flutter/guide/receive_lnurl_pay.md).

> Note: Lightning payments work in Spark even if the receiver is offline. To understand how it works under the hood, read [this](https://docs.spark.money/learn/lightning).

## Bitcoin

For on-chain payments you can generate a Bitcoin deposit address to receive payments. By default the existing address is returned; you can optionally request a new address to rotate to a fresh one for improved privacy. All previously generated addresses remain monitored.

On-chain deposits go through the following lifecycle:

1. **Detected** — The SDK detects the deposit and emits a `SdkEvent.NewDeposits` event. The deposit may or may not have sufficient confirmations to be claimed yet.
2. **Sufficient confirmations** — After **3 on-chain confirmations**, the deposit has sufficient confirmations and the SDK automatically attempts to claim it.
3. **Claimed or unclaimed** — If claiming succeeds, the funds are added to your balance. If it fails (e.g. fees too high), the deposit remains unclaimed and can be [manually claimed or refunded](/llms/flutter/guide/onchain_claims.md).

```dart
bool? newAddress; // Set to true to get a new address
ReceivePaymentRequest request = ReceivePaymentRequest(
    paymentMethod: ReceivePaymentMethod.bitcoinAddress(
        newAddress: newAddress));
ReceivePaymentResponse response = await sdk.receivePayment(
  request: request,
);

String paymentRequest = response.paymentRequest;
print("Payment request: $paymentRequest");
BigInt receiveFeeSats = response.fee;
print("Fees: $receiveFeeSats sats");
```



To track pending deposits, use `listUnclaimedDeposits` and filter by the `isMature` field:

```dart
final request = ListUnclaimedDepositsRequest();
final response = await sdk.listUnclaimedDeposits(request: request);

final pendingDeposits =
    response.deposits.where((d) => !d.isMature).toList();

for (DepositInfo deposit in pendingDeposits) {
  print("Pending deposit: ${deposit.txid}:${deposit.vout}");
  print("Amount: ${deposit.amountSats} sats");
}
```



## Spark

For payments between Spark users, you can use a Spark address or generate a Spark invoice to receive payments.

#### Spark address

Spark addresses are static.

```dart
ReceivePaymentRequest request =
    ReceivePaymentRequest(paymentMethod: ReceivePaymentMethod.sparkAddress());
ReceivePaymentResponse response = await sdk.receivePayment(
  request: request,
);

String paymentRequest = response.paymentRequest;
print("Payment request: $paymentRequest");
BigInt receiveFeeSats = response.fee;
print("Fees: $receiveFeeSats sats");
```



#### Spark invoice

Spark invoices are single-use and may impose restrictions on the payment, such as amount, expiry, and who is able to pay it.

```dart
String optionalDescription = "<invoice description>";
BigInt optionalAmountSats = BigInt.from(5000);
// Optionally set the expiry UNIX timestamp in seconds
BigInt optionalExpiryTimeSeconds = BigInt.from(1716691200);
String optionalSenderPublicKey = "<sender public key>";

ReceivePaymentRequest request =
    ReceivePaymentRequest(paymentMethod: ReceivePaymentMethod.sparkInvoice(
      description: optionalDescription,
      amount: optionalAmountSats,
      expiryTime: optionalExpiryTimeSeconds,
      senderPublicKey: optionalSenderPublicKey,
    ));
ReceivePaymentResponse response = await sdk.receivePayment(
  request: request,
);

String paymentRequest = response.paymentRequest;
print("Payment request: $paymentRequest");
BigInt receiveFeeSats = response.fee;
print("Fees: $receiveFeeSats sats");
```



## Event Flows

Once a receive payment is initiated, you can follow and react to the different payment events using the guide below for each payment method. See [listening to events](/llms/flutter/guide/events.md) for how to subscribe to events. 

The `SdkEvent.Synced` event is also emitted as the SDK syncs in the background. See [fetching the balance](/llms/flutter/guide/get_info.md) for the recommended pattern for refreshing the balance and payments list.

#### Lightning

| Event                | Description                                                       | UX Suggestion                                    |
| -------------------- | ----------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The Spark transfer was detected and the claim process will start. | Show payment as pending.                         |
| **PaymentSucceeded** | The Spark transfer is claimed and the payment is complete.        | Show the payment as complete and call `getInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/flutter/guide/get_info.md). |

#### Bitcoin

The following events are emitted in order during the deposit lifecycle. See [Listening to events](/llms/flutter/guide/events.md) for how to subscribe.

| Event                 | Description                                                                                                                              | UX Suggestion                                                                                               |
| --------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| **NewDeposits**       | New deposits were detected. Each deposit includes a `isMature` field indicating whether it has enough confirmations to be claimed. | Show the deposit to the user. If it does not yet have sufficient confirmations, show it as pending.          |
| **ClaimedDeposits**   | The SDK successfully claimed confirmed deposits.                                                                                         |                                                                                                             |
| **UnclaimedDeposits** | Claiming failed (e.g. fee exceeded the configured maximum or the UTXO could not be found).                                               | Allow the user to manually claim or refund. See [Claiming on-chain deposits](/llms/flutter/guide/onchain_claims.md). |
| **PaymentPending**    | The Spark transfer was detected and the claim process will start.                                                                        | Show payment as pending.                                                                                    |
| **PaymentSucceeded**  | The Spark transfer is claimed and the payment is complete.                                                                               | Show the payment as complete and call `getInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/flutter/guide/get_info.md).                                                            |

#### Spark

| Event                | Description                                                                                                                                                                                          | UX Suggestion                                    |
| -------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The Spark transfer was detected and the claim process will start. For Spark HTLC payments, the claim will only start once the HTLC is claimed. For more details see [Spark HTLC payments](htlcs.md). | Show payment as pending.                         |
| **PaymentSucceeded** | The Spark transfer is claimed and the payment is complete.                                                                                                                                           | Show the payment as complete and call `getInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/flutter/guide/get_info.md). |
