# Receiving payments using LNURL-Pay and Lightning addresses

## What is a Lightning address?

A Lightning address is a human-readable identifier formatted like an email address (e.g., `user@domain.com`) that can be used to receive Bitcoin payments over the Lightning Network. Behind the scenes, it uses the LNURL-Pay protocol to dynamically generate invoices when someone wants to send a payment to this address.

## Configuring a custom domain

To use Lightning addresses with the Breez SDK, you first need to supply a domain. There are two options:

1. **Use a hosted LNURL server**: You can have your custom domain configured to an LNURL server run by Breez.
2. **Self-hosted LNURL server**: You can run your own [LNURL server](https://github.com/breez/spark-sdk/tree/main/crates/breez-sdk/lnurl) in a self-hosted environment.

In case you choose to point your domain to a hosted LNURL server, you will need to add a CNAME record in your domain's DNS settings.

> **Note:**: If you're using Cloudflare, make sure the CNAME record is set to 'DNS only' (not 'Proxied').

**Option 1: Using your domain without any subdomain**

This points yourdomain.com directly to the LNURL server. Some DNS providers do not support this method. If yours doesn't support CNAME or ALIAS records for the root domain, you will need to configure your domain at the registrar level to use an external DNS provider (like Google Cloud DNS).
* **Host/Name**: @
* **Type**: CNAME (or ALIAS if available)
* **Value/Target**: breez.tips

**Option 2: Using a subdomain**
This points a subdomain like pay.yourdomain.com to the LNURL server.
* **Host/Name**: pay (or your chosen prefix like payment, tip, donate)
* **Type**: CNAME
* **Value/Target**: breez.tips

[Send us](mailto:contact@breez.technology) your domain name (e.g., yourdomain.com or pay.yourdomain.com).

We will verify and add it to our list of allowed domains.

## Configuring Lightning addresses for users

Configure your domain in the SDK by passing the `lnurl_domain` parameter in the SDK configuration:

### Rust

```rust
let mut config = default_config(Network::Mainnet);
config.api_key = Some("your-api-key".to_string());
config.lnurl_domain = Some("yourdomain.com".to_string());
```

### Swift

```swift
var config = defaultConfig(network: Network.mainnet)
config.apiKey = "your-api-key"
config.lnurlDomain = "yourdomain.com"
```

### Kotlin

```kotlin
val config = defaultConfig(Network.MAINNET)
config.apiKey = "your-api-key"
config.lnurlDomain = "yourdomain.com"
```

### C#

```csharp
var config = BreezSdkSparkMethods.DefaultConfig(Network.Mainnet) with
{
    apiKey = "your-api-key",
    lnurlDomain = "yourdomain.com"
};
```

### Javascript (Wasm)

```typescript
const config = defaultConfig('mainnet')
config.apiKey = 'your-api-key'
config.lnurlDomain = 'yourdomain.com'
```

### React Native

```typescript
const config = defaultConfig(Network.Mainnet)
config.apiKey = 'your-api-key'
config.lnurlDomain = 'yourdomain.com'
```

### Flutter

```dart
final config = defaultConfig(network: Network.mainnet)
    .copyWith(
      apiKey: 'your-api-key',
      lnurlDomain: 'yourdomain.com'
    );
```

### Python

```python
config = default_config(network=Network.MAINNET)
config.api_key = "your-api-key"
config.lnurl_domain = "yourdomain.com"
```

### Go

```go
lnurlDomain := "yourdomain.com"
apiKey := "your-api-key"
config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
config.ApiKey = &apiKey
config.LnurlDomain = &lnurlDomain
```



## Managing Lightning addresses

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.check_lightning_address_available

The SDK provides several functions to manage Lightning addresses:

### Checking address availability

Before registering a Lightning address, you can check if the username is available. In your UI you can use a quick check mark to show the address is available before registering.

#### Rust

```rust
let request = CheckLightningAddressRequest { username };

let is_available = sdk.check_lightning_address_available(request).await?;
```

#### Swift

```swift
let request = CheckLightningAddressRequest(
    username: username
)

let available = try await sdk.checkLightningAddressAvailable(req: request)
```

#### Kotlin

```kotlin
val request = CheckLightningAddressRequest(
    username = username
)

val available = sdk.checkLightningAddressAvailable(request)
```

#### C#

```csharp
var request = new CheckLightningAddressRequest(username: username);
var isAvailable = await sdk.CheckLightningAddressAvailable(request);
```

#### Javascript (Wasm)

```typescript
const request = {
  username
}

const available = await sdk.checkLightningAddressAvailable(request)
```

#### React Native

```typescript
const request = {
  username
}

const available = await sdk.checkLightningAddressAvailable(request)
```

#### Flutter

```dart
final request = CheckLightningAddressRequest(
  username: username,
);

final available = await sdk.checkLightningAddressAvailable(request: request);
```

#### Python

```python
request = CheckLightningAddressRequest(username=username)
is_available = await sdk.check_lightning_address_available(request)
```

#### Go

```go
request := breez_sdk_spark.CheckLightningAddressRequest{
	Username: username,
}

isAvailable, err := sdk.CheckLightningAddressAvailable(request)
if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return false, err
}
```



### Registering a Lightning address

Once you've confirmed a username is available, you can register it by passing a username and a description. The username will be used in `username@domain.com`. The description will be included in lnurl metadata and as the invoice description, so this is what the sender will see. The description is optional, and will default to `Pay to username@domain.com`.

> **Note:** Each user can have only one Lightning address per domain when using the Breez LNURL server. Registering a new address on the same domain will replace the previous one, but it won't be available to others.

#### Rust

```rust
let request = RegisterLightningAddressRequest {
    username,
    description,
};

let address_info = sdk.register_lightning_address(request).await?;
let lightning_address = address_info.lightning_address;
let lnurl_url = address_info.lnurl.url;
let lnurl_bech32 = address_info.lnurl.bech32;
```

#### Swift

```swift
let request = RegisterLightningAddressRequest(
    username: username,
    description: description
)

let addressInfo = try await sdk.registerLightningAddress(request: request)
let lightningAddress = addressInfo.lightningAddress
let lnurlUrl = addressInfo.lnurl.url
let lnurlBech32 = addressInfo.lnurl.bech32
```

#### Kotlin

```kotlin
val request = RegisterLightningAddressRequest(
    username = username,
    description = description
)

val addressInfo = sdk.registerLightningAddress(request)
val lightningAddress = addressInfo.lightningAddress
val lnurlUrl = addressInfo.lnurl.url
val lnurlBech32 = addressInfo.lnurl.bech32
```

#### C#

```csharp
var request = new RegisterLightningAddressRequest(
    username: username,
    description: description
);

var addressInfo = await sdk.RegisterLightningAddress(request);
var lightningAddress = addressInfo.lightningAddress;
var lnurlUrl = addressInfo.lnurl.url;
var lnurlBech32 = addressInfo.lnurl.bech32;
```

#### Javascript (Wasm)

```typescript
const request = {
  username,
  description
}

const addressInfo = await sdk.registerLightningAddress(request)
const lightningAddress = addressInfo.lightningAddress
const lnurlUrl = addressInfo.lnurl.url
const lnurlBech32 = addressInfo.lnurl.bech32
```

#### React Native

```typescript
const request = {
  username,
  description
}

const addressInfo = await sdk.registerLightningAddress(request)
const lightningAddress = addressInfo.lightningAddress
const lnurlUrl = addressInfo.lnurl.url
const lnurlBech32 = addressInfo.lnurl.bech32
```

#### Flutter

```dart
final request = RegisterLightningAddressRequest(
  username: username,
  description: description,
);

final addressInfo = await sdk.registerLightningAddress(request: request);
final lightningAddress = addressInfo.lightningAddress;
final lnurlUrl = addressInfo.lnurl.url;
final lnurlBech32 = addressInfo.lnurl.bech32;
```

#### Python

```python
request = RegisterLightningAddressRequest(
    username=username,
    description=description
)

address_info = await sdk.register_lightning_address(request)
lightning_address = address_info.lightning_address
lnurl_url = address_info.lnurl.url
lnurl_bech32 = address_info.lnurl.bech32
```

#### Go

```go
request := breez_sdk_spark.RegisterLightningAddressRequest{
	Username:    username,
	Description: &description,
}

addressInfo, err := sdk.RegisterLightningAddress(request)
if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

_ = addressInfo.LightningAddress
_ = addressInfo.Lnurl.Url
_ = addressInfo.Lnurl.Bech32
```



### Retrieving Lightning address information

You can retrieve information about the currently registered Lightning address.

#### Rust

```rust
let address_info_opt = sdk.get_lightning_address().await?;

if let Some(info) = address_info_opt {
    let lightning_address = &info.lightning_address;
    let username = &info.username;
    let description = &info.description;
    let lnurl_url = &info.lnurl.url;
    let lnurl_bech32 = &info.lnurl.bech32;
}
```

#### Swift

```swift
if let addressInfo = try await sdk.getLightningAddress() {
    let lightningAddress = addressInfo.lightningAddress
    let username = addressInfo.username
    let description = addressInfo.description
    let lnurlUrl = addressInfo.lnurl.url
    let lnurlBech32 = addressInfo.lnurl.bech32
}
```

#### Kotlin

```kotlin
val addressInfoOpt = sdk.getLightningAddress()

if (addressInfoOpt != null) {
    val lightningAddress = addressInfoOpt.lightningAddress
    val username = addressInfoOpt.username
    val description = addressInfoOpt.description
    val lnurlUrl = addressInfoOpt.lnurl.url
    val lnurlBech32 = addressInfoOpt.lnurl.bech32
}
```

#### C#

```csharp
var addressInfoOpt = await sdk.GetLightningAddress();

if (addressInfoOpt != null)
{
    var lightningAddress = addressInfoOpt.lightningAddress;
    var username = addressInfoOpt.username;
    var description = addressInfoOpt.description;
    var lnurlUrl = addressInfoOpt.lnurl.url;
    var lnurlBech32 = addressInfoOpt.lnurl.bech32;
}
```

#### Javascript (Wasm)

```typescript
const addressInfoOpt = await sdk.getLightningAddress()

if (addressInfoOpt != null) {
  const lightningAddress = addressInfoOpt.lightningAddress
  const username = addressInfoOpt.username
  const description = addressInfoOpt.description
  const lnurlUrl = addressInfoOpt.lnurl.url
  const lnurlBech32 = addressInfoOpt.lnurl.bech32
}
```

#### React Native

```typescript
const addressInfoOpt = await sdk.getLightningAddress()

if (addressInfoOpt != null) {
  const lightningAddress = addressInfoOpt.lightningAddress
  const username = addressInfoOpt.username
  const description = addressInfoOpt.description
  const lnurlUrl = addressInfoOpt.lnurl.url
  const lnurlBech32 = addressInfoOpt.lnurl.bech32
}
```

#### Flutter

```dart
final addressInfoOpt = await sdk.getLightningAddress();

if (addressInfoOpt == null) {
  throw Exception("No Lightning Address registered for this user.");
}

final lightningAddress = addressInfoOpt.lightningAddress;
final username = addressInfoOpt.username;
final description = addressInfoOpt.description;
final lnurlUrl = addressInfoOpt.lnurl.url;
final lnurlBech32 = addressInfoOpt.lnurl.bech32;
```

#### Python

```python
address_info_opt = await sdk.get_lightning_address()

if address_info_opt is not None:
    lightning_address = address_info_opt.lightning_address
    username = address_info_opt.username
    description = address_info_opt.description
    lnurl_url = address_info_opt.lnurl.url
    lnurl_bech32 = address_info_opt.lnurl.bech32
```

#### Go

```go
addressInfoOpt, err := sdk.GetLightningAddress()
if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

if addressInfoOpt != nil {
	_ = addressInfoOpt.LightningAddress
	_ = addressInfoOpt.Username
	_ = addressInfoOpt.Description
	_ = addressInfoOpt.Lnurl.Url
	_ = addressInfoOpt.Lnurl.Bech32
}
```



### Transferring a Lightning address

A user who already owns a registered Lightning address can hand it over to a different owner (pubkey) in a single atomic server operation: ownership is removed from the old pubkey and the new pubkey takes it in one step, without exposing a window during which the username could be snatched by a third party.

> **Note:** Existing payments are not transferred to the new owner. Only the address.

The flow has two steps, one method each, run by the current owner and then the new owner:

**Step 1: Current owner (pubkey A)** calls `authorize_lightning_address_transfer` with the new owner's `identity_pubkey` (which the new owner obtains via `get_info`). It returns a `TransferAuthorization` (carrying the `username`, A's `pubkey`, and `signature`), which grants B the right to take over the username.

