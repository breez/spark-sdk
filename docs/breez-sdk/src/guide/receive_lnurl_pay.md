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

Configure your domain in the SDK by passing the {{#name lnurl_domain}} parameter in the SDK configuration:

{{#tabs lightning_address:config-lightning-address}}

<h2 id="managing-lightning-address">
    <a class="header" href="#managing-lightning-address">Managing Lightning addresses</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.check_lightning_address_available">API docs</a>
</h2>

The SDK provides several functions to manage Lightning addresses:

### Checking address availability

Before registering a Lightning address, you can check if the username is available. In your UI you can use a quick check mark to show the address is available before registering.

{{#tabs lightning_address:check-lightning-address}}

### Registering a Lightning address

Once you've confirmed a username is available, you can register it by passing a username and a description. The username will be used in `username@domain.com`. The description will be included in lnurl metadata and as the invoice description, so this is what the sender will see. The description is optional, and will default to `Pay to username@domain.com`.

> **Note:** Each user can have only one Lightning address per domain when using the Breez LNURL server. Registering a new address on the same domain will replace the previous one, making it available to others.

{{#tabs lightning_address:register-lightning-address}}

### Retrieving Lightning address information

You can retrieve information about the currently registered Lightning address.

{{#tabs lightning_address:get-lightning-address}}

### Transferring a Lightning address

A user who already owns a registered Lightning address can hand it over to a different owner (pubkey) in a single atomic server operation — ownership is removed from the old pubkey and the new pubkey takes it in one step, without exposing a window during which the username could be snatched by a third party.

The flow has two steps, run on two different SDKs:

**Step 1 — current owner (pubkey A):** produce a transfer authorization by signing a fixed message of the form `transfer:{pubkey_a}-{username}-{pubkey_b}`. Use {{#name sign_message}} on the SDK that currently owns the username. `pubkey_b` is the {{#name identity_pubkey}} of the receiving pubkey (available via {{#name get_info}}). The `username` must be the sanitized (lowercased and trimmed) form.

> **Note:** There is no timestamp in this message — A's authorization is a persistent capability for this specific A → B → username triple. Anyone holding it can submit the transfer, but it can only ever move the name to B.

{{#tabs lightning_address:sign-lightning-address-transfer}}

The pair `{pubkey, signature}` is then sent out-of-band to the new owner (e.g. via QR code, deep link, or a secure message).

**Step 2 — new owner (pubkey B):** call {{#name register_lightning_address}} with the `transfer` field populated. The SDK detects the field and routes the request to the server's atomic transfer endpoint instead of the regular register endpoint. In one transaction the server verifies B's request signature, verifies A's authorization, and swaps ownership.

{{#tabs lightning_address:register-lightning-address-transfer}}

If pubkey B had a different username registered, it is replaced by the transferred one. The server rejects the call if pubkey A does not currently own the username (e.g. the name was already transferred to a third pubkey).

### Deleting a Lightning address

When a user no longer wants to use the Lightning address, you can delete it.

{{#tabs lightning_address:delete-lightning-address}}

### Listening for Lightning address changes

When using the SDK on multiple devices, Lightning address changes made on one device are automatically synced to others. The SDK emits a {{#enum SdkEvent::LightningAddressChanged}} event when a change from another device is detected, containing the updated {{#name LightningAddressInfo}} or no value if the address was deleted. See [Listening to events](./events.md) for how to subscribe to events.

## Accessing LNURL payment metadata

When receiving payments via LNURL-Pay or Lightning addresses, additional metadata may be included with the payment. This metadata is available on the received payment.

### Sender comment

If the sender includes a comment with their payment (as defined in [LUD-12](https://github.com/lnurl/luds/blob/luds/12.md)), it will be available on the received payment. This is the message that the sender wrote when making the payment.

{{#tabs lightning_address:access-sender-comment}}

### Nostr Zap request

If the payment was sent as a Nostr Zap (as defined in [NIP-57](https://github.com/nostr-protocol/nips/blob/master/57.md)), the received payment will include the zap request event. It carries the signed Nostr event (kind 9734) used to create the zap, and will also include the zap receipt event (kind 9735) once that has been created and published.

{{#tabs lightning_address:access-nostr-zap}}

### Payment verification (LUD-21)

Payments received through your Lightning address support [LUD-21](https://github.com/lnurl/luds/blob/luds/21.md) invoice verification, allowing third parties to verify payment completion via a public verify URL.

## Payment notifications

You can receive webhook notifications when your users get paid via their Lightning Address. See [Lightning Address payment notifications](./lnurl_webhooks.md) for details.

