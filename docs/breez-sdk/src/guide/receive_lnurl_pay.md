<h1 id="lightning-address">
    <a class="header" href="#lightning-address">Receiving payments using LNURL-Pay and Lightning addresses</a>
</h1>

<h2 id="what-is-lightning-address">
    <a class="header" href="#what-is-lightning-address">What is a Lightning address?</a>
</h2>

A Lightning address is a human-readable identifier formatted like an email address (e.g., `user@domain.com`) that can be used to receive Bitcoin payments over the Lightning Network. Behind the scenes, it uses the LNURL-Pay protocol to dynamically generate invoices when someone wants to send a payment to this address.

<h2 id="lnurl-server">
    <a class="header" href="#lnurl-server">Configuring a custom domain</a>
</h2>

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

Send us your domain name (e.g., yourdomain.com or pay.yourdomain.com).

We will verify and add it to our list of allowed domains.

<h2 id="configuring-lightning-address">
    <a class="header" href="#configuring-lightning-address">Configuring Lightning addresses for users</a>
</h2>

Configure your domain in the SDK by passing the {{#name lnurl_domain}} parameter in the SDK configuration:

{{#tabs lightning_address:config-lightning-address}}

<h2 id="managing-lightning-address">
    <a class="header" href="#managing-lightning-address">Managing Lightning addresses</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.check_lightning_address_available">API docs</a>
</h2>

The SDK provides several functions to manage Lightning addresses:

<h3 id="checking-availability">
    <a class="header" href="#checking-availability">Checking address availability</a>
</h3>

Before registering a Lightning address, you can check if the username is available. In your UI you can use a quick check mark to show the address is available before registering.

{{#tabs lightning_address:check-lightning-address}}

<h3 id="registering-address">
    <a class="header" href="#registering-address">Registering a Lightning address</a>
</h3>

Once you've confirmed a username is available, you can register it by passing a username and a description. The username will be used in `username@domain.com`. The description will be included in lnurl metadata and as the invoice description, so this is what the sender will see. The description is optional, and will default to `Pay to username@domain.com`.

{{#tabs lightning_address:register-lightning-address}}

<h3 id="retrieving-address">
    <a class="header" href="#retrieving-address">Retrieving Lightning address information</a>
</h3>

You can retrieve information about the currently registered Lightning address.

{{#tabs lightning_address:get-lightning-address}}

<h3 id="deleting-address">
    <a class="header" href="#deleting-address">Deleting a Lightning address</a>
</h3>

When a user no longer wants to use the Lightning address, you can delete it.

{{#tabs lightning_address:delete-lightning-address}}

<h2 id="lnurl-metadata">
    <a class="header" href="#lnurl-metadata">Accessing LNURL payment metadata</a>
</h2>

When receiving payments via LNURL-Pay or Lightning addresses, additional metadata may be included with the payment. This metadata is available on the received payment.

<h3 id="sender-comment">
    <a class="header" href="#sender-comment">Sender comment</a>
</h3>

If the sender includes a comment with their payment (as defined in [LUD-12](https://github.com/lnurl/luds/blob/luds/12.md)), it will be available on the received payment. This is the message that the sender wrote when making the payment.

{{#tabs lightning_address:access-sender-comment}}

<h3 id="nostr-zap">
    <a class="header" href="#nostr-zap">Nostr Zap request</a>
</h3>

If the payment was sent as a Nostr Zap (as defined in [NIP-57](https://github.com/nostr-protocol/nips/blob/master/57.md)), the received payment will include the zap request event. It carries the signed Nostr event (kind 9734) used to create the zap, and will also include the zap receipt event (kind 9735) once that has been created and published.

{{#tabs lightning_address:access-nostr-zap}}

> **Note:** When used in [private mode](./config.md#private-mode-enabled-by-default), the nostr zap receipt will be published by the SDK when online. When used in public mode, the zap receipt will be published by the LNURL server on your behalf.
