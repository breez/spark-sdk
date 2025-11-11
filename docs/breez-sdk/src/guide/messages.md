# Signing and verifying messages

Through signing and verifying messages we can provide proof that a digital signature was created by a private key.

<h2 id="signing-a-message">
    <a class="header" href="#signing-a-message">Signing a message</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.sign_message">API docs</a>
</h2>

By signing a message using the SDK we can provide a digital signature. Anyone with the `message`, `pubkey` and `signature` can verify the signature was created by the private key of this pubkey.

{{#tabs messages:sign-message}}

<h2 id="verifying-a-message">
    <a class="header" href="#verifying-a-message">Verifying a message</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.check_message">API docs</a>
</h2>

You can prove control of a private key by verifying a `message` with it's `signature` and `pubkey`.

{{#tabs messages:check-message}}
