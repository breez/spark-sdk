# Using Turnkey

[Turnkey](https://www.turnkey.com/) keeps the wallet's keys inside a secure enclave. The SDK ships Turnkey-backed signers, so a server can run wallets without ever holding key material: signing happens inside Turnkey, and what the server holds is an API credential whose permissions you control with Turnkey policies.

Turnkey is meant for server deployments (see [Server mode](server_mode.md)). Depending on the policy you attach to the server's credential, it supports two ways of sending payments: the server signs everything itself, or each payment is approved by the end user via [Client signing](client_signing.md).

The SDK connects to an existing Spark wallet in your Turnkey organization or sub-organization. Creating the wallet itself is done with Turnkey directly and is out of the SDK's scope.

<h2 id="connecting">
    <a class="header" href="#connecting">Connecting</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/turnkey/fn.create_turnkey_signer.html">API docs</a>
</h2>

Create the signers with {{#name create_turnkey_signer}} and connect with {{#name connect_with_signer}}, the same way as any [external signer](external_signer.md):

{{#tabs turnkey:turnkey-connect}}

A few notes on the configuration:

- {{#name api_private_key}} is a server secret. It authenticates every Turnkey request; keep it out of client code and logs.
- The API key pair can be secp256k1 or P-256 (Turnkey's console default). All published bindings support both. If you use the Rust crate directly, enable the `turnkey` cargo feature, plus `turnkey-p256` for P-256 keys.
- {{#name max_rps}} paces requests to Turnkey. Unset uses Turnkey's documented limit of 10 requests per second per sub-organization; set it if your account has a different limit.

### Reconnecting without network calls

Server deployments often build a fresh SDK instance per request. Setting {{#name identity_public_key}} makes the signer setup network-free: after the first connect, read {{#name identity_pubkey}} from {{#name get_info}}, store it alongside the wallet, and pass it in the config on later connects. It is a stable, non-secret value, but it must belong to the same wallet.

### Wallets under a deny-export policy

{{#name create_turnkey_signer}} keeps every Spark key in the enclave, but exports one dedicated non-Spark key on first use for local encryption operations. If your Turnkey policy forbids any key export, use {{#name create_turnkey_signing_only_signer}} instead: no key is ever exported. Connect its signers as described in [Signers Without Local ECIES/HMAC Support](external_signer.md#signers-without-local-ecieshmac-support), which also lists the trade-offs of a signing-only signer.

## Signing models

How payments are authorized is decided by the Turnkey policy attached to each credential, not by SDK code. Configure the policies in Turnkey; see the [Turnkey policy documentation](https://docs.turnkey.com/concepts/policies/overview) for the mechanics.

### Server-side signing

The server's API credential is allowed to run all Spark signing activities. Every SDK flow then works exactly as documented, starting with [Sending payments](send_payment.md): the server prepares, signs and sends on its own. Use this when the server is trusted to send payments autonomously.

### User-approved payments

The policy allows the server's credential to run everything except the transfer approval activity (`SPARK_PREPARE_TRANSFER`), which requires the end user's own Turnkey credential (for example a passkey registered with Turnkey). The server then drives the send with the [Client signing](client_signing.md) flow: it prepares the payment and builds the package, the user approves and signs the one item that needs their credential, and the server publishes it. The rest of the signing (`SPARK_SIGN_FROST`) stays with the server under policy, so a payment can never be sent without the user, and no key leaves Turnkey on either side.

## Availability

- Turnkey signers are available on all platforms except Flutter, which does not support external signers (see [Using an External Signer](external_signer.md)).
- In the Rust crate the integration is behind the `turnkey` cargo feature. The published bindings ship with it enabled.
