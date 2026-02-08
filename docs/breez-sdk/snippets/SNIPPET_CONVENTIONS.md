# Snippet Conventions

This document defines conventions for documentation snippets across all 9 supported languages. **Rust is the canonical reference** - all changes should be made to Rust first, then propagated to other languages.

## Languages

| Language | Directory | Extension | Naming |
|----------|-----------|-----------|--------|
| Rust | `rust/src/` | `.rs` | `snake_case` |
| Go | `go/` | `.go` | `PascalCase` |
| Python | `python/src/` | `.py` | `snake_case` |
| Kotlin | `kotlin_mpp_lib/shared/src/commonMain/kotlin/com/example/kotlinmpplib/` | `.kt` | `camelCase` |
| Swift | `swift/BreezSdkSnippets/Sources/` | `.swift` | `camelCase` |
| C# | `csharp/` | `.cs` | `PascalCase` |
| Flutter | `flutter/lib/` | `.dart` | `camelCase` |
| WASM | `wasm/` | `.ts` | `camelCase` |
| React Native | `react-native/` | `.ts` | `camelCase` |

## ANCHOR Markers

All languages use identical ANCHOR syntax (even Python):

```
// ANCHOR: anchor-name
[code]
// ANCHOR_END: anchor-name
```

Rules:
- Anchor names: **kebab-case**, identical across all languages
- Opening: `// ANCHOR: name` (colon after ANCHOR)
- Closing: `// ANCHOR_END: name` (underscore, no colon)
- Python uses `//` for anchors (not `#`) for tooling compatibility

## Function Signatures

### Rust
```rust
async fn function_name(sdk: &BreezSdk) -> Result<()> {
```

### Go
```go
func FunctionName(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.ResponseType, error) {
```

### Python
```python
async def function_name(sdk: BreezSdk):
```

### Kotlin
```kotlin
suspend fun functionName(sdk: BreezSdk) {
```

### Swift
```swift
func functionName(sdk: BreezSdk) async throws {
```

### C#
```csharp
async Task FunctionName(BreezSdk sdk) {
```

### Flutter
```dart
Future<ResponseType> functionName(BreezSdk sdk) async {
```

### WASM / React Native
```typescript
const exampleFunctionName = async (sdk: BreezSdk) => {
```

## Import Patterns

### Rust
```rust
use anyhow::Result;
use breez_sdk_spark::*;
use log::info;
```

### Go
```go
import (
    "log"
    "math/big"

    "github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)
```

### Python
```python
import logging
from breez_sdk_spark import (
    BreezSdk,
    SpecificType,
)
```

### Kotlin
```kotlin
import com.example.BreezSdk
import com.example.SpecificType
```

### Swift
```swift
import BreezSdkSpark
```

### C#
```csharp
using BreezSdkSpark;
```

### Flutter
```dart
import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';
```

### WASM
```typescript
import { type BreezSdk, SpecificType } from '@breeztech/breez-sdk-spark'
```

### React Native
```typescript
import { type BreezSdk, SpecificType, SpecificType_Tags } from '@breeztech/breez-sdk-spark-react-native'
```

## Simple Enum Matching (no associated data)

For enums without associated data (e.g., `ServiceStatus`, `Network`), use direct comparison or switch:

### Rust
```rust
match status {
    ServiceStatus::Operational => { /* ... */ }
    ServiceStatus::Degraded => { /* ... */ }
}
```

### Go
```go
switch status {
case breez_sdk_spark.ServiceStatusOperational:
    log.Printf("Operational")
case breez_sdk_spark.ServiceStatusDegraded:
    log.Printf("Degraded")
}
```

### Python
```python
if status == ServiceStatus.OPERATIONAL:
    logging.debug("Operational")
elif status == ServiceStatus.DEGRADED:
    logging.debug("Degraded")
```

### Kotlin
```kotlin
when (status) {
    ServiceStatus.OPERATIONAL -> { /* ... */ }
    ServiceStatus.DEGRADED -> { /* ... */ }
}
```

### Swift
```swift
switch status {
case .operational:
    print("Operational")
case .degraded:
    print("Degraded")
}
```

### C#
```csharp
switch (status) {
    case ServiceStatus.Operational:
        Console.WriteLine("Operational");
        break;
    case ServiceStatus.Degraded:
        Console.WriteLine("Degraded");
        break;
}
```

### Flutter
```dart
switch (status) {
    case ServiceStatus.operational:
        print("Operational");
        break;
    case ServiceStatus.degraded:
        print("Degraded");
        break;
}
```

### WASM
```typescript
switch (status) {
    case 'operational': { /* ... */ break }
    case 'degraded': { /* ... */ break }
}
```

### React Native
```typescript
switch (status) {
    case ServiceStatus.Operational: { /* ... */ break }
    case ServiceStatus.Degraded: { /* ... */ break }
}
```

## Enum/Type Discrimination (with associated data)

### Rust - if let / match
```rust
if let PaymentMethod::Bolt11Invoice { fee_sats, .. } = response.payment_method {
    info!("Fee: {fee_sats} sats");
}
```