> **Note:** Both owners sign the same canonical message (`"transfer:{username}-{pubkey_b}"`) with no timestamp, so A's authorization is a persistent capability for this specific (address, B) pair. Only B can actually submit the transfer, because the server also requires B's own signature over the same bytes; A's authorization alone doesn't let any third party move the username.

#### Rust

```rust
let authorization = current_owner_sdk
    .authorize_lightning_address_transfer(AuthorizeTransferRequest {
        transferee_pubkey: transferee_pubkey.to_string(),
    })
    .await?;
```

#### Swift

```swift
let authorization = try await currentOwnerSdk.authorizeLightningAddressTransfer(
    request: AuthorizeTransferRequest(
        transfereePubkey: transfereePubkey
    )
)
```

#### Kotlin

```kotlin
val authorization = currentOwnerSdk.authorizeLightningAddressTransfer(
    AuthorizeTransferRequest(
        transfereePubkey = transfereePubkey,
    )
)
```

#### C#

```csharp
var authorization = await currentOwnerSdk.AuthorizeLightningAddressTransfer(
    new AuthorizeTransferRequest(
        transfereePubkey: transfereePubkey));
```

#### Javascript (Wasm)

```typescript
const authorization = await currentOwnerSdk.authorizeLightningAddressTransfer({
  transfereePubkey
})
```

