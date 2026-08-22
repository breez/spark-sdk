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

Configure your domain in the SDK by passing the `lnurlDomain` parameter in the SDK configuration:

```swift
var config = defaultConfig(network: Network.mainnet)
config.apiKey = "your-api-key"
config.lnurlDomain = "yourdomain.com"
```



## Managing Lightning addresses

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.check_lightning_address_available

The SDK provides several functions to manage Lightning addresses:

### Checking address availability

Before registering a Lightning address, you can check if the username is available. In your UI you can use a quick check mark to show the address is available before registering.

> **Note:** The answer is specific to this wallet. A username this wallet gave up earlier is reported as available to it, since only it can take that username back, while every other wallet is told the same username is unavailable.

> **Note:** Each check is signed with the wallet's identity key, so it costs a signing operation and a server round trip. Where signing is remote or needs user approval that is slow, so check once the user finishes typing rather than on every keystroke.

```swift
let request = CheckLightningAddressRequest(
    username: username
)

let available = try await sdk.checkLightningAddressAvailable(req: request)
```



### Registering a Lightning address

Once you've confirmed a username is available, you can register it by passing a username and a description. The username will be used in `username@domain.com`. The description will be included in lnurl metadata and as the invoice description, so this is what the sender will see. The description is optional, and will default to `Pay to username@domain.com`.

> **Note:** Each user can have only one Lightning address per domain when using the Breez LNURL server. Registering a new address on the same domain will replace the previous one, but it won't be available to others.

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



### Retrieving Lightning address information

You can retrieve information about the currently registered Lightning address.

```swift
if let addressInfo = try await sdk.getLightningAddress() {
    let lightningAddress = addressInfo.lightningAddress
    let username = addressInfo.username
    let description = addressInfo.description
    let lnurlUrl = addressInfo.lnurl.url
    let lnurlBech32 = addressInfo.lnurl.bech32
}
```



### Transferring a Lightning address

A user who already owns a registered Lightning address can hand it over to a different owner (pubkey) in a single atomic server operation: ownership is removed from the old pubkey and the new pubkey takes it in one step, without exposing a window during which the username could be snatched by a third party.

> **Note:** Existing payments are not transferred to the new owner. Only the address.

The flow has two steps, one method each, run by the current owner and then the new owner:

**Step 1: Current owner (pubkey A)** calls `authorizeLightningAddressTransfer` with the new owner's `identityPubkey` (which the new owner obtains via `getInfo`). It returns a `TransferAuthorization` (carrying the `username`, A's `pubkey`, `signature`, the `domain` the address is registered on, and the `timestamp` it was produced at), which grants B the right to take over the username.

> **Note:** The two owners sign different messages: A's names A as the outgoing owner and B as the incoming one, and B's names the description B is choosing, so neither signature can stand in for the other. Both cover the same `domain` and `timestamp`, so an authorization is only valid on the server it was made for, and only for 10 minutes. Only B can submit the transfer, because the server requires B's own signature alongside A's; A's authorization alone doesn't let any third party move the username.

```swift
let authorization = try await currentOwnerSdk.authorizeLightningAddressTransfer(
    request: AuthorizeTransferRequest(
        transfereePubkey: transfereePubkey
    )
)
```



The returned `TransferAuthorization` is then handed to the new owner over any channel. In an in-app migration, where a user moves their username from an old wallet to a new one, the app holds both SDK instances and passes it directly between them; to hand the username to a separate wallet, share it as a QR code or link. It carries everything B needs to claim.

Because the authorization expires 10 minutes after it is produced, generate it when B is ready to claim rather than ahead of time. If B claims too late, the call fails and A simply authorizes again.

> **Note:** Both wallets must be on an SDK version that signs the timestamped messages described above. A transfer between an older wallet and a newer one is rejected as an invalid signature.

**Step 2: New owner (pubkey B)** calls `claimLightningAddressTransfer`, passing A's authorization. The SDK submits the transfer to the server which, in one transaction, verifies B's request signature, verifies A's authorization, and transfers ownership, returning the newly-owned `LightningAddressInfo`.

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



If pubkey B had a different username registered, it is replaced by the transferred one and stays reserved for B. The server rejects the call if pubkey A does not currently own the username (e.g. the name was already transferred to a third pubkey).

### Deleting a Lightning address

When a user no longer wants to use the Lightning address, you can delete it.

> **Note:** The username stays reserved for this wallet after deletion. While the reservation stands no one else can register it, so senders who saved the old address are not redirected to a stranger, and the wallet can register it again later. How long a reservation stands is up to the server.

```swift
try await sdk.deleteLightningAddress()
```



### Listening for Lightning address changes

When using the SDK on multiple devices, Lightning address changes made on one device are automatically synced to others. The SDK emits a `SdkEvent.lightningAddressChanged` event when a change from another device is detected, containing the updated `LightningAddressInfo` or no value if the address was deleted. See [Listening to events](./events.md) for how to subscribe to events.

## Accessing LNURL payment metadata

When receiving payments via LNURL-Pay or Lightning addresses, additional metadata may be included with the payment. This metadata is available on the received payment.

### Sender comment

If the sender includes a comment with their payment (as defined in [LUD-12](https://github.com/lnurl/luds/blob/luds/12.md)), it will be available on the received payment. This is the message that the sender wrote when making the payment.

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



### Nostr Zap request

If the payment was sent as a Nostr Zap (as defined in [NIP-57](https://github.com/nostr-protocol/nips/blob/master/57.md)), the received payment will include the zap request event. It carries the signed Nostr event (kind 9734) used to create the zap, and will also include the zap receipt event (kind 9735) once that has been created and published.

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



### Payment verification (LUD-21)

Payments received through your Lightning address support [LUD-21](https://github.com/lnurl/luds/blob/luds/21.md) invoice verification, allowing third parties to verify payment completion via a public verify URL.

## Payment notifications

You can receive webhook notifications when your users get paid via their Lightning Address. See [Lightning Address payment notifications](./lnurl_webhooks.md) for details.
