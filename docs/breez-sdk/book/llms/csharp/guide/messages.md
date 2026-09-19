# Signing and verifying messages

Through signing and verifying messages we can provide proof that a digital signature was created by a private key.

## Signing a message

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.sign_message

By signing a message using the SDK we can provide a digital signature. Anyone with the `message`, `pubkey` and `signature` can verify the signature was created by the private key of this pubkey.

Messages starting with `breez-lnurl:` are refused: that namespace is reserved for requests to the Lightning address server.

```csharp
var message = "<message to sign>";
// Set to true to get a compact signature rather than a DER
var compact = true;
var signMessageRequest = new SignMessageRequest(
    message: message,
    compact: compact
);
var signMessageResponse = await sdk.SignMessage(request: signMessageRequest);

var signature = signMessageResponse.signature;
var pubkey = signMessageResponse.pubkey;

Console.WriteLine($"Pubkey: {pubkey}");
Console.WriteLine($"Signature: {signature}");
```



## Verifying a message

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.check_message

You can prove control of a private key by verifying a `message` with it's `signature` and `pubkey`.

```csharp
var message = "<message>";
var pubkey = "<pubkey of signer>";
var signature = "<message signature>";
var checkMessageRequest = new CheckMessageRequest(
    message: message,
    pubkey: pubkey,
    signature: signature
);
var checkMessageResponse = await sdk.CheckMessage(request: checkMessageRequest);

var isValid = checkMessageResponse.isValid;

Console.WriteLine($"Signature valid: {isValid}");
```