### Go - type switch
```go
switch method := response.PaymentMethod.(type) {
case breez_sdk_spark.PaymentMethodBolt11Invoice:
    log.Printf("Fee: %v sats", method.FeeSats)
}
```

### Python - isinstance
```python
if isinstance(response.payment_method, PaymentMethod.BOLT11_INVOICE):
    logging.debug(f"Fee: {response.payment_method.fee_sats} sats")
```

### Kotlin - is
```kotlin
if (response.paymentMethod is PaymentMethod.Bolt11Invoice) {
    val feeSats = (response.paymentMethod as PaymentMethod.Bolt11Invoice).feeSats
}
```

### Swift - if case let
```swift
if case let .bolt11Invoice(feeSats, _) = response.paymentMethod {
    print("Fee: \(feeSats) sats")
}
```

### C# - pattern matching
```csharp
if (response.paymentMethod is PaymentMethod.Bolt11Invoice bolt11) {
    Console.WriteLine($"Fee: {bolt11.feeSats} sats");
}
```

### Flutter - is
```dart
if (response.paymentMethod is PaymentMethod_Bolt11Invoice) {
    final feeSats = (response.paymentMethod as PaymentMethod_Bolt11Invoice).feeSats;
    print("Fee: $feeSats sats");
}
```

### WASM - type discriminant
```typescript
if (response.paymentMethod.type === 'bolt11Invoice') {
    console.log(`Fee: ${response.paymentMethod.feeSats} sats`)
}
```

### React Native - tag discriminant
```typescript
if (response.paymentMethod?.tag === PaymentMethod_Tags.Bolt11Invoice) {
    console.log(`Fee: ${response.paymentMethod.inner.feeSats} sats`)
}
```

## Logging

| Language | Pattern |
|----------|---------|
| Rust | `info!("Message: {variable}");` |
| Go | `log.Printf("Message: %v", variable)` |
| Python | `logging.debug(f"Message: {variable}")` |
| Kotlin | `// Log.v("Breez", "Message: $variable")` (commented) |
| Swift | `print("Message: \(variable)")` |
| C# | `Console.WriteLine($"Message: {variable}");` |
| Flutter | `print("Message: $variable");` |
| WASM | `console.log(\`Message: ${variable}\`)` |
| React Native | `console.log(\`Message: ${variable}\`)` |

**TypeScript linting (WASM & React Native):** ESLint enforces single quotes. Only use backtick template literals when there is string interpolation (`${...}`). For static strings, use single quotes:
```typescript
// CORRECT:
console.log('Spark is fully operational')
console.log(`Balance: ${balanceSats} sats`)

// WRONG (lint error - no interpolation, must use single quotes):
console.log(`Spark is fully operational`)
```

## Error Handling

### Rust
```rust
let response = sdk.method().await?;
```

### Go
```go
response, err := sdk.Method(request)
if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
    return nil, err
}
```

### Python
```python
try:
    response = await sdk.method(request=request)
except Exception as error:
    logging.error(error)
    raise
```

### Kotlin
```kotlin
try {
    val response = sdk.method(request)
} catch (e: Exception) {
    // handle error
}
```

### Swift
```swift
do {
    let response = try await sdk.method(request: request)
} catch {
    // handle error
}
```

### C# / Flutter / WASM / React Native
No explicit try-catch in most snippets (implicit async error propagation).

## Optional Values

Use `optional` prefix consistently:
- Rust/Python: `optional_amount_sats`
- Others: `optionalAmountSats`

## Request Construction

### Rust
```rust
let request = RequestType {
    field: value,
    optional_field: Some(value),
};
```

### Go
```go
request := breez_sdk_spark.RequestType{
    Field: value,
    OptionalField: &optionalValue,
}
```

### Python
```python
request = RequestType(
    field=value,
    optional_field=optional_value,
)
```

### Kotlin
```kotlin
val request = RequestType(field, optionalField)
```

### Swift
```swift
let request = RequestType(field: value, optionalField: optionalValue)
```

### C#
```csharp
var request = new RequestType(
    field: value,
    optionalField: optionalValue
);
```

### Flutter
```dart
final request = RequestType(
    field: value,
    optionalField: optionalValue);
```

### WASM
```typescript
const response = await sdk.method({
    field: value,
    optionalField: optionalValue
})
```

### React Native
```typescript
const response = await sdk.method({
    field: value,
    optionalField: optionalValue
})
```

## Verification

Run after each language change:
```bash
# First time after SDK interface change:
cargo xtask check-doc-snippets --package <language>

# Subsequent runs (faster):
cargo xtask check-doc-snippets --package <language> --skip-build
```

Languages: `rust`, `go`, `python`, `kotlin-mpp`, `swift`, `csharp`, `flutter`, `wasm`, `react-native`

**Node.js Requirement:** WASM and React Native require Node >= 22.
```bash
# Set Node version if nvm is available:
command -v nvm && nvm use 22 || true
```
