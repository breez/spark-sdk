# Seedless restore

Seedless restore enables wallet recovery using passkeys with the WebAuthn PRF extension, eliminating the need to backup mnemonic phrases. Wallet seeds are derived deterministically from a passkey and a user-chosen salt, with salts stored on Nostr relays for discovery during restore.

## Overview

The seedless restore flow uses two key derivations:

1. **Nostr Identity**: `PRF(passkey, magic_salt)` derives a Nostr keypair for salt storage
2. **Wallet Seed**: `PRF(passkey, user_salt)` derives a 24-word BIP39 mnemonic

Salts are published as Nostr kind-1 events, allowing users to discover their wallets on any device with access to their passkey.

<div class="warning">
<h4>Developer note</h4>
The passkey PRF functionality must be implemented by your application using platform-specific APIs (WebAuthn in browsers, native passkey APIs on mobile). The SDK orchestrates the flow but requires you to provide a PRF provider implementation.
</div>

## Implementing the PRF provider

Your application must implement the PRF provider to interface with platform passkey APIs.

{{#tabs seedless_restore:implement-prf-provider}}

<h2 id="creating-a-seed">
    <a class="header" href="#creating-a-seed">Creating a seed</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/seedless_restore/struct.SeedlessRestore.html#method.create_seed">API docs</a>
</h2>

To create a new seedless wallet, provide a user-chosen salt (e.g., "personal", "business"). The salt is published to Nostr for later discovery.

{{#tabs seedless_restore:create-seed}}

<h2 id="listing-salts">
    <a class="header" href="#listing-salts">Listing available salts</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/seedless_restore/struct.SeedlessRestore.html#method.list_salts">API docs</a>
</h2>

To restore a wallet, first query Nostr for salts associated with the passkey's identity.

{{#tabs seedless_restore:list-salts}}

<h2 id="restoring-a-seed">
    <a class="header" href="#restoring-a-seed">Restoring a seed</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/seedless_restore/struct.SeedlessRestore.html#method.restore_seed">API docs</a>
</h2>

Once you have the salt, restore the wallet seed.

{{#tabs seedless_restore:restore-seed}}

## Security considerations

- **Passkey security**: The wallet's security depends on the passkey. Different passkeys produce different wallets.
- **Salt visibility**: Salts are published publicly on Nostr. Security comes from the passkey secret, not the salt.
- **PRF availability**: Check `is_prf_available()` to gracefully handle devices without PRF support.
