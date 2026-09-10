# Signing and verifying messages

Through signing and verifying messages we can provide proof that a digital signature was created by a private key.

## Signing a message

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.sign_message

By signing a message using the SDK we can provide a digital signature. Anyone with the `message`, `pubkey` and `signature` can verify the signature was created by the private key of this pubkey.

Messages starting with `breez-lnurl:` are refused: that namespace is reserved for requests to the Lightning address server.

```kotlin
val message = "<message to sign>"
// Set to true to get a compact signature rather than a DER
val compact = true
try {
    val signMessageRequest = SignMessageRequest(message, compact)
    val signMessageResponse = sdk.signMessage(signMessageRequest)

    val signature = signMessageResponse?.signature
    val pubkey = signMessageResponse?.pubkey

    // Log.v("Breez", "Pubkey: ${pubkey}")
    // Log.v("Breez", "Signature: ${signature}")
} catch (e: Exception) {
    // handle error
}
```



## Verifying a message

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.check_message

You can prove control of a private key by verifying a `message` with it's `signature` and `pubkey`.

```kotlin
val message = "<message>"
val pubkey = "<pubkey of signer>"
val signature = "<message signature>"
try {
    val checkMessageRequest = CheckMessageRequest(message, pubkey, signature)
    val checkMessageResponse = sdk.checkMessage(checkMessageRequest)

    val isValid = checkMessageResponse?.isValid

    // Log.v("Breez", "Signature valid: ${isValid}")
} catch (e: Exception) {
    // handle error
}
```
