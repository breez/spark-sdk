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

Configure your domain in the SDK by passing the `LnurlDomain` parameter in the SDK configuration:

```csharp
var config = BreezSdkSparkMethods.DefaultConfig(Network.Mainnet) with
{
    apiKey = "your-api-key",
    lnurlDomain = "yourdomain.com"
};
```



## Managing Lightning addresses

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.check_lightning_address_available

The SDK provides several functions to manage Lightning addresses:

### Checking address availability

Before registering a Lightning address, you can check if the username is available. In your UI you can use a quick check mark to show the address is available before registering.

```csharp
var request = new CheckLightningAddressRequest(username: username);
var isAvailable = await sdk.CheckLightningAddressAvailable(request);
```



### Registering a Lightning address

Once you've confirmed a username is available, you can register it by passing a username and a description. The username will be used in `username@domain.com`. The description will be included in lnurl metadata and as the invoice description, so this is what the sender will see. The description is optional, and will default to `Pay to username@domain.com`.

> **Note:** Each user can have only one Lightning address per domain when using the Breez LNURL server. Registering a new address on the same domain will replace the previous one, but it won't be available to others.

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



### Retrieving Lightning address information

You can retrieve information about the currently registered Lightning address.

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



### Transferring a Lightning address

A user who already owns a registered Lightning address can hand it over to a different owner (pubkey) in a single atomic server operation: ownership is removed from the old pubkey and the new pubkey takes it in one step, without exposing a window during which the username could be snatched by a third party.

> **Note:** Existing payments are not transferred to the new owner. Only the address.

The flow has two steps, one method each, run by the current owner and then the new owner:

**Step 1: Current owner (pubkey A)** calls `AuthorizeLightningAddressTransfer` with the new owner's `IdentityPubkey` (which the new owner obtains via `GetInfo`). It returns a `TransferAuthorization` (carrying the `username`, A's `pubkey`, and `signature`), which grants B the right to take over the username.

> **Note:** Both owners sign the same canonical message (`"transfer:{username}-{pubkey_b}"`) with no timestamp, so A's authorization is a persistent capability for this specific (address, B) pair. Only B can actually submit the transfer, because the server also requires B's own signature over the same bytes; A's authorization alone doesn't let any third party move the username.

```csharp
var authorization = await currentOwnerSdk.AuthorizeLightningAddressTransfer(
    new AuthorizeTransferRequest(
        transfereePubkey: transfereePubkey));
```



The returned `TransferAuthorization` is then handed to the new owner over any channel. In an in-app migration, where a user moves their username from an old wallet to a new one, the app holds both SDK instances and passes it directly between them; to hand the username to a separate wallet, share it as a QR code or link. It already carries the username, so B needs nothing else to claim.

**Step 2: New owner (pubkey B)** calls `ClaimLightningAddressTransfer`, passing A's authorization. The SDK submits the transfer to the server which, in one transaction, verifies B's request signature, verifies A's authorization, and transfers ownership, returning the newly-owned `LightningAddressInfo`.

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



If pubkey B had a different username registered, it is replaced by the transferred one. The server rejects the call if pubkey A does not currently own the username (e.g. the name was already transferred to a third pubkey).

### Deleting a Lightning address

When a user no longer wants to use the Lightning address, you can delete it.

```csharp
await sdk.DeleteLightningAddress();
```



### Listening for Lightning address changes

When using the SDK on multiple devices, Lightning address changes made on one device are automatically synced to others. The SDK emits a `SdkEvent.LightningAddressChanged` event when a change from another device is detected, containing the updated `LightningAddressInfo` or no value if the address was deleted. See [Listening to events](./events.md) for how to subscribe to events.

## Accessing LNURL payment metadata

When receiving payments via LNURL-Pay or Lightning addresses, additional metadata may be included with the payment. This metadata is available on the received payment.

### Sender comment

If the sender includes a comment with their payment (as defined in [LUD-12](https://github.com/lnurl/luds/blob/luds/12.md)), it will be available on the received payment. This is the message that the sender wrote when making the payment.

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



### Nostr Zap request

If the payment was sent as a Nostr Zap (as defined in [NIP-57](https://github.com/nostr-protocol/nips/blob/master/57.md)), the received payment will include the zap request event. It carries the signed Nostr event (kind 9734) used to create the zap, and will also include the zap receipt event (kind 9735) once that has been created and published.

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



### Payment verification (LUD-21)

Payments received through your Lightning address support [LUD-21](https://github.com/lnurl/luds/blob/luds/21.md) invoice verification, allowing third parties to verify payment completion via a public verify URL.

## Payment notifications

You can receive webhook notifications when your users get paid via their Lightning Address. See [Lightning Address payment notifications](./lnurl_webhooks.md) for details.
