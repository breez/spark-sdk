<h1 id="initializing">
    <a class="header" href="#initializing">Initializing the SDK</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.build">API docs</a>
</h1>

The first step is to construct the SDK using the SDK Builder. When using the SDK Builder you can configure:
- The network, mnemonic and Breez API key being used
- How the SDK storage is managed (the default storage uses file based SQLite storage)
- The chain service to use (bring-your-own or use the SDK's default)
- The REST client to use with LNURL requests (bring-your-own or use the SDK's default)

<div class="warning">
<h4>Developer note</h4>
For Wasm Web, SDK storage is managed using IndexedDB.
</div>


The SDK uses the storage to store the state of the SDK instance. When handling multiple instances of the SDK, each instance needs to have a different storage defined.

Now you are ready to interact with the SDK.

<custom-tabs category="lang">
<div slot="title">Rust</div>
<section>

```rust,ignore
{{#include ../../snippets/rust/src/getting_started.rs:init-sdk}}
```

</section>

<div slot="title">Swift</div>
<section>

```swift,ignore
{{#include ../../snippets/swift/BreezSdkSnippets/Sources/GettingStarted.swift:init-sdk}}
```

</section>

<div slot="title">Kotlin</div>
<section>

```kotlin,ignore
{{#include ../../snippets/kotlin_mpp_lib/shared/src/commonMain/kotlin/com/example/kotlinmpplib/GettingStarted.kt:init-sdk}}
```

</section>

<div slot="title">Javascript</div>
<section>

```typescript
{{#include ../../snippets/wasm/getting_started.ts:init-sdk}}
```

</section>

<div slot="title">Python</div>
<section>

```python,ignore 
{{#include ../../snippets/python/src/getting_started.py:init-sdk}}
```
</section>
</custom-tabs>

<div class="warning">
<h4>Developer note</h4>

Some platforms may require that you use an application specific directory that is writable within the application sandbox. For example applications running on Android or iOS.

</div>

<h2 id="disconnecting">
    <a class="header" href="#disconnecting">Disconnecting</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.disconnect">API docs</a>
</h2>

Once you are done using the SDK, you can call the `disconnect` method to free up the resources which are currently in use.

This is especially useful in cases where the SDK has to be re-instantiated, for example when you need to change the mnemonic and/or configuration.

<custom-tabs category="lang">
<div slot="title">Rust</div>
<section>

```rust,ignore
{{#include ../../snippets/rust/src/getting_started.rs:disconnect}}
```

</section>

<div slot="title">Swift</div>
<section>

```swift,ignore
{{#include ../../snippets/swift/BreezSdkSnippets/Sources/GettingStarted.swift:disconnect}}
```

</section>

<div slot="title">Kotlin</div>
<section>

```kotlin,ignore
{{#include ../../snippets/kotlin_mpp_lib/shared/src/commonMain/kotlin/com/example/kotlinmpplib/GettingStarted.kt:disconnect}}
```

</section>

<div slot="title">Javascript</div>
<section>

```typescript
{{#include ../../snippets/wasm/getting_started.ts:disconnect}}
```

</section>

<div slot="title">Python</div>
<section>

```python,ignore 
{{#include ../../snippets/python/src/getting_started.py:disconnect}}
```
</section>
</custom-tabs>
