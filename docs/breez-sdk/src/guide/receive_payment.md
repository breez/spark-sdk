# Receiving payments

Once the SDK is initialized, you can directly begin receiving payments. The receive process takes two steps:
1. [Preparing the Payment](receive_payment.md#preparing-payments)
1. [Receiving the Payment](receive_payment.md#receiving-payments-1)

<h2 id="preparing-payments">
    <a class="header" href="#preparing-payments">Preparing Payments</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.prepare_receive_payment">API docs</a>
</h2>

During the prepare step, the SDK ensures that the inputs are valid with respect to the specified payment method,
and also returns the relative fees related to the payment so they can be confirmed.


The SDK currently supports three methods of receiving: Lightning, Bitcoin and Spark.

### Lightning

#### BOLT11 invoice

When receiving via Lightning, we can generate a BOLT11 invoice to be paid.  Note that the payment may fallback to a direct Spark payment (if the payer's client supports this).

<custom-tabs category="lang">
<div slot="title">Rust</div>
<section>

```rust,ignore
{{#include ../../snippets/rust/src/receive_payment.rs:prepare-receive-payment-lightning}}
```
</section>

<div slot="title">Swift</div>
<section>

```swift,ignore
{{#include ../../snippets/swift/BreezSdkSnippets/Sources/ReceivePayment.swift:prepare-receive-payment-lightning}}
```
</section>

<div slot="title">Kotlin</div>
<section>

```kotlin,ignore
{{#include ../../snippets/kotlin_mpp_lib/shared/src/commonMain/kotlin/com/example/kotlinmpplib/ReceivePayment.kt:prepare-receive-payment-lightning}}
```
</section>

<div slot="title">Javascript</div>
<section>

```typescript
{{#include ../../snippets/wasm/receive_payment.ts:prepare-receive-payment-lightning}}
```
</section>

<div slot="title">Python</div>
<section>

```python,ignore 
{{#include ../../snippets/python/src/receive_payment.py:prepare-receive-payment-lightning}}
```
</section>
</custom-tabs>

### Bitcoin

<custom-tabs category="lang">
<div slot="title">Rust</div>
<section>

```rust,ignore
{{#include ../../snippets/rust/src/receive_payment.rs:prepare-receive-payment-onchain}}
```
</section>

<div slot="title">Swift</div>
<section>

```swift,ignore
{{#include ../../snippets/swift/BreezSdkSnippets/Sources/ReceivePayment.swift:prepare-receive-payment-onchain}}
```
</section>

<div slot="title">Kotlin</div>
<section>

```kotlin,ignore
{{#include ../../snippets/kotlin_mpp_lib/shared/src/commonMain/kotlin/com/example/kotlinmpplib/ReceivePayment.kt:prepare-receive-payment-onchain}}
```
</section>

<div slot="title">Javascript</div>
<section>

```typescript
{{#include ../../snippets/wasm/receive_payment.ts:prepare-receive-payment-onchain}}
```
</section>

<div slot="title">Python</div>
<section>

```python,ignore 
{{#include ../../snippets/python/src/receive_payment.py:prepare-receive-payment-onchain}}
```
</section>
</custom-tabs>

### Spark

<custom-tabs category="lang">
<div slot="title">Rust</div>
<section>

```rust,ignore
{{#include ../../snippets/rust/src/receive_payment.rs:prepare-receive-payment-spark}}
```
</section>

<div slot="title">Swift</div>
<section>

```swift,ignore
{{#include ../../snippets/swift/BreezSdkSnippets/Sources/ReceivePayment.swift:prepare-receive-payment-spark}}
```
</section>

<div slot="title">Kotlin</div>
<section>

```kotlin,ignore
{{#include ../../snippets/kotlin_mpp_lib/shared/src/commonMain/kotlin/com/example/kotlinmpplib/ReceivePayment.kt:prepare-receive-payment-spark}}
```
</section>

<div slot="title">Javascript</div>
<section>

```typescript
{{#include ../../snippets/wasm/receive_payment.ts:prepare-receive-payment-spark}}
```
</section>

<div slot="title">Python</div>
<section>

```python,ignore 
{{#include ../../snippets/python/src/receive_payment.py:prepare-receive-payment-spark}}
```
</section>
</custom-tabs>

<h2 id="receiving-payments">
    <a class="header" href="#receiving-payments">Receiving Payments</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.receive_payment">API docs</a>
</h2>

Once the payment has been prepared, all you have to do is pass the prepare response as an argument to the
receive method.


<custom-tabs category="lang">
<div slot="title">Rust</div>
<section>

```rust,ignore
{{#include ../../snippets/rust/src/receive_payment.rs:receive-payment}}
```
</section>

<div slot="title">Swift</div>
<section>

```swift,ignore
{{#include ../../snippets/swift/BreezSdkSnippets/Sources/ReceivePayment.swift:receive-payment}}
```
</section>

<div slot="title">Kotlin</div>
<section>

```kotlin,ignore
{{#include ../../snippets/kotlin_mpp_lib/shared/src/commonMain/kotlin/com/example/kotlinmpplib/ReceivePayment.kt:receive-payment}}
```
</section>

<div slot="title">Javascript</div>
<section>

```typescript
{{#include ../../snippets/wasm/receive_payment.ts:receive-payment}}
```
</section>

<div slot="title">Python</div>
<section>

```python,ignore 
{{#include ../../snippets/python/src/receive_payment.py:receive-payment}}
```
</section>
</custom-tabs>
