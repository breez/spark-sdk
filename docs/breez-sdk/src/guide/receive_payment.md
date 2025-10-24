<h1 id="receiving-payments">
    <a class="header" href="#receiving-payments">Receiving payments</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.receive_payment">API docs</a>
</h1>

Once the SDK is initialized, you can directly begin receiving payments. The SDK currently supports three methods of receiving: Lightning, Bitcoin and Spark.

### Lightning

#### BOLT11 invoice

When receiving via Lightning, we can generate a BOLT11 invoice to be paid. Setting the invoice amount fixes the amount the sender should pay.

**Note:** the payment may fallback to a direct Spark payment (if the payer's client supports this).

<custom-tabs category="lang">
<div slot="title">Rust</div>
<section>

```rust,ignore
{{#include ../../snippets/rust/src/receive_payment.rs:receive-payment-lightning-bolt11}}
```

</section>

<div slot="title">Swift</div>
<section>

```swift,ignore
{{#include ../../snippets/swift/BreezSdkSnippets/Sources/ReceivePayment.swift:receive-payment-lightning-bolt11}}
```

</section>

<div slot="title">Kotlin</div>
<section>

```kotlin,ignore
{{#include ../../snippets/kotlin_mpp_lib/shared/src/commonMain/kotlin/com/example/kotlinmpplib/ReceivePayment.kt:receive-payment-lightning-bolt11}}
```

</section>

<div slot="title">Javascript</div>
<section>

```typescript
{{#include ../../snippets/wasm/receive_payment.ts:receive-payment-lightning-bolt11}}
```

</section>

<div slot="title">React Native</div>
<section>

```typescript
{{#include ../../snippets/react-native/receive_payment.ts:receive-payment-lightning-bolt11}}
```

</section>

<div slot="title">Flutter</div>
<section>

```dart,ignore
{{#include ../../snippets/flutter/lib/receive_payment.dart:receive-payment-lightning-bolt11}}
```

</section>

<div slot="title">Python</div>
<section>

```python,ignore
{{#include ../../snippets/python/src/receive_payment.py:receive-payment-lightning-bolt11}}
```

</section>

<div slot="title">Go</div>
<section>

```go,ignore
{{#include ../../snippets/go/receive_payment.go:receive-payment-lightning-bolt11}}
```

</section>
</custom-tabs>

#### LNURL-Pay & Lightning address

To receive via LNURL-Pay and/or a Lightning address, follow [these instructions](/guide/receive_lnurl_pay.md).

> Note: Lightning payments work in Spark even if the receiver is offline. To understand how it works under the hood, read [this](https://docs.spark.money/spark/lightning).

### Bitcoin

For onchain payments you can use the static Bitcoin address to receive payments.
The SDK monitors the specified address for new UTXOs and automatically initiates the claim process when funds are detected.

<custom-tabs category="lang">
<div slot="title">Rust</div>
<section>

```rust,ignore
{{#include ../../snippets/rust/src/receive_payment.rs:receive-payment-onchain}}
```

</section>

<div slot="title">Swift</div>
<section>

```swift,ignore
{{#include ../../snippets/swift/BreezSdkSnippets/Sources/ReceivePayment.swift:receive-payment-onchain}}
```

</section>

<div slot="title">Kotlin</div>
<section>

```kotlin,ignore
{{#include ../../snippets/kotlin_mpp_lib/shared/src/commonMain/kotlin/com/example/kotlinmpplib/ReceivePayment.kt:receive-payment-onchain}}
```

</section>

<div slot="title">Javascript</div>
<section>

```typescript
{{#include ../../snippets/wasm/receive_payment.ts:receive-payment-onchain}}
```

</section>

<div slot="title">React Native</div>
<section>

```typescript
{{#include ../../snippets/react-native/receive_payment.ts:receive-payment-onchain}}
```

</section>

<div slot="title">Flutter</div>
<section>

```dart,ignore
{{#include ../../snippets/flutter/lib/receive_payment.dart:receive-payment-onchain}}
```

</section>

<div slot="title">Python</div>
<section>

```python,ignore
{{#include ../../snippets/python/src/receive_payment.py:receive-payment-onchain}}
```

</section>

<div slot="title">Go</div>
<section>

```go,ignore
{{#include ../../snippets/go/receive_payment.go:receive-payment-onchain}}
```

</section>
</custom-tabs>

### Spark

For payments between Spark users, you can use a Spark address or generate a Spark invoice to receive payments.

#### Spark address

Spark addresses are static.

<custom-tabs category="lang">
<div slot="title">Rust</div>
<section>

```rust,ignore
{{#include ../../snippets/rust/src/receive_payment.rs:receive-payment-spark-address}}
```

</section>

<div slot="title">Swift</div>
<section>

```swift,ignore
{{#include ../../snippets/swift/BreezSdkSnippets/Sources/ReceivePayment.swift:receive-payment-spark-address}}
```

</section>

<div slot="title">Kotlin</div>
<section>

```kotlin,ignore
{{#include ../../snippets/kotlin_mpp_lib/shared/src/commonMain/kotlin/com/example/kotlinmpplib/ReceivePayment.kt:receive-payment-spark-address}}
```

</section>

<div slot="title">Javascript</div>
<section>

```typescript
{{#include ../../snippets/wasm/receive_payment.ts:receive-payment-spark-address}}
```

</section>

<div slot="title">React Native</div>
<section>

```typescript
{{#include ../../snippets/react-native/receive_payment.ts:receive-payment-spark-address}}
```

</section>

<div slot="title">Flutter</div>
<section>

```dart,ignore
{{#include ../../snippets/flutter/lib/receive_payment.dart:receive-payment-spark-address}}
```

</section>

<div slot="title">Python</div>
<section>

```python,ignore
{{#include ../../snippets/python/src/receive_payment.py:receive-payment-spark-address}}
```

</section>

<div slot="title">Go</div>
<section>

```go,ignore
{{#include ../../snippets/go/receive_payment.go:receive-payment-spark-address}}
```

</section>
</custom-tabs>

#### Spark invoice

Spark invoices are single-use and may impose restrictions on the payment, such as amount, expiry, and who is able to pay it.

<custom-tabs category="lang">
<div slot="title">Rust</div>
<section>

```rust,ignore
{{#include ../../snippets/rust/src/receive_payment.rs:receive-payment-spark-invoice}}
```

</section>

<div slot="title">Swift</div>
<section>

```swift,ignore
{{#include ../../snippets/swift/BreezSdkSnippets/Sources/ReceivePayment.swift:receive-payment-spark-invoice}}
```

</section>

<div slot="title">Kotlin</div>
<section>

```kotlin,ignore
{{#include ../../snippets/kotlin_mpp_lib/shared/src/commonMain/kotlin/com/example/kotlinmpplib/ReceivePayment.kt:receive-payment-spark-invoice}}
```

</section>

<div slot="title">Javascript</div>
<section>

```typescript
{{#include ../../snippets/wasm/receive_payment.ts:receive-payment-spark-invoice}}
```

</section>

<div slot="title">React Native</div>
<section>

```typescript
{{#include ../../snippets/react-native/receive_payment.ts:receive-payment-spark-invoice}}
```

</section>

<div slot="title">Flutter</div>
<section>

```dart,ignore
{{#include ../../snippets/flutter/lib/receive_payment.dart:receive-payment-spark-invoice}}
```

</section>

<div slot="title">Python</div>
<section>

```python,ignore
{{#include ../../snippets/python/src/receive_payment.py:receive-payment-spark-invoice}}
```

</section>

<div slot="title">Go</div>
<section>

```go,ignore
{{#include ../../snippets/go/receive_payment.go:receive-payment-spark-invoice}}
```

</section>
</custom-tabs>

## Waiting for a payment

It is generally recommended to use [event flows] to react to payment completion. However, there is a convenience function to wait for payment completion.

<custom-tabs category="lang">
<div slot="title">Rust</div>
<section>

```rust,ignore
{{#include ../../snippets/rust/src/receive_payment.rs:wait-for-payment}}
```

</section>

<div slot="title">Swift</div>
<section>

```swift,ignore
{{#include ../../snippets/swift/BreezSdkSnippets/Sources/ReceivePayment.swift:wait-for-payment}}
```

</section>

<div slot="title">Kotlin</div>
<section>

```kotlin,ignore
{{#include ../../snippets/kotlin_mpp_lib/shared/src/commonMain/kotlin/com/example/kotlinmpplib/ReceivePayment.kt:wait-for-payment}}
```

</section>

<div slot="title">Javascript</div>
<section>

```typescript
{{#include ../../snippets/wasm/receive_payment.ts:wait-for-payment}}
```

</section>

<div slot="title">React Native</div>
<section>

```typescript
{{#include ../../snippets/react-native/receive_payment.ts:wait-for-payment}}
```

</section>

<div slot="title">Flutter</div>
<section>

```dart,ignore
{{#include ../../snippets/flutter/lib/receive_payment.dart:wait-for-payment}}
```

</section>

<div slot="title">Python</div>
<section>

```python,ignore
{{#include ../../snippets/python/src/receive_payment.py:wait-for-payment}}
```

</section>

<div slot="title">Go</div>
<section>

```go,ignore
{{#include ../../snippets/go/receive_payment.go:wait-for-payment}}
```

</section>
</custom-tabs>

[event flows]: #event-flows

## Event Flows

Once a receive payment is initiated, you can follow and react to the different payment events using the guide below for each payment method. See [Listening to events](/guide/events.md) for how to subscribe to events.

| Event      | Description                                    | UX Suggestion                                                                                                                         |
| ---------- | ---------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| **Synced** | The SDK has synced payments in the background. | Update the payments list and balance. See [listing payments](/guide/list_payments.md) and [fetching the balance](/guide/get_info.md). |

### Lightning

| Event                | Description                                                | UX Suggestion                                    |
| -------------------- | ---------------------------------------------------------- | ------------------------------------------------ |
| **PaymentSucceeded** | The Spark transfer is claimed and the payment is complete. | Update the balance and show payment as complete. |

### Bitcoin

| Event                      | Description                                                                                                                                                                                               | UX Suggestion                                                                                           |
| -------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| **ClaimDepositsFailed**    | The SDK attempted to claim static address deposits but they failed from one of several reasons. Either the claim fee exceeded the maximum allowed limit or there was an issue finding the available UTXO. | Allow the user to refund these failed deposits. See [Refunding payments](/guide/refunding_payments.md). |
| **ClaimDepositsSucceeded** | The SDK successfully claimed static address deposits.                                                                                                                                                     |                                                                                                         |
| **PaymentSucceeded**       | The Spark transfer is claimed and the payment is complete.                                                                                                                                                | Update the balance and show payment as complete.                                                        |

### Spark

| Event                | Description                                                | UX Suggestion                                    |
| -------------------- | ---------------------------------------------------------- | ------------------------------------------------ |
| **PaymentSucceeded** | The Spark transfer is claimed and the payment is complete. | Update the balance and show payment as complete. |
