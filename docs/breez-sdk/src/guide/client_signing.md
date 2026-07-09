# Client signing

Client signing lets a server drive payments while the key that approves them stays with the user. The server prepares the payment and builds a small package that describes it, the user reviews and signs the package on their side, and the server publishes it to complete the payment.

Use it when the SDK runs on your server, for example hosting wallets for many users, and the server must not be able to send payments on its own. It works for Spark addresses and invoices, Lightning invoices, token payments, Bitcoin addresses and LNURL payments.

Client signing is fully opt-in. Without it, {{#name send_payment}} works as described in [Sending payments](send_payment.md).

## How it works

1. **Prepare** on the server with {{#name prepare_send_payment}}, exactly as in [Sending payments](send_payment.md). This validates the input and returns the fees.
2. **Build** on the server with {{#name build_unsigned_transfer_package}}. This returns the one item the user needs to sign. It carries the amount, fee and destination of the payment.
3. **Sign** on the user's side. The user reviews the package and signs it with their signer.
4. **Publish** on the server with {{#name publish_signed_transfer_package}} to complete the payment.

Sometimes the wallet first needs to re-shape its funds so it can send the exact amount (a denomination swap). That swap also needs the user's signature, so it arrives as its own package: publishing it returns {{#enum PublishSignedTransferPackageResponse::SwapCompleted}}, and you build again from the same prepare response. Repeat until publishing returns {{#enum PublishSignedTransferPackageResponse::PaymentSent}}.

The server keeps no state between these steps. Everything needed to complete the payment travels inside the requests and responses, so building and publishing can happen in different processes or on different instances. This fits [Server mode](server_mode.md) deployments, where an SDK instance is built per request.

<h2 id="signing-on-the-users-side">
    <a class="header" href="#signing-on-the-users-side">Signing on the user's side</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/signer/trait.ExternalSparkSigner.html">API docs</a>
</h2>

The user's side does not need a connected SDK, only a signer that holds the user's key: any {{#name ExternalSparkSigner}} implementation (see [Using an External Signer](external_signer.md)), whether it runs on the user's device or fronts a remote signing service.

The package tells the user exactly what they are approving: the amount, the fee and the destination. Show these to the user before signing. Sign Transfer and Swap packages with {{#name prepare_transfer}}, and Token packages with {{#name prepare_token_transaction}}:

{{#tabs client_signing:client-signing-sign-package}}

<h2 id="driving-the-send">
    <a class="header" href="#driving-the-send">Driving the send from the server</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.build_unsigned_transfer_package">API docs</a>
</h2>

Prepare once, then repeat build, sign and publish until the payment is sent:

{{#tabs client_signing:client-signing-send}}

### Bitcoin

For Bitcoin addresses, choose the confirmation speed when building the package. The fee, and therefore what the user signs, depends on it:

{{#tabs client_signing:client-signing-build-onchain-options}}

### Lightning

For BOLT11 invoices the build options work like the send options in [Sending payments](send_payment.md#lightning-1): {{#name prefer_spark}} sends via a direct Spark transfer when the invoice also contains a Spark address, and {{#name completion_timeout_secs}} controls how long publishing waits for the payment to complete before returning it while still pending:

{{#tabs client_signing:client-signing-build-bolt11-options}}

### Tokens

Token payments follow the same loop. Prepare with a token identifier as in [Token payments](token_payments.md). The package amounts are in the token's base units, and the user signs with {{#name prepare_token_transaction}}. A Token package with {{#name is_swap}} set means the wallet first needs to combine token outputs: publishing it returns {{#enum PublishSignedTransferPackageResponse::SwapCompleted}}, just like the Bitcoin case.

<h2 id="lnurl-pay">
    <a class="header" href="#lnurl-pay">LNURL-Pay</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.build_unsigned_lnurl_pay_package">API docs</a>
</h2>

LNURL payments have their own pair of methods, because completing them includes the LNURL exchange with the recipient's service. Prepare with {{#name prepare_lnurl_pay}} as in [LNURL-Pay](lnurl_pay.md), then run the same loop with {{#name build_unsigned_lnurl_pay_package}} and {{#name publish_signed_lnurl_pay_package}}. The result carries the LNURL response, including any success action:

{{#tabs client_signing:client-signing-lnurl-pay}}

## Failures and retries

- Publishing the same signed package twice returns the same result, so it is safe to retry after a lost response or a network error.
- If publishing fails because the wallet's funds moved or fees changed since the package was built, prepare again and restart the loop with a fresh package.
- Never reuse a signature for a changed payment. Any change to the amount, fee or destination needs a new package, reviewed and signed by the user.

## Remote signers

The signature does not have to come from a device holding the mnemonic. Any {{#name ExternalSparkSigner}} implementation can sign the package, including one backed by a remote signing service. For example, with a Turnkey signer ({{#name create_turnkey_signer}}), a policy can require the end user to approve the transfer signing activity (`SPARK_PREPARE_TRANSFER`) while allowing the server to run the rest of the signing (`SPARK_SIGN_FROST`). The payment then still cannot be sent without the user, and no key material leaves the enclave. See [Using an External Signer](external_signer.md) for the signer interfaces.

## Limitations

- Payments with a conversion step (see [Converting tokens](token_conversion.md)) are not supported.
- USDC/USDT cross-chain sends are not supported.
