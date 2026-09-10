# Signing and verifying messages

Through signing and verifying messages we can provide proof that a digital signature was created by a private key.

## Signing a message

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.sign_message

By signing a message using the SDK we can provide a digital signature. Anyone with the `message`, `pubkey` and `signature` can verify the signature was created by the private key of this pubkey.

Messages starting with `breez-lnurl:` are refused: that namespace is reserved for requests to the Lightning address server.

### Rust

```rust
let message = "<message to sign>".to_string();
// Set to true to get a compact signature rather than a DER
let compact = true;

let sign_message_request = SignMessageRequest { message, compact };
let sign_message_response = sdk.sign_message(sign_message_request).await?;

let signature = sign_message_response.signature;
let pubkey = sign_message_response.pubkey;

info!("Pubkey: {}", pubkey);
info!("Signature: {}", signature);
```

### Swift

```swift
// Set to true to get a compact signature rather than a DER
let compact = true

let signMessageRequest = SignMessageRequest(
    message: "<message to sign>",
    compact: compact
)
let signMessageResponse = try await sdk
    .signMessage(request: signMessageRequest)

let signature = signMessageResponse.signature
let pubkey = signMessageResponse.pubkey

print("Pubkey: {}", pubkey);
print("Signature: {}", signature);
```

### Kotlin

```kotlin
val message = "<message to sign>"
// Set to true to get a compact signature rather than a DER
val compact = true
try {
    val signMessageRequest = SignMessageRequest(message, compact)
    val signMessageResponse = sdk.signMessage(signMessageRequest)

    val signature = signMessageResponse?.signature
    val pubkey = signMessageResponse?.pubkey

    // Log.v("Breez", "Pubkey: ${pubkey}")
    // Log.v("Breez", "Signature: ${signature}")
} catch (e: Exception) {
    // handle error
}
```

### C#

```csharp
var message = "<message to sign>";
// Set to true to get a compact signature rather than a DER
var compact = true;
var signMessageRequest = new SignMessageRequest(
    message: message,
    compact: compact
);
var signMessageResponse = await sdk.SignMessage(request: signMessageRequest);

var signature = signMessageResponse.signature;
var pubkey = signMessageResponse.pubkey;

Console.WriteLine($"Pubkey: {pubkey}");
Console.WriteLine($"Signature: {signature}");
```

### Javascript (Wasm)

```typescript
// Set to true to get a compact signature rather than a DER
const compact = true

const signMessageResponse = await sdk.signMessage({
  message: '<message to sign>',
  compact
})

const signature = signMessageResponse.signature
const pubkey = signMessageResponse.pubkey

console.log(`Pubkey: ${pubkey}`)
console.log(`Signature: ${signature}`)
```

### React Native

```typescript
// Set to true to get a compact signature rather than a DER
const compact = true

const signMessageResponse = await sdk.signMessage({
  message: '<message to sign>',
  compact
})

const signature = signMessageResponse.signature
const pubkey = signMessageResponse.pubkey

console.log(`Pubkey: ${pubkey}`)
console.log(`Signature: ${signature}`)
```

### Flutter

```dart
// Set to true to get a compact signature rather than a DER
bool compact = true;

SignMessageRequest signMessageRequest = SignMessageRequest(
  message: "<message to sign>",
  compact: compact,
);

SignMessageResponse signMessageResponse = await sdk.signMessage(
  request: signMessageRequest,
);

String signature = signMessageResponse.signature;
String pubkey = signMessageResponse.pubkey;

print("Pubkey: $pubkey");
print("Signature: $signature");
```

### Python

```python
message = "<message to sign>"
# Set to true to get a compact signature rather than a DER
compact = True
try:
    sign_message_request = SignMessageRequest(
        message=message, compact=compact
    )
    sign_message_response = await sdk.sign_message(request=sign_message_request)

    signature = sign_message_response.signature
    pubkey = sign_message_response.pubkey

    logging.debug(f"Pubkey: {pubkey}")
    logging.debug(f"Signature: {signature}")
except Exception as error:
    logging.error(error)
    raise
```

### Go