#### React Native

```typescript
const authorization = await currentOwnerSdk.authorizeLightningAddressTransfer({
  transfereePubkey
})
```

#### Flutter

```dart
final authorization = await currentOwnerSdk.authorizeLightningAddressTransfer(
  request: AuthorizeTransferRequest(
    transfereePubkey: transfereePubkey,
  ),
);
```

#### Python

```python
request = AuthorizeTransferRequest(
    transferee_pubkey=transferee_pubkey
)

authorization = await current_owner_sdk.authorize_lightning_address_transfer(request)
```

#### Go

```go
request := breez_sdk_spark.AuthorizeTransferRequest{
	TransfereePubkey: transfereePubkey,
}

authorization, err := currentOwnerSdk.AuthorizeLightningAddressTransfer(request)
if err != nil {
	return nil, err
}
```



The returned `TransferAuthorization` is then handed to the new owner over any channel. In an in-app migration, where a user moves their username from an old wallet to a new one, the app holds both SDK instances and passes it directly between them; to hand the username to a separate wallet, share it as a QR code or link. It already carries the username, so B needs nothing else to claim.

**Step 2: New owner (pubkey B)** calls `claim_lightning_address_transfer`, passing A's authorization. The SDK submits the transfer to the server which, in one transaction, verifies B's request signature, verifies A's authorization, and transfers ownership, returning the newly-owned `LightningAddressInfo`.

