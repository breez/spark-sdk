# Using LNURL-Auth

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.lnurl_auth

LNURL-Auth allows users to authenticate with services using their Lightning app, without requiring passwords or usernames. The Breez SDK supports LNURL-Auth following the LUD-04 and LUD-05 specifications.

## How it works

LNURL-Auth uses cryptographic key derivation to generate domain-specific keys, ensuring that:
- Each service gets a unique authentication key
- Your master key remains private
- Authentication is secure and passwordless

The SDK handles:
1. Domain-specific key derivation (LUD-05)
2. Challenge signing
3. Callback to the LNURL service

## Parsing LNURL-Auth URLs

After [parsing](parse.md) an LNURL-Auth URL, you'll receive an `LnurlAuthRequestDetails` object containing:
- **k1** - The authentication challenge (hex-encoded 32 bytes)
- **action** - Optional action type: `register`, `login`, `link`, or `auth`
- **domain** - The service domain requesting authentication
- **url** - The callback URL

### Rust

```rust
// LNURL-auth URL from a service
// Can be in the form:
// - lnurl1... (bech32 encoded)
// - https://service.com/lnurl-auth?tag=login&k1=...
let lnurl_auth_url = "lnurl1...";

if let Ok(InputType::LnurlAuth(request_data)) = sdk.parse(lnurl_auth_url).await {
    info!("Domain: {}", request_data.domain);
    info!("Action: {:?}", request_data.action);

    // Show domain to user and ask for confirmation
    // This is important for security
}
```

### Swift

```swift
// LNURL-auth URL from a service
// Can be in the form:
// - lnurl1... (bech32 encoded)
// - https://service.com/lnurl-auth?tag=login&k1=...
let lnurlAuthUrl = "lnurl1..."

if case .lnurlAuth(v1: let requestData) = try await sdk.parse(input: lnurlAuthUrl) {
    print("Domain: \(requestData.domain)")
    print("Action: \(String(describing: requestData.action))")

    // Show domain to user and ask for confirmation
    // This is important for security
}
```

### Kotlin

```kotlin
// LNURL-auth URL from a service
// Can be in the form:
// - lnurl1... (bech32 encoded)
// - https://service.com/lnurl-auth?tag=login&k1=...
val lnurlAuthUrl = "lnurl1..."

when (val inputType = sdk.parse(lnurlAuthUrl)) {
    is InputType.LnurlAuth -> {
        val requestData = inputType.v1
        println("Domain: ${requestData.domain}")
        println("Action: ${requestData.action}")

        // Show domain to user and ask for confirmation
        // This is important for security
    }
    else -> {}
}
```

### C#

```csharp
// LNURL-auth URL from a service
// Can be in the form:
// - lnurl1... (bech32 encoded)
// - https://service.com/lnurl-auth?tag=login&k1=...
var lnurlAuthUrl = "lnurl1...";

var inputType = await sdk.Parse(lnurlAuthUrl);
if (inputType is InputType.LnurlAuth lnurlAuth)
{
    var requestData = lnurlAuth.v1;
    Console.WriteLine($"Domain: {requestData.domain}");
    Console.WriteLine($"Action: {requestData.action}");

    // Show domain to user and ask for confirmation
    // This is important for security
}
```

### Javascript (Wasm)

```typescript
// LNURL-auth URL from a service
// Can be in the form:
// - lnurl1... (bech32 encoded)
// - https://service.com/lnurl-auth?tag=login&k1=...
const lnurlAuthUrl = 'lnurl1...'

const inputType = await sdk.parse(lnurlAuthUrl)
if (inputType.type === 'lnurlAuth') {
  console.log(`Domain: ${inputType.domain}`)
  console.log(`Action: ${inputType.action}`)

  // Show domain to user and ask for confirmation
  // This is important for security
}
```

### React Native

```typescript
// LNURL-auth URL from a service
// Can be in the form:
// - lnurl1... (bech32 encoded)
// - https://service.com/lnurl-auth?tag=login&k1=...
const lnurlAuthUrl = 'lnurl1...'

const inputType = await sdk.parse(lnurlAuthUrl)
if (inputType.tag === InputType_Tags.LnurlAuth) {
  const requestData = inputType.inner[0]
  console.log(`Domain: ${requestData.domain}`)
  console.log(`Action: ${requestData.action}`)

  // Show domain to user and ask for confirmation
  // This is important for security
}
```

### Flutter

```dart
// LNURL-auth URL from a service
// Can be in the form:
// - lnurl1... (bech32 encoded)
// - https://service.com/lnurl-auth?tag=login&k1=...
String lnurlAuthUrl = "lnurl1...";

InputType inputType = await sdk.parse(input: lnurlAuthUrl);
if (inputType is InputType_LnurlAuth) {
  LnurlAuthRequestDetails requestData = inputType.field0;
  print("Domain: ${requestData.domain}");
  print("Action: ${requestData.action}");

  // Show domain to user and ask for confirmation
  // This is important for security
}
```

### Python

```python
# LNURL-auth URL from a service
# Can be in the form:
# - lnurl1... (bech32 encoded)
# - https://service.com/lnurl-auth?tag=login&k1=...
lnurl_auth_url = "lnurl1..."

try:
    input_type = await sdk.parse(lnurl_auth_url)
    if isinstance(input_type, InputType.LNURL_AUTH):
        request_data = input_type[0]
        logging.debug(f"Domain: {request_data.domain}")
        logging.debug(f"Action: {request_data.action}")

        # Show domain to user and ask for confirmation
        # This is important for security
except Exception as error:
    logging.error(error)
    raise
```

### Go

