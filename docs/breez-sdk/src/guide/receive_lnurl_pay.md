<h1 id="lightning-address">
    <a class="header" href="#lightning-address">Receiving payments using LNURL-Pay and Lightning addresses</a>
</h1>

<h2 id="what-is-lightning-address">
    <a class="header" href="#what-is-lightning-address">What is a Lightning Address?</a>
</h2>

A Lightning Address is a human-readable identifier formatted like an email address (e.g., `user@domain.com`) that can be used to receive Bitcoin payments over the Lightning Network. Behind the scenes, it uses the LNURL-Pay protocol to dynamically generate invoices when someone wants to send a payment to this address.

<h2 id="lnurl-server">
    <a class="header" href="#lnurl-server">Configuring an LNURL server</a>
</h2>

To use Lightning Addresses with the Breez SDK, you first need to supply a domain. There are two options:

1. **Use a hosted LNURL server**: You can have your custom domain configured to an LNURL server run by Breez.
2. **Self-hosted LNURL server**: You can run your own LNURL server in a self-hosted environment.

In case you choose to point your domain to an hosted LNURL server, you will need to add a CNAME record in your domain’s DNS settings:

* **Host/Name**: @ (or the subdomain you want to use, e.g., www)
* **Type**: CNAME
* **Value/Target**: breez.tips

Send us your domain name (e.g., example.com or www.example.com).

We will verify and add it to our list of allowed domains.

<h2 id="configuring-lightning-address">
    <a class="header" href="#configuring-lightning-address">Configuring Lightning addresses for users</a>
</h2>

configure your domain in the SDK by passing the `lnurl_domain` parameter in the SDK configuration:

<custom-tabs category="lang">
<div slot="title">Rust</div>
<section>

```rust,ignore
{{#include ../../snippets/rust/src/lightning_address.rs:config-lightning-address}}
```
</section>

<div slot="title">Swift</div>
<section>

```swift,ignore
{{#include ../../snippets/swift/BreezSdkSnippets/Sources/LightningAddress.swift:config-lightning-address}}
```
</section>

<div slot="title">Kotlin</div>
<section>

```kotlin,ignore
{{#include ../../snippets/kotlin_mpp_lib/shared/src/commonMain/kotlin/com/example/kotlinmpplib/LightningAddress.kt:config-lightning-address}}
```
</section>

<div slot="title">Javascript</div>
<section>

```typescript,ignore
{{#include ../../snippets/wasm/lightning_address.ts:config-lightning-address}}
```
</section>

<div slot="title">Flutter</div>
<section>

```dart,ignore
{{#include ../../snippets/flutter/lib/lightning_address.dart:config-lightning-address}}
```
</section>

<div slot="title">Python</div>
<section>

```python,ignore
{{#include ../../snippets/python/src/lightning_address.py:config-lightning-address}}
```
</section>

<div slot="title">Go</div>
<section>

```go,ignore
{{#include ../../snippets/go/lightning_address.go:config-lightning-address}}
```
</section>
</custom-tabs>

<h2 id="managing-lightning-address">
    <a class="header" href="#managing-lightning-address">Managing Lightning Addresses</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.check_lightning_address_available">API docs</a>
</h2>

The SDK provides several functions to manage Lightning Addresses:

<h3 id="checking-availability">
    <a class="header" href="#checking-availability">Checking Address Availability</a>
</h3>

Before registering a Lightning Address, you can check if the username is available. In your UI you can use a quick check mark to show the address is available before registering.

<custom-tabs category="lang">
<div slot="title">Rust</div>
<section>

```rust,ignore
{{#include ../../snippets/rust/src/lightning_address.rs:check-lightning-address}}
```
</section>

<div slot="title">Swift</div>
<section>

```swift,ignore
{{#include ../../snippets/swift/BreezSdkSnippets/Sources/LightningAddress.swift:check-lightning-address}}
```
</section>

<div slot="title">Kotlin</div>
<section>

```kotlin,ignore
{{#include ../../snippets/kotlin_mpp_lib/shared/src/commonMain/kotlin/com/example/kotlinmpplib/LightningAddress.kt:check-lightning-address}}
```
</section>

<div slot="title">Javascript</div>
<section>

```typescript,ignore
{{#include ../../snippets/wasm/lightning_address.ts:check-lightning-address}}
```
</section>

<div slot="title">Flutter</div>
<section>

```dart,ignore
{{#include ../../snippets/flutter/lib/lightning_address.dart:check-lightning-address}}
```
</section>

<div slot="title">Python</div>
<section>

```python,ignore
{{#include ../../snippets/python/src/lightning_address.py:check-lightning-address}}
```
</section>

<div slot="title">Go</div>
<section>

```go,ignore
{{#include ../../snippets/go/lightning_address.go:check-lightning-address}}
```
</section>
</custom-tabs>