#### Rust

```rust
let address = new_owner_sdk
    .claim_lightning_address_transfer(ClaimTransferRequest {
        authorization,
        description,
    })
    .await?;
let lightning_address = address.lightning_address;
let lnurl_url = address.lnurl.url;
let lnurl_bech32 = address.lnurl.bech32;
```

#### Swift

```swift
let addressInfo = try await newOwnerSdk.claimLightningAddressTransfer(
    request: ClaimTransferRequest(
        authorization: authorization,
        description: description
    )
)
let lightningAddress = addressInfo.lightningAddress
let lnurlUrl = addressInfo.lnurl.url
let lnurlBech32 = addressInfo.lnurl.bech32
```

#### Kotlin

```kotlin
val address = newOwnerSdk.claimLightningAddressTransfer(
    ClaimTransferRequest(
        authorization = authorization,
        description = description,
    )
)
val lightningAddress = address.lightningAddress
val lnurlUrl = address.lnurl.url
val lnurlBech32 = address.lnurl.bech32
```

#### C#

```csharp
var address = await newOwnerSdk.ClaimLightningAddressTransfer(
    new ClaimTransferRequest(
        authorization: authorization,
        description: description
    ));
var lightningAddress = address.lightningAddress;
var lnurlUrl = address.lnurl.url;
var lnurlBech32 = address.lnurl.bech32;
```

