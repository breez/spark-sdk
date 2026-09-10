# Supporting fiat currencies

## List fiat currencies

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.list_fiat_currencies

You can get the full details of supported fiat currencies, such as symbols and localized names:

### Rust

```rust
let response = sdk.list_fiat_currencies().await?;
```

### Swift

```swift
let response = try await sdk.listFiatCurrencies()
```

### Kotlin

```kotlin
try {
    val response = sdk.listFiatCurrencies()
} catch (e: Exception) {
    // handle error
}
```

### C#

```csharp
var response = await sdk.ListFiatCurrencies();
```

### Javascript (Wasm)

```typescript
const response = await sdk.listFiatCurrencies()
```

### React Native

```typescript
const response = await sdk.listFiatCurrencies()
```

### Flutter

```dart
ListFiatCurrenciesResponse response = await sdk.listFiatCurrencies();
```

### Python

```python
try:
    response = await sdk.list_fiat_currencies()
except Exception as error:
    print(error)
    raise
```

### Go

```go
response, err := sdk.ListFiatCurrencies()

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}
```



## Fetch fiat rates

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.list_fiat_rates

To get the current BTC rate in the various supported fiat currencies:

### Rust

```rust
let response = sdk.list_fiat_rates().await?;
```

### Swift

```swift
let response = try await sdk.listFiatRates()
```

### Kotlin

```kotlin
try {
    val response = sdk.listFiatRates()
} catch (e: Exception) {
    // handle error
}
```

### C#

```csharp
var response = await sdk.ListFiatRates();
```

### Javascript (Wasm)

```typescript
const response = await sdk.listFiatRates()
```

### React Native

```typescript
const response = await sdk.listFiatRates()
```

### Flutter

```dart
ListFiatRatesResponse response = await sdk.listFiatRates();
```

### Python

```python
try:
    response = await sdk.list_fiat_rates()
except Exception as error:
    print(error)
    raise
```

### Go

```go
response, err := sdk.ListFiatRates()

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}
```

