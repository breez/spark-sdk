# Signing and verifying messages

Through signing and verifying messages we can provide proof that a digital signature was created by a private key.

## Signing a message

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.sign_message

By signing a message using the SDK we can provide a digital signature. Anyone with the `message`, `pubkey` and `signature` can verify the signature was created by the private key of this pubkey.

Messages starting with `breez-lnurl:` are refused: that namespace is reserved for requests to the Lightning address server.

```swift
// Set to true to get a compact signature rather than a DER
let compact = true

let signMessageRequest = SignMessageRequest(
    message: "<message to sign>",
    compact: compact
)
let signMessageResponse = try await sdk
    .signMessage(request: signMessageRequest)

let signature = signMessageResponse.signature
let pubkey = signMessageResponse.pubkey

print("Pubkey: {}", pubkey);
print("Signature: {}", signature);
```



## Verifying a message

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.check_message

You can prove control of a private key by verifying a `message` with it's `signature` and `pubkey`.

```swift
let checkMessageRequest = CheckMessageRequest(
    message: "<message>",
    pubkey: "<pubkey of signer>",
    signature: "<message signature>"
)
let checkMessageResponse = try await sdk
    .checkMessage(request: checkMessageRequest)

let isValid = checkMessageResponse.isValid

print("Signature valid: {}", isValid);
```