#### Javascript (Wasm)

```typescript
const addressInfo = await newOwnerSdk.claimLightningAddressTransfer({
  authorization,
  description
})
const lightningAddress = addressInfo.lightningAddress
const lnurlUrl = addressInfo.lnurl.url
const lnurlBech32 = addressInfo.lnurl.bech32
```

#### React Native

```typescript
const addressInfo = await newOwnerSdk.claimLightningAddressTransfer({
  authorization,
  description
})
const lightningAddress = addressInfo.lightningAddress
const lnurlUrl = addressInfo.lnurl.url
const lnurlBech32 = addressInfo.lnurl.bech32
```

#### Flutter

```dart
final address = await newOwnerSdk.claimLightningAddressTransfer(
  request: ClaimTransferRequest(
    authorization: authorization,
    description: description,
  ),
);
final lightningAddress = address.lightningAddress;
final lnurlUrl = address.lnurl.url;
final lnurlBech32 = address.lnurl.bech32;
```

#### Python

```python
request = ClaimTransferRequest(
    authorization=authorization,
    description=description
)

address_info = await new_owner_sdk.claim_lightning_address_transfer(request)
lightning_address = address_info.lightning_address
lnurl_url = address_info.lnurl.url
lnurl_bech32 = address_info.lnurl.bech32
```

#### Go

```go
request := breez_sdk_spark.ClaimTransferRequest{
	Authorization: authorization,
	Description:   &description,
}

address, err := newOwnerSdk.ClaimLightningAddressTransfer(request)
if err != nil {
	return nil, err
}

_ = address.LightningAddress
_ = address.Lnurl.Url
_ = address.Lnurl.Bech32
```



If pubkey B had a different username registered, it is replaced by the transferred one. The server rejects the call if pubkey A does not currently own the username (e.g. the name was already transferred to a third pubkey).

### Deleting a Lightning address

When a user no longer wants to use the Lightning address, you can delete it.

#### Rust

```rust
sdk.delete_lightning_address().await?;
```

#### Swift

```swift
try await sdk.deleteLightningAddress()
```

#### Kotlin

```kotlin
sdk.deleteLightningAddress()
```

#### C#

```csharp
await sdk.DeleteLightningAddress();
```

#### Javascript (Wasm)

```typescript
await sdk.deleteLightningAddress()
```

#### React Native

```typescript
await sdk.deleteLightningAddress()
```

#### Flutter

```dart
await sdk.deleteLightningAddress();
```

#### Python

```python
await sdk.delete_lightning_address()
```

#### Go

```go
err := sdk.DeleteLightningAddress()
if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return err
}
```



### Listening for Lightning address changes

