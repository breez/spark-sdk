# Signing and verifying messages

Through signing and verifying messages we can provide proof that a digital signature was created by a private key.

<h2 id="signing-a-message">
    <a class="header" href="#signing-a-message">Signing a message</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.sign_message">API docs</a>
</h2>

By signing a message using the SDK we can provide a digital signature. Anyone with the `message`, `pubkey` and `signature` can verify the signature was created by the private key of this pubkey.

{{#tabs messages:sign-message}}

<h2 id="verifying-a-message">
    <a class="header" href="#verifying-a-message">Verifying a message (no wallet needed) <span class="badge badge-new">New</span></a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/fn.verify_message.html">API docs</a>
</h2>

You can verify a `message` with its `signature` and `pubkey` without needing a wallet connection. This is a pure cryptographic operation available as a static method on the `Breez` namespace or as a free function.

{{#tabs messages:verify-message}}

<h2 id="verifying-a-message-legacy">
    <a class="header" href="#verifying-a-message-legacy">Verifying a message (legacy) <span class="badge badge-deprecated">Deprecated</span></a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.check_message">API docs</a>
</h2>

> **Deprecated:** Use {{#name verify_message}} above instead. The {{#name check_message}} method requires a wallet connection but performs the same pure cryptographic operation.

{{#tabs messages:check-message}}
