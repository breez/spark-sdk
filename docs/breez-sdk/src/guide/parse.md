# Parsing inputs

The SDK provides a versatile parsing module designed to process a wide range of input strings and return parsed data in various standardized formats. 

Natively supported formats include: BOLT11 invoices, LNURLs of different types, Bitcoin addresses, and others. For the complete list, consult the [API documentation](https://breez.github.io/spark-sdk/breez_sdk_spark/enum.InputType.html).

<div class="warning">
<h4>Developer note</h4>
The amounts returned from calling parse on Lightning based inputs (BOLT11, LNURL) are denominated in millisatoshi.
</div>

<custom-tabs category="lang">
<div slot="title">Rust</div>
<section>

```rust,ignore
{{#include ../../snippets/rust/src/parsing_inputs.rs:parse-inputs}}
```
</section>

<div slot="title">Swift</div>
<section>

```swift,ignore
{{#include ../../snippets/swift/BreezSdkSnippets/Sources/ParsingInputs.swift:parse-inputs}}
```
</section>

<div slot="title">Kotlin</div>
<section>

```kotlin,ignore
{{#include ../../snippets/kotlin_mpp_lib/shared/src/commonMain/kotlin/com/example/kotlinmpplib/ParsingInputs.kt:parse-inputs}}
```
</section>

<div slot="title">Javascript</div>
<section>

```typescript
{{#include ../../snippets/wasm/parsing_inputs.ts:parse-inputs}}
```
</section>

<div slot="title">Flutter</div>
<section>

```dart,ignore
{{#include ../../snippets/flutter/lib/parsing_inputs.dart:parse-inputs}}
```
</section>

<div slot="title">Python</div>
<section>

```python,ignore 
{{#include ../../snippets/python/src/parsing_inputs.py:parse-inputs}}
```
</section>

<div slot="title">Go</div>
<section>

```go,ignore
{{#include ../../snippets/go/parsing_inputs.go:parse-inputs}}
```
</section>
</custom-tabs>