```go
// LNURL-auth URL from a service
// Can be in the form:
// - lnurl1... (bech32 encoded)
// - https://service.com/lnurl-auth?tag=login&k1=...
lnurlAuthUrl := "lnurl1..."

inputType, err := sdk.Parse(lnurlAuthUrl)
if err == nil {
	if lnurlAuth, ok := inputType.(breez_sdk_spark.InputTypeLnurlAuth); ok {
		requestData := lnurlAuth.Field0
		log.Printf("Domain: %s", requestData.Domain)
		log.Printf("Action: %v", requestData.Action)

		// Show domain to user and ask for confirmation
		// This is important for security
	}
}
```



## Performing Authentication

Once you have the authentication request details, you can perform the authentication by passing the request to the `lnurl_auth` method. The SDK will:
1. Derive a domain-specific key pair
2. Sign the challenge with the derived key
3. Send the signature and public key to the service

### Rust

```rust
// Perform LNURL authentication
let result = sdk.lnurl_auth(request_data).await?;

match result {
    LnurlCallbackStatus::Ok => {
        info!("Authentication successful");
    }
    LnurlCallbackStatus::ErrorStatus { error_details } => {
        info!("Authentication failed: {}", error_details.reason);
    }
}
```

### Swift

```swift
// Perform LNURL authentication
let result = try await sdk.lnurlAuth(requestData: requestData)

switch result {
case .ok:
    print("Authentication successful")
case .errorStatus(errorDetails: let errorDetails):
    print("Authentication failed: \(errorDetails.reason)")
}
```

### Kotlin

```kotlin
// Perform LNURL authentication
when (val result = sdk.lnurlAuth(requestData)) {
    is LnurlCallbackStatus.Ok -> {
        println("Authentication successful")
    }
    is LnurlCallbackStatus.ErrorStatus -> {
        println("Authentication failed: ${result.errorDetails.reason}")
    }
}
```

### C#

```csharp
// Perform LNURL authentication
var result = await sdk.LnurlAuth(requestData);

if (result is LnurlCallbackStatus.Ok)
{
    Console.WriteLine("Authentication successful");
}
else if (result is LnurlCallbackStatus.ErrorStatus errorStatus)
{
    Console.WriteLine($"Authentication failed: {errorStatus.errorDetails.reason}");
}
```

### Javascript (Wasm)

```typescript
// Perform LNURL authentication
const result = await sdk.lnurlAuth(requestData)

if (result.type === 'ok') {
  console.log('Authentication successful')
} else if (result.type === 'errorStatus') {
  console.log(`Authentication failed: ${result.errorDetails.reason}`)
}
```

### React Native

```typescript
// Perform LNURL authentication
const result = await sdk.lnurlAuth(requestData)

if (result.tag === LnurlCallbackStatus_Tags.Ok) {
  console.log('Authentication successful')
} else if (result.tag === LnurlCallbackStatus_Tags.ErrorStatus) {
  console.log(`Authentication failed: ${result.inner.errorDetails.reason}`)
}
```

### Flutter

```dart
// Perform LNURL authentication
LnurlCallbackStatus result = await sdk.lnurlAuth(requestData: requestData);

if (result is LnurlCallbackStatus_Ok) {
  print("Authentication successful");
} else if (result is LnurlCallbackStatus_ErrorStatus) {
  print("Authentication failed: ${result.errorDetails.reason}");
}
```

### Python

```python
# Perform LNURL authentication
try:
    result = await sdk.lnurl_auth(request_data=request_data)

    if isinstance(result, LnurlCallbackStatus.OK):
        logging.debug("Authentication successful")
    elif isinstance(result, LnurlCallbackStatus.ERROR_STATUS):
        logging.debug(f"Authentication failed: {result.error_details.reason}")
except Exception as error:
    logging.error(error)
    raise
```

### Go

```go
// Perform LNURL authentication
result, err := sdk.LnurlAuth(requestData)
if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	log.Printf("Authentication error: %v", err)
	return
}

switch v := result.(type) {
case breez_sdk_spark.LnurlCallbackStatusOk:
	log.Println("Authentication successful")
case breez_sdk_spark.LnurlCallbackStatusErrorStatus:
	log.Printf("Authentication failed: %s", v.ErrorDetails.Reason)
}
```



**Developer note**

The SDK automatically derives domain-specific keys according to LUD-05, ensuring that each service gets a unique linking key. This protects user privacy by preventing services from correlating user identities across different domains.

## Action Types

LNURL-Auth supports different action types that indicate the purpose of the authentication:

- **register** - Create a new account
- **login** - Sign in to an existing account
- **link** - Link the Lightning wallet to an existing account
- **auth** - Generic authentication

Your application can use the `action` field to provide appropriate UI feedback to users.

## Security Considerations

- Always verify the domain before authenticating
- Show the domain to users for confirmation
- The SDK derives unique keys per domain to prevent tracking
- Authentication keys cannot be used to access funds

## Supported Specs

- [LUD-01](https://github.com/lnurl/luds/blob/luds/01.md) LNURL bech32 encoding
- [LUD-04](https://github.com/lnurl/luds/blob/luds/04.md) `auth` base spec
- [LUD-05](https://github.com/lnurl/luds/blob/luds/05.md) BIP32-based seed generation for `auth`
- [LUD-17](https://github.com/lnurl/luds/blob/luds/17.md) Support for lnurl auth 

---

Identifier casing: `get_info` here is `getInfo` in Swift, Kotlin, JavaScript, React Native and Flutter, and `GetInfo` in Go and C#. Enum variants: `SdkEvent::Synced` is `SdkEvent.SYNCED` in Python, `SdkEvent.synced` in Swift, `SdkEventSynced` in Go, and `SdkEvent.Synced` elsewhere.