```go
message := "<message to sign>"
// Set to true to get a compact signature rather than a DER
compact := true

signMessageRequest := breez_sdk_spark.SignMessageRequest{
	Message: message,
	Compact: compact,
}
signMessageResponse, err := sdk.SignMessage(signMessageRequest)
if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

signature := signMessageResponse.Signature
pubkey := signMessageResponse.Pubkey

log.Printf("Pubkey: %v", pubkey)
log.Printf("Signature: %v", signature)
```



## Verifying a message

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.check_message

You can prove control of a private key by verifying a `message` with it's `signature` and `pubkey`.

### Rust

```rust
let check_message_request = CheckMessageRequest {
    message: "<message>".to_string(),
    pubkey: "<pubkey of signer>".to_string(),
    signature: "<message signature>".to_string(),
};
let check_message_response = sdk.check_message(check_message_request).await?;

let is_valid = check_message_response.is_valid;

info!("Signature valid: {}", is_valid);
```

### Swift

```swift
let checkMessageRequest = CheckMessageRequest(
    message: "<message>",
    pubkey: "<pubkey of signer>",
    signature: "<message signature>"
)
let checkMessageResponse = try await sdk
    .checkMessage(request: checkMessageRequest)

let isValid = checkMessageResponse.isValid

print("Signature valid: {}", isValid);
```

### Kotlin

```kotlin
val message = "<message>"
val pubkey = "<pubkey of signer>"
val signature = "<message signature>"
try {
    val checkMessageRequest = CheckMessageRequest(message, pubkey, signature)
    val checkMessageResponse = sdk.checkMessage(checkMessageRequest)

    val isValid = checkMessageResponse?.isValid

    // Log.v("Breez", "Signature valid: ${isValid}")
} catch (e: Exception) {
    // handle error
}
```

### C#

```csharp
var message = "<message>";
var pubkey = "<pubkey of signer>";
var signature = "<message signature>";
var checkMessageRequest = new CheckMessageRequest(
    message: message,
    pubkey: pubkey,
    signature: signature
);
var checkMessageResponse = await sdk.CheckMessage(request: checkMessageRequest);

var isValid = checkMessageResponse.isValid;

Console.WriteLine($"Signature valid: {isValid}");
```

### Javascript (Wasm)

```typescript
const checkMessageResponse = await sdk.checkMessage({
  message: '<message>',
  pubkey: '<pubkey of signer>',
  signature: '<message signature>'
})
const isValid = checkMessageResponse.isValid

console.log(`Signature valid: ${isValid}`)
```

### React Native

```typescript
const checkMessageResponse = await sdk.checkMessage({
  message: '<message>',
  pubkey: '<pubkey of signer>',
  signature: '<message signature>'
})
const isValid = checkMessageResponse.isValid

console.log(`Signature valid: ${isValid}`)
```

### Flutter

```dart
CheckMessageRequest checkMessageRequest = CheckMessageRequest(
  message: "<message>",
  pubkey: "<pubkey of signer>",
  signature: "<message signature>",
);

CheckMessageResponse checkMessageResponse = await sdk.checkMessage(
  request: checkMessageRequest,
);

bool isValid = checkMessageResponse.isValid;

print("Signature valid: $isValid");
```

### Python

```python
message = "<message>"
pubkey = "<pubkey of signer>"
signature = "<message signature>"
try:
    check_message_request = CheckMessageRequest(
        message=message, pubkey=pubkey, signature=signature
    )
    check_message_response = await sdk.check_message(request=check_message_request)

    is_valid = check_message_response.is_valid

    logging.debug(f"Signature valid: {is_valid}")
except Exception as error:
    logging.error(error)
    raise
```

### Go

```go
message := "<message>"
pubkey := "<pubkey of signer>"
signature := "<message signature>"

checkMessageRequest := breez_sdk_spark.CheckMessageRequest{
	Message:   message,
	Pubkey:    pubkey,
	Signature: signature,
}
checkMessageResponse, err := sdk.CheckMessage(checkMessageRequest)
if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

isValid := checkMessageResponse.IsValid

log.Printf("Signature valid: %v", isValid)
```

