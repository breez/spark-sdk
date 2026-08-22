# Signing and verifying messages

Through signing and verifying messages we can provide proof that a digital signature was created by a private key.

## Signing a message

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.sign_message

By signing a message using the SDK we can provide a digital signature. Anyone with the `message`, `pubkey` and `signature` can verify the signature was created by the private key of this pubkey.

Messages starting with `breez-lnurl:` are refused: that namespace is reserved for requests to the Lightning address server.

```rust
let message = "<message to sign>".to_string();
// Set to true to get a compact signature rather than a DER
let compact = true;

let sign_message_request = SignMessageRequest { message, compact };
let sign_message_response = sdk.sign_message(sign_message_request).await?;

let signature = sign_message_response.signature;
let pubkey = sign_message_response.pubkey;

info!("Pubkey: {}", pubkey);
info!("Signature: {}", signature);
```



## Verifying a message

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.check_message

You can prove control of a private key by verifying a `message` with it's `signature` and `pubkey`.

```rust
let check_message_request = CheckMessageRequest {
    message: "<message>".to_string(),
    pubkey: "<pubkey of signer>".to_string(),
    signature: "<message signature>".to_string(),
};
let check_message_response = sdk.check_message(check_message_request).await?;

let is_valid = check_message_response.is_valid;

info!("Signature valid: {}", is_valid);
```