<h3 id="registering-address">
    <a class="header" href="#registering-address">Registering a Lightning Address</a>
</h3>

Once you've confirmed a username is available, you can register it by passing a username and a description. The username will be used in `username@domain.com`. The description will be included in lnurl metadata and as the invoice description, so this is what the sender will see.

<custom-tabs category="lang">
<div slot="title">Rust</div>
<section>

```rust,ignore
{{#include ../../snippets/rust/src/lightning_address.rs:register-lightning-address}}
```
</section>

<div slot="title">Swift</div>
<section>

```swift,ignore
{{#include ../../snippets/swift/BreezSdkSnippets/Sources/LightningAddress.swift:register-lightning-address}}
```
</section>

<div slot="title">Kotlin</div>
<section>

```kotlin,ignore
{{#include ../../snippets/kotlin_mpp_lib/shared/src/commonMain/kotlin/com/example/kotlinmpplib/LightningAddress.kt:register-lightning-address}}
```
</section>

<div slot="title">Javascript</div>
<section>

```typescript,ignore
{{#include ../../snippets/wasm/lightning_address.ts:register-lightning-address}}
```
</section>

<div slot="title">Flutter</div>
<section>

```dart,ignore
{{#include ../../snippets/flutter/lib/lightning_address.dart:register-lightning-address}}
```
</section>

<div slot="title">Python</div>
<section>

```python,ignore
{{#include ../../snippets/python/src/lightning_address.py:register-lightning-address}}
```
</section>

<div slot="title">Go</div>
<section>

```go,ignore
{{#include ../../snippets/go/lightning_address.go:register-lightning-address}}
```
</section>
</custom-tabs>

<h3 id="retrieving-address">
    <a class="header" href="#retrieving-address">Retrieving Lightning Address Information</a>
</h3>

You can retrieve information about the currently registered Lightning Address.

<custom-tabs category="lang">
<div slot="title">Rust</div>
<section>

```rust,ignore
{{#include ../../snippets/rust/src/lightning_address.rs:get-lightning-address}}
```
</section>

<div slot="title">Swift</div>
<section>

```swift,ignore
{{#include ../../snippets/swift/BreezSdkSnippets/Sources/LightningAddress.swift:get-lightning-address}}
```
</section>

<div slot="title">Kotlin</div>
<section>

```kotlin,ignore
{{#include ../../snippets/kotlin_mpp_lib/shared/src/commonMain/kotlin/com/example/kotlinmpplib/LightningAddress.kt:get-lightning-address}}
```
</section>

<div slot="title">Javascript</div>
<section>

```typescript,ignore
{{#include ../../snippets/wasm/lightning_address.ts:get-lightning-address}}
```
</section>

<div slot="title">Flutter</div>
<section>

```dart,ignore
{{#include ../../snippets/flutter/lib/lightning_address.dart:get-lightning-address}}
```
</section>

<div slot="title">Python</div>
<section>

```python,ignore
{{#include ../../snippets/python/src/lightning_address.py:get-lightning-address}}
```
</section>

<div slot="title">Go</div>
<section>

```go,ignore
{{#include ../../snippets/go/lightning_address.go:get-lightning-address}}
```
</section>
</custom-tabs>

<h3 id="deleting-address">
    <a class="header" href="#deleting-address">Deleting a Lightning Address</a>
</h3>

When a user no longer wants to use the Lightning Address, you can delete it.

<custom-tabs category="lang">
<div slot="title">Rust</div>
<section>

```rust,ignore
{{#include ../../snippets/rust/src/lightning_address.rs:delete-lightning-address}}
```
</section>

<div slot="title">Swift</div>
<section>

```swift,ignore
{{#include ../../snippets/swift/BreezSdkSnippets/Sources/LightningAddress.swift:delete-lightning-address}}
```
</section>

<div slot="title">Kotlin</div>
<section>

```kotlin,ignore
{{#include ../../snippets/kotlin_mpp_lib/shared/src/commonMain/kotlin/com/example/kotlinmpplib/LightningAddress.kt:delete-lightning-address}}
```
</section>

<div slot="title">Javascript</div>
<section>

```typescript,ignore
{{#include ../../snippets/wasm/lightning_address.ts:delete-lightning-address}}
```
</section>

<div slot="title">Flutter</div>
<section>

```dart,ignore
{{#include ../../snippets/flutter/lib/lightning_address.dart:delete-lightning-address}}
```
</section>

<div slot="title">Python</div>
<section>

```python,ignore
{{#include ../../snippets/python/src/lightning_address.py:delete-lightning-address}}
```
</section>

<div slot="title">Go</div>
<section>

```go,ignore
{{#include ../../snippets/go/lightning_address.go:delete-lightning-address}}
```
</section>
</custom-tabs>

