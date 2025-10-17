# Sending and receiving tokens

Spark supports tokens using the [BTKN protocol](https://docs.spark.money/lrc20/hello-btkn). The Breez SDK enables you to send and receive these tokens using the standard payments API.

<h2 id="fetching-the-balance">
    <a class="header" href="#fetching-the-balance">Fetching token balances</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_info">API docs</a>
</h2>

Once connected, the token balances and their metadata can be retrieved.

<custom-tabs category="lang">
<div slot="title">Rust</div>
<section>

```rust,ignore
{{#include ../../snippets/rust/src/tokens.rs:fetch-token-balances}}
```

</section>

<div slot="title">Swift</div>
<section>

```swift,ignore
{{#include ../../snippets/swift/BreezSdkSnippets/Sources/Tokens.swift:fetch-token-balances}}
```

</section>

<div slot="title">Kotlin</div>
<section>

```kotlin,ignore
{{#include ../../snippets/kotlin_mpp_lib/shared/src/commonMain/kotlin/com/example/kotlinmpplib/Tokens.kt:fetch-token-balances}}
```

</section>

<div slot="title">Javascript</div>
<section>

```typescript
{{#include ../../snippets/wasm/tokens.ts:fetch-token-balances}}
```

</section>

<div slot="title">React Native</div>
<section>

```typescript
{{#include ../../snippets/react-native/tokens.ts:fetch-token-balances}}
```

</section>

<div slot="title">Flutter</div>
<section>

```dart,ignore
{{#include ../../snippets/flutter/lib/tokens.dart:fetch-token-balances}}
```

</section>

<div slot="title">Python</div>
<section>

```python,ignore
{{#include ../../snippets/python/src/tokens.py:fetch-token-balances}}
```

</section>

<div slot="title">Go</div>
<section>

```go,ignore
{{#include ../../snippets/go/tokens.go:fetch-token-balances}}
```

</section>
</custom-tabs>

<div class="warning">
<h4>Developer note</h4>
Token balances are cached for fast responses. For details on ensuring up-to-date balances, see <a href="./get_info.md#fetching-the-balance">here</a> section.
</div>

<h2 id="fetching-token-metadata">
    <a class="header" href="#fetching-token-metadata">Fetching token metadata</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_tokens_metadata">API docs</a>
</h2>

The metadata of existing tokens can be fetched and will be cached for faster subsequent lookups.

<custom-tabs category="lang">
<div slot="title">Rust</div>
<section>

```rust,ignore
{{#include ../../snippets/rust/src/tokens.rs:fetch-token-metadata}}
```

</section>

<div slot="title">Swift</div>
<section>

```swift,ignore
{{#include ../../snippets/swift/BreezSdkSnippets/Sources/Tokens.swift:fetch-token-metadata}}
```

</section>

<div slot="title">Kotlin</div>
<section>

```kotlin,ignore
{{#include ../../snippets/kotlin_mpp_lib/shared/src/commonMain/kotlin/com/example/kotlinmpplib/Tokens.kt:fetch-token-metadata}}
```

</section>

<div slot="title">Javascript</div>
<section>

```typescript
{{#include ../../snippets/wasm/tokens.ts:fetch-token-metadata}}
```

</section>

<div slot="title">React Native</div>
<section>

```typescript
{{#include ../../snippets/react-native/tokens.ts:fetch-token-metadata}}
```

</section>

<div slot="title">Flutter</div>
<section>

```dart,ignore
{{#include ../../snippets/flutter/lib/tokens.dart:fetch-token-metadata}}
```

</section>

<div slot="title">Python</div>
<section>

```python,ignore
{{#include ../../snippets/python/src/tokens.py:fetch-token-metadata}}
```

</section>

<div slot="title">Go</div>
<section>

```go,ignore
{{#include ../../snippets/go/tokens.go:fetch-token-metadata}}
```

</section>
</custom-tabs>

<h2 id="receiving-payments">
    <a class="header" href="#receiving-payments">Receiving a token payment</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.receive_payment">API docs</a>
</h2>

A user just needs to share their Spark address in order to receive tokens. The Spark address can be fetched as described [here](./receive_payment.md#spark).

<h2 id="preparing-payments">
    <a class="header" href="#preparing-payments">Sending a token payment</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.prepare_send_payment">API docs</a>
</h2>

To send tokens, a Spark address should be provided as the payment request. The address may contain an embedded token identifier and an amount, in which case the SDK will process the payment accordingly, or a token identifier and amount have to be provided.

<custom-tabs category="lang">
<div slot="title">Rust</div>
<section>

```rust,ignore
{{#include ../../snippets/rust/src/tokens.rs:send-token-payment}}
```

</section>

<div slot="title">Swift</div>
<section>

```swift,ignore
{{#include ../../snippets/swift/BreezSdkSnippets/Sources/Tokens.swift:send-token-payment}}
```

</section>

<div slot="title">Kotlin</div>
<section>

```kotlin,ignore
{{#include ../../snippets/kotlin_mpp_lib/shared/src/commonMain/kotlin/com/example/kotlinmpplib/Tokens.kt:send-token-payment}}
```

</section>

<div slot="title">Javascript</div>
<section>

```typescript
{{#include ../../snippets/wasm/tokens.ts:send-token-payment}}
```

</section>

<div slot="title">React Native</div>
<section>

```typescript
{{#include ../../snippets/react-native/tokens.ts:send-token-payment}}
```

</section>

<div slot="title">Flutter</div>
<section>

```dart,ignore
{{#include ../../snippets/flutter/lib/tokens.dart:send-token-payment}}
```

</section>

<div slot="title">Python</div>
<section>

```python,ignore
{{#include ../../snippets/python/src/tokens.py:send-token-payment}}
```

</section>

<div slot="title">Go</div>
<section>

```go,ignore
{{#include ../../snippets/go/tokens.go:send-token-payment}}
```

</section>
</custom-tabs>

<h2 id="listing-payments">
    <a class="header" href="#listing-payments">Listing token payments</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.list_payments">API docs</a>
</h2>

Token payments can be listed just like other payments as described [here](./list_payments.md).