When using the SDK on multiple devices, Lightning address changes made on one device are automatically synced to others. The SDK emits a `SdkEvent::LightningAddressChanged` event when a change from another device is detected, containing the updated `LightningAddressInfo` or no value if the address was deleted. See [Listening to events](./events.md) for how to subscribe to events.

## Accessing LNURL payment metadata

When receiving payments via LNURL-Pay or Lightning addresses, additional metadata may be included with the payment. This metadata is available on the received payment.

### Sender comment

If the sender includes a comment with their payment (as defined in [LUD-12](https://github.com/lnurl/luds/blob/luds/12.md)), it will be available on the received payment. This is the message that the sender wrote when making the payment.

#### Rust

```rust
// Check if this is a lightning payment with LNURL receive metadata
if let Some(PaymentDetails::Lightning {
    lnurl_receive_metadata: Some(metadata),
    ..
}) = payment.details
{
    // Access the sender comment if present
    if let Some(comment) = metadata.sender_comment {
        println!("Sender comment: {}", comment);
    }
}
```

#### Swift

```swift
// Check if this is a lightning payment with LNURL receive metadata
if case .lightning(let details) = payment.details {
    // Access the sender comment if present
    if let metadata = details.lnurlReceiveMetadata,
       let comment = metadata.senderComment {
        print("Sender comment: \(comment)")
    }
}
```

#### Kotlin

```kotlin
// Check if this is a lightning payment with LNURL receive metadata
if (payment.details is PaymentDetails.Lightning) {
    val details = payment.details as PaymentDetails.Lightning
    val metadata = details.lnurlReceiveMetadata

    // Access the sender comment if present
    metadata?.senderComment?.let { comment ->
        println("Sender comment: $comment")
    }
}
```

#### C#

```csharp
// Check if this is a lightning payment with LNURL receive metadata
if (payment.details is PaymentDetails.Lightning lightningDetails)
{
    var metadata = lightningDetails.lnurlReceiveMetadata;

    // Access the sender comment if present
    if (metadata?.senderComment != null)
    {
        Console.WriteLine($"Sender comment: {metadata.senderComment}");
    }
}
```

#### Javascript (Wasm)

```typescript
// Check if this is a lightning payment with LNURL receive metadata
if (payment.details?.type === 'lightning') {
  const metadata = payment.details.lnurlReceiveMetadata

  // Access the sender comment if present
  if (metadata?.senderComment != null) {
    console.log('Sender comment:', metadata.senderComment)
  }
}
```

#### React Native

```typescript
// Check if this is a lightning payment with LNURL receive metadata
if (payment.details?.tag === PaymentDetails_Tags.Lightning) {
  const metadata = payment.details.inner.lnurlReceiveMetadata

  // Access the sender comment if present
  if (metadata?.senderComment != null) {
    console.log('Sender comment:', metadata.senderComment)
  }
}
```

#### Flutter

```dart
// Check if this is a lightning payment with LNURL receive metadata
if (payment.details case PaymentDetails_Lightning lightningDetails) {
  final metadata = lightningDetails.lnurlReceiveMetadata;

  // Access the sender comment if present
  final comment = metadata?.senderComment;
  if (comment != null) {
    print('Sender comment: $comment');
  }
}
```

#### Python

```python
# Check if this is a lightning payment with LNURL receive metadata
if isinstance(payment.details, PaymentDetails.LIGHTNING):
    metadata = payment.details.lnurl_receive_metadata

    # Access the sender comment if present
    if metadata is not None and metadata.sender_comment is not None:
        print(f"Sender comment: {metadata.sender_comment}")
```

#### Go

```go
// Check if this is a lightning payment with LNURL receive metadata
if lightningDetails, ok := (*payment.Details).(breez_sdk_spark.PaymentDetailsLightning); ok {
	metadata := lightningDetails.LnurlReceiveMetadata

	// Access the sender comment if present
	if metadata != nil && metadata.SenderComment != nil {
		println("Sender comment:", *metadata.SenderComment)
	}
}
```



### Nostr Zap request

If the payment was sent as a Nostr Zap (as defined in [NIP-57](https://github.com/nostr-protocol/nips/blob/master/57.md)), the received payment will include the zap request event. It carries the signed Nostr event (kind 9734) used to create the zap, and will also include the zap receipt event (kind 9735) once that has been created and published.

#### Rust

```rust
// Check if this is a lightning payment with LNURL receive metadata
if let Some(PaymentDetails::Lightning {
    lnurl_receive_metadata: Some(metadata),
    ..
}) = payment.details
{
    // Access the Nostr zap request if present
    if let Some(zap_request) = metadata.nostr_zap_request {
        // The zap_request is a JSON string containing the Nostr event (kind 9734)
        println!("Nostr zap request: {}", zap_request);
    }

    // Access the Nostr zap receipt if present
    if let Some(zap_receipt) = metadata.nostr_zap_receipt {
        // The zap_receipt is a JSON string containing the Nostr event (kind 9735)
        println!("Nostr zap receipt: {}", zap_receipt);
    }
}
```

#### Swift

```swift
// Check if this is a lightning payment with LNURL receive metadata
if case .lightning(let details) = payment.details {
    if let metadata = details.lnurlReceiveMetadata {
        // Access the Nostr zap request if present
        if let zapRequest = metadata.nostrZapRequest {
            // The zapRequest is a JSON string containing the Nostr event (kind 9734)
            print("Nostr zap request: \(zapRequest)")
        }

        // Access the Nostr zap receipt if present
        if let zapReceipt = metadata.nostrZapReceipt {
            // The zapReceipt is a JSON string containing the Nostr event (kind 9735)
            print("Nostr zap receipt: \(zapReceipt)")
        }
    }
}
```

#### Kotlin

```kotlin
// Check if this is a lightning payment with LNURL receive metadata
if (payment.details is PaymentDetails.Lightning) {
    val details = payment.details as PaymentDetails.Lightning
    val metadata = details.lnurlReceiveMetadata

    // Access the Nostr zap request if present
    metadata?.nostrZapRequest?.let { zapRequest ->
        // The zapRequest is a JSON string containing the Nostr event (kind 9734)
        println("Nostr zap request: $zapRequest")
    }

    // Access the Nostr zap receipt if present
    metadata?.nostrZapReceipt?.let { zapReceipt ->
        // The zapReceipt is a JSON string containing the Nostr event (kind 9735)
        println("Nostr zap receipt: $zapReceipt")
    }
}
```

#### C#

```csharp
// Check if this is a lightning payment with LNURL receive metadata
if (payment.details is PaymentDetails.Lightning lightningDetails)
{
    var metadata = lightningDetails.lnurlReceiveMetadata;

    if (metadata != null)
    {
        // Access the Nostr zap request if present
        if (metadata.nostrZapRequest != null)
        {
            // The nostrZapRequest is a JSON string containing the Nostr event (kind 9734)
            Console.WriteLine($"Nostr zap request: {metadata.nostrZapRequest}");
        }

        // Access the Nostr zap receipt if present
        if (metadata.nostrZapReceipt != null)
        {
            // The nostrZapReceipt is a JSON string containing the Nostr event (kind 9735)
            Console.WriteLine($"Nostr zap receipt: {metadata.nostrZapReceipt}");
        }
    }
}
```

#### Javascript (Wasm)

```typescript
// Check if this is a lightning payment with LNURL receive metadata
if (payment.details?.type === 'lightning') {
  const metadata = payment.details.lnurlReceiveMetadata

  // Access the Nostr zap request if present
  if (metadata?.nostrZapRequest != null) {
    // The nostrZapRequest is a JSON string containing the Nostr event (kind 9734)
    console.log('Nostr zap request:', metadata.nostrZapRequest)
  }

  // Access the Nostr zap receipt if present
  if (metadata?.nostrZapReceipt != null) {
    // The nostrZapReceipt is a JSON string containing the Nostr event (kind 9735)
    console.log('Nostr zap receipt:', metadata.nostrZapReceipt)
  }
}
```

#### React Native

```typescript
// Check if this is a lightning payment with LNURL receive metadata
if (payment.details?.tag === PaymentDetails_Tags.Lightning) {
  const metadata = payment.details.inner.lnurlReceiveMetadata

  // Access the Nostr zap request if present
  if (metadata?.nostrZapRequest != null) {
    // The nostrZapRequest is a JSON string containing the Nostr event (kind 9734)
    console.log('Nostr zap request:', metadata.nostrZapRequest)
  }

  // Access the Nostr zap receipt if present
  if (metadata?.nostrZapReceipt != null) {
    // The nostrZapReceipt is a JSON string containing the Nostr event (kind 9735)
    console.log('Nostr zap receipt:', metadata.nostrZapReceipt)
  }
}
```

#### Flutter

```dart
// Check if this is a lightning payment with LNURL receive metadata
if (payment.details case PaymentDetails_Lightning lightningDetails) {
  final metadata = lightningDetails.lnurlReceiveMetadata;

  if (metadata != null) {
    // Access the Nostr zap request if present
    final zapRequest = metadata.nostrZapRequest;
    if (zapRequest != null) {
      // The zapRequest is a JSON string containing the Nostr event (kind 9734)
      print('Nostr zap request: $zapRequest');
    }

    // Access the Nostr zap receipt if present
    final zapReceipt = metadata.nostrZapReceipt;
    if (zapReceipt != null) {
      // The zapReceipt is a JSON string containing the Nostr event (kind 9735)
      print('Nostr zap receipt: $zapReceipt');
    }
  }
}
```

#### Python

```python
# Check if this is a lightning payment with LNURL receive metadata
if isinstance(payment.details, PaymentDetails.LIGHTNING):
    metadata = payment.details.lnurl_receive_metadata

    if metadata is not None:
        # Access the Nostr zap request if present
        if metadata.nostr_zap_request is not None:
            # The nostr_zap_request is a JSON string containing the Nostr event (kind 9734)
            print(f"Nostr zap request: {metadata.nostr_zap_request}")

        # Access the Nostr zap receipt if present
        if metadata.nostr_zap_receipt is not None:
            # The nostr_zap_receipt is a JSON string containing the Nostr event (kind 9735)
            print(f"Nostr zap receipt: {metadata.nostr_zap_receipt}")
```

#### Go

```go
// Check if this is a lightning payment with LNURL receive metadata
if lightningDetails, ok := (*payment.Details).(breez_sdk_spark.PaymentDetailsLightning); ok {
	metadata := lightningDetails.LnurlReceiveMetadata

	if metadata != nil {
		// Access the Nostr zap request if present
		if metadata.NostrZapRequest != nil {
			// The NostrZapRequest is a JSON string containing the Nostr event (kind 9734)
			println("Nostr zap request:", *metadata.NostrZapRequest)
		}

		// Access the Nostr zap receipt if present
		if metadata.NostrZapReceipt != nil {
			// The NostrZapReceipt is a JSON string containing the Nostr event (kind 9735)
			println("Nostr zap receipt:", *metadata.NostrZapReceipt)
		}
	}
}
```



### Payment verification (LUD-21)

Payments received through your Lightning address support [LUD-21](https://github.com/lnurl/luds/blob/luds/21.md) invoice verification, allowing third parties to verify payment completion via a public verify URL.

## Payment notifications

You can receive webhook notifications when your users get paid via their Lightning Address. See [Lightning Address payment notifications](./lnurl_webhooks.md) for details.


---

Identifier casing: `get_info` here is `getInfo` in Swift, Kotlin, JavaScript, React Native and Flutter, and `GetInfo` in Go and C#. Enum variants: `SdkEvent::Synced` is `SdkEvent.SYNCED` in Python, `SdkEvent.synced` in Swift, `SdkEventSynced` in Go, and `SdkEvent.Synced` elsewhere.
