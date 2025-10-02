# Sending payments

Once the SDK is initialized, you can directly begin sending payments. The send process takes two steps:
1. [Preparing the Payment](send_payment.md#preparing-payments)
1. [Sending the Payment](send_payment.md#sending-payments)

For sending payments via LNURL, see [LNURL-Pay](lnurl_pay.md).

<h2 id="preparing-payments">
    <a class="header" href="#preparing-payments">Preparing Payments</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.prepare_send_payment">API docs</a>
</h2>

During the prepare step, the SDK ensures that the inputs are valid with respect to the payment request type,
and also returns the fees related to the payment so they can be confirmed.

The payment request field supports Lightning invoices, Bitcoin addresses and Spark addresses.

### Lightning

#### BOLT11 invoice

For BOLT11 invoices the amount can be optionally set. The amount set in the request is only taken into account if it's an amountless invoice. 

If the invoice also contains a Spark address, it means the payment can be sent directly with a Spark Transfer instead. When this is the case, the prepare response includes the Spark transfer fee.

<custom-tabs category="lang">
<div slot="title">Rust</div>
<section>

```rust,ignore
{{#include ../../snippets/rust/src/send_payment.rs:prepare-send-payment-lightning-bolt11}}
```
</section>

<div slot="title">Swift</div>
<section>

```swift,ignore
{{#include ../../snippets/swift/BreezSdkSnippets/Sources/SendPayment.swift:prepare-send-payment-lightning-bolt11}}
```
</section>

<div slot="title">Kotlin</div>
<section>

```kotlin,ignore
{{#include ../../snippets/kotlin_mpp_lib/shared/src/commonMain/kotlin/com/example/kotlinmpplib/SendPayment.kt:prepare-send-payment-lightning-bolt11}}
```
</section>

<div slot="title">Javascript</div>
<section>

```typescript
{{#include ../../snippets/wasm/send_payment.ts:prepare-send-payment-lightning-bolt11}}
```
</section>

<div slot="title">React Native</div>
<section>

```typescript
{{#include ../../snippets/react-native/send_payment.ts:prepare-send-payment-lightning-bolt11}}
```
</section>

<div slot="title">Flutter</div>
<section>

```dart,ignore
{{#include ../../snippets/flutter/lib/send_payment.dart:prepare-send-payment-lightning-bolt11}}
```
</section>

<div slot="title">Python</div>
<section>

```python,ignore 
{{#include ../../snippets/python/src/send_payment.py:prepare-send-payment-lightning-bolt11}}
```
</section>

<div slot="title">Go</div>
<section>

```go,ignore
{{#include ../../snippets/go/send_payment.go:prepare-send-payment-lightning-bolt11}}
```
</section>
</custom-tabs>

### Bitcoin

For Bitcoin addresses the amount must be set in the request. The prepare response includes fee quotes for three payment speeds: Slow, Medium and Fast.

<custom-tabs category="lang">
<div slot="title">Rust</div>
<section>

```rust,ignore
{{#include ../../snippets/rust/src/send_payment.rs:prepare-send-payment-onchain}}
```
</section>

<div slot="title">Swift</div>
<section>

```swift,ignore
{{#include ../../snippets/swift/BreezSdkSnippets/Sources/SendPayment.swift:prepare-send-payment-onchain}}
```
</section>

<div slot="title">Kotlin</div>
<section>

```kotlin,ignore
{{#include ../../snippets/kotlin_mpp_lib/shared/src/commonMain/kotlin/com/example/kotlinmpplib/SendPayment.kt:prepare-send-payment-onchain}}
```
</section>

<div slot="title">Javascript</div>
<section>

```typescript
{{#include ../../snippets/wasm/send_payment.ts:prepare-send-payment-onchain}}
```
</section>

<div slot="title">React Native</div>
<section>

```typescript
{{#include ../../snippets/react-native/send_payment.ts:prepare-send-payment-onchain}}
```
</section>

<div slot="title">Flutter</div>
<section>

```dart,ignore
{{#include ../../snippets/flutter/lib/send_payment.dart:prepare-send-payment-onchain}}
```
</section>

<div slot="title">Python</div>
<section>

```python,ignore 
{{#include ../../snippets/python/src/send_payment.py:prepare-send-payment-onchain}}
```
</section>

<div slot="title">Go</div>
<section>

```go,ignore
{{#include ../../snippets/go/send_payment.go:prepare-send-payment-onchain}}
```
</section>
</custom-tabs>

### Spark

For Spark addresses the amount must be set in the request. Sending to a Spark address uses a direct Spark transfer.

<custom-tabs category="lang">
<div slot="title">Rust</div>
<section>

```rust,ignore
{{#include ../../snippets/rust/src/send_payment.rs:prepare-send-payment-spark}}
```
</section>

<div slot="title">Swift</div>
<section>

```swift,ignore
{{#include ../../snippets/swift/BreezSdkSnippets/Sources/SendPayment.swift:prepare-send-payment-spark}}
```
</section>

<div slot="title">Kotlin</div>
<section>

```kotlin,ignore
{{#include ../../snippets/kotlin_mpp_lib/shared/src/commonMain/kotlin/com/example/kotlinmpplib/SendPayment.kt:prepare-send-payment-spark}}
```
</section>

<div slot="title">Javascript</div>
<section>

```typescript
{{#include ../../snippets/wasm/send_payment.ts:prepare-send-payment-spark}}
```
</section>

<div slot="title">React Native</div>
<section>

```typescript
{{#include ../../snippets/react-native/send_payment.ts:prepare-send-payment-spark}}
```
</section>

<div slot="title">Flutter</div>
<section>

```dart,ignore
{{#include ../../snippets/flutter/lib/send_payment.dart:prepare-send-spark}}
```
</section>

<div slot="title">Python</div>
<section>

```python,ignore 
{{#include ../../snippets/python/src/send_payment.py:prepare-send-payment-spark}}
```
</section>

<div slot="title">Go</div>
<section>

```go,ignore
{{#include ../../snippets/go/send_payment.go:prepare-send-payment-spark}}
```
</section>
</custom-tabs>

<h2 id="sending-payments">
    <a class="header" href="#sending-payments">Sending Payments</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.send_payment">API docs</a>
</h2>

Once the payment has been prepared, pass the prepare response as an argument to the send method and set any selected payment options.

### Lightning

In the send payment options for BOLT11 invoices, you can set whether to prefer to use Spark to transfer the payment if the invoice contains a Spark address. By default, using Spark transfers are disabled.

By default, this function waits until the Lightning payment is complete before returning. You can override this behavior by specifying a timeout in seconds, after which the function will return even if the payment is still pending. This is useful if you want to display the payment as 'pending' after a certain period. Setting the timeout to 0 will cause the function to return a pending payment immediately after initiating it.

<custom-tabs category="lang">
<div slot="title">Rust</div>
<section>

```rust,ignore
{{#include ../../snippets/rust/src/send_payment.rs:send-payment-lightning-bolt11}}
```
</section>

<div slot="title">Swift</div>
<section>

```swift,ignore
{{#include ../../snippets/swift/BreezSdkSnippets/Sources/SendPayment.swift:send-payment-lightning-bolt11}}
```
</section>

<div slot="title">Kotlin</div>
<section>

```kotlin,ignore
{{#include ../../snippets/kotlin_mpp_lib/shared/src/commonMain/kotlin/com/example/kotlinmpplib/SendPayment.kt:send-payment-lightning-bolt11}}
```
</section>

<div slot="title">Javascript</div>
<section>

```typescript
{{#include ../../snippets/wasm/send_payment.ts:send-payment-lightning-bolt11}}
```
</section>

<div slot="title">React Native</div>
<section>

```typescript
{{#include ../../snippets/react-native/send_payment.ts:send-payment-lightning-bolt11}}
```
</section>

<div slot="title">Flutter</div>
<section>

```dart,ignore
{{#include ../../snippets/flutter/lib/send_payment.dart:send-payment-lightning-bolt11}}
```
</section>

<div slot="title">Python</div>
<section>

```python,ignore 
{{#include ../../snippets/python/src/send_payment.py:send-payment-lightning-bolt11}}
```
</section>

<div slot="title">Go</div>
<section>

```go,ignore
{{#include ../../snippets/go/send_payment.go:send-payment-lightning-bolt11}}
```
</section>
</custom-tabs>

### Bitcoin

For Bitcoin addresses you can set the confirmation speed in the send payment options. By default it's set to Fast.

<custom-tabs category="lang">
<div slot="title">Rust</div>
<section>

```rust,ignore
{{#include ../../snippets/rust/src/send_payment.rs:send-payment-onchain}}
```
</section>

<div slot="title">Swift</div>
<section>

```swift,ignore
{{#include ../../snippets/swift/BreezSdkSnippets/Sources/SendPayment.swift:send-payment-onchain}}
```
</section>

<div slot="title">Kotlin</div>
<section>

```kotlin,ignore
{{#include ../../snippets/kotlin_mpp_lib/shared/src/commonMain/kotlin/com/example/kotlinmpplib/SendPayment.kt:send-payment-onchain}}
```
</section>

<div slot="title">Javascript</div>
<section>

```typescript
{{#include ../../snippets/wasm/send_payment.ts:send-payment-onchain}}
```
</section>

<div slot="title">React Native</div>
<section>

```typescript
{{#include ../../snippets/react-native/send_payment.ts:send-payment-onchain}}
```
</section>

<div slot="title">Flutter</div>
<section>

```dart,ignore
{{#include ../../snippets/flutter/lib/send_payment.dart:send-payment-onchain}}
```
</section>

<div slot="title">Python</div>
<section>

```python,ignore 
{{#include ../../snippets/python/src/send_payment.py:send-payment-onchain}}
```
</section>

<div slot="title">Go</div>
<section>

```go,ignore
{{#include ../../snippets/go/send_payment.go:send-payment-onchain}}
```
</section>
</custom-tabs>

### Spark

For Spark addresses no send payment options are needed.

<custom-tabs category="lang">
<div slot="title">Rust</div>
<section>

```rust,ignore
{{#include ../../snippets/rust/src/send_payment.rs:send-payment-spark}}
```
</section>

<div slot="title">Swift</div>
<section>

```swift,ignore
{{#include ../../snippets/swift/BreezSdkSnippets/Sources/SendPayment.swift:send-payment-spark}}
```
</section>

<div slot="title">Kotlin</div>
<section>

```kotlin,ignore
{{#include ../../snippets/kotlin_mpp_lib/shared/src/commonMain/kotlin/com/example/kotlinmpplib/SendPayment.kt:send-payment-spark}}
```
</section>

<div slot="title">Javascript</div>
<section>

```typescript
{{#include ../../snippets/wasm/send_payment.ts:send-payment-spark}}
```
</section>

<div slot="title">React Native</div>
<section>

```typescript
{{#include ../../snippets/react-native/send_payment.ts:send-payment-spark}}
```
</section>

<div slot="title">Flutter</div>
<section>

```dart,ignore
{{#include ../../snippets/flutter/lib/send_payment.dart:send-payment-spark}}
```
</section>

<div slot="title">Python</div>
<section>

```python,ignore 
{{#include ../../snippets/python/src/send_payment.py:send-payment-spark}}
```
</section>

<div slot="title">Go</div>
<section>

```go,ignore
{{#include ../../snippets/go/send_payment.go:send-payment-spark}}
```
</section>
</custom-tabs>

## Event Flows
Once a send payment is initiated, you can follow and react to the different payment events using the guide below for each payment method. See [Listening to events](/guide/events.html) for how to subscribe to events.

| Event | Description | UX Suggestion |
| --- | --- | --- |
| **Synced** | The SDK has synced payments in the background.  | Update the payments list and balance. See [listing payments](/guide/list_payments.md) and [fetching the balance](/guide/get_info.md). |

### Lightning
| Event | Description | UX Suggestion |
| --- | --- | --- |
| **PaymentSucceeded** | The Lightning invoice has been paid either over Lightning or via a Spark transfer | Update the balance and show payment as complete. |
| **PaymentFailed** | The attempt to pay the Lightning invoice failed. |  |

### Bitcoin
| Event | Description | UX Suggestion |
| --- | --- | --- |
| **PaymentSucceeded** | The payment amount was successfully withdrawn onchain. | Update the balance and show payment as complete. |

### Spark
| Event | Description | UX Suggestion |
| --- | --- | --- |
| **PaymentSucceeded** | The Spark transfer is complete. | Update the balance and show payment as complete. |
