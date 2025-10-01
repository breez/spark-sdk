<h1 id="fetching-the-balance">
    <a class="header" href="#fetching-the-balance">Fetching the balance</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_info">API docs</a>
</h1>

Once connected, the balance can be retrieved at any time.

<custom-tabs category="lang">
<div slot="title">Rust</div>
<section>

```rust,ignore
{{#include ../../snippets/rust/src/getting_started.rs:fetch-balance}}
```
</section>

<div slot="title">Swift</div>
<section>

```swift,ignore
{{#include ../../snippets/swift/BreezSdkSnippets/Sources/GettingStarted.swift:fetch-balance}}
```
</section>

<div slot="title">Kotlin</div>
<section>

```kotlin,ignore
{{#include ../../snippets/kotlin_mpp_lib/shared/src/commonMain/kotlin/com/example/kotlinmpplib/GettingStarted.kt:fetch-balance}}
```
</section>

<div slot="title">Javascript</div>
<section>

```typescript
{{#include ../../snippets/wasm/getting_started.ts:fetch-balance}}
```
</section>

<div slot="title">React Native</div>
<section>

```typescript
{{#include ../../snippets/react-native/getting_started.ts:fetch-balance}}
```
</section>

<div slot="title">Flutter</div>
<section>

```dart,ignore
{{#include ../../snippets/flutter/lib/getting_started.dart:fetch-balance}}
```
</section>

<div slot="title">Python</div>
<section>

```python,ignore 
{{#include ../../snippets/python/src/getting_started.py:fetch-balance}}
```
</section>

<div slot="title">Go</div>
<section>

```go,ignore
{{#include ../../snippets/go/getting_started.go:fetch-balance}}
```
</section>
</custom-tabs>

<div class="warning">
<h4>Developer note</h4>
The SDK maintains a cached balance for fast responses and updates it on every change. The `get_info` call returns the value from this cache to provide a low-latency user experience.

Right after startup, the cache may not yet reflect the latest state from the network. Depends on your use case you can use one of the following options to get the fully up to date balance:

- If your application runs continuously in the background, call `get_info` after each `SdkEvent::Synced` event.
- If you're only briefly using the SDK to fetch the balance, call `get_info` with `ensure_synced = true` before disconnecting.

</div>